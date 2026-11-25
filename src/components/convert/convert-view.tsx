"use client";

import { FileImage, FolderOpen, FolderSearch, Loader2, Play, Square, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import {
  backgroundApplies,
  dimensionsLabel,
  formatLabel,
  mergeEntries,
  qualityApplies,
  runBlockReason,
  runnableEntries,
} from "@/lib/convert/options";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { formatBytes } from "@/lib/models/catalog";
import {
  type ConvertEntry,
  type ConvertFinished,
  type ConvertItemResult,
  type ConvertProgress,
  type ConvertSettings,
  type ConvertSettingsPatch,
  type OutFormat,
  type PngCompression,
  convertCancel,
  convertClear,
  convertList,
  convertOpenOutputDir,
  convertPickFiles,
  convertPickFolder,
  convertPickOutputDir,
  convertRemove,
  convertRun,
  convertSetAccepting,
  convertSettingsGet,
  convertSettingsUpdate,
} from "@/lib/tauri/convert";
import { toAppError } from "@/lib/tauri/errors";
import {
  onConvertAdded,
  onConvertDrag,
  onConvertFinished,
  onConvertItem,
  onConvertProgress,
} from "@/lib/tauri/events";
import { cn } from "@/lib/utils";

const FORMATS: { value: OutFormat; label: string }[] = [
  { value: "png", label: "PNG" },
  { value: "jpeg", label: "JPEG" },
  { value: "webp", label: "WebP" },
];

const PNG_LEVELS: { value: PngCompression; key: MessageKey }[] = [
  { value: "fast", key: "convert.png.fast" },
  { value: "default", key: "convert.png.default" },
  { value: "best", key: "convert.png.best" },
];

/** 画像の形式変換ページ(docs/43)。 */
export function ConvertView() {
  const t = useT();
  const errorMessage = useErrorMessage();

  const [entries, setEntries] = useState<ConvertEntry[]>([]);
  const [settings, setSettings] = useState<ConvertSettings | null>(null);
  const [quality, setQuality] = useState(85);
  const [dragOver, setDragOver] = useState(false);
  const [runId, setRunId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [progress, setProgress] = useState<ConvertProgress | null>(null);
  const [results, setResults] = useState<Record<string, ConvertItemResult>>({});
  const [finished, setFinished] = useState<ConvertFinished | null>(null);
  const runIdRef = useRef<string | null>(null);
  // `convertRun` の応答(runId の確定)より先にイベントが届くことがあるため、開始中は受け入れる
  const startingRef = useRef(false);
  const finishedEarlyRef = useRef(false);
  const isCurrentRun = (id: string) =>
    runIdRef.current === null ? startingRef.current : id === runIdRef.current;

  const running = runId !== null || starting;

  const fail = useCallback((e: unknown) => toast.error(errorMessage(toAppError(e))), [errorMessage]);

  // 初期化: 設定と、すでにある一覧を読む。ページが開いている間だけドロップを受け付ける
  useEffect(() => {
    let alive = true;
    const unlisten: (() => void)[] = [];
    void (async () => {
      try {
        const [s, list] = await Promise.all([convertSettingsGet(), convertList()]);
        if (!alive) return;
        setSettings(s);
        setQuality(s.quality);
        setEntries(list);
      } catch (e) {
        fail(e);
      }
      try {
        await convertSetAccepting(true);
      } catch {
        // Tauri の外(開発中のブラウザ)では何もしない
      }
      const subs = await Promise.all([
        onConvertAdded((added) => setEntries((cur) => mergeEntries(cur, added))),
        onConvertDrag(setDragOver),
        onConvertProgress((p) => {
          if (isCurrentRun(p.runId)) setProgress(p);
        }),
        onConvertItem((r) => setResults((cur) => ({ ...cur, [r.id]: r }))),
        onConvertFinished((f) => {
          if (!isCurrentRun(f.runId)) return;
          if (startingRef.current) finishedEarlyRef.current = true;
          runIdRef.current = null;
          setRunId(null);
          setFinished(f);
        }),
      ]);
      if (alive) unlisten.push(...subs);
      else subs.forEach((u) => u());
    })();
    return () => {
      alive = false;
      unlisten.forEach((u) => u());
      void convertSetAccepting(false).catch(() => {});
    };
  }, [fail]);

  async function update(patch: ConvertSettingsPatch) {
    try {
      setSettings(await convertSettingsUpdate(patch));
    } catch (e) {
      fail(e);
    }
  }

  async function pick(kind: "files" | "folder") {
    try {
      const added = await (kind === "files" ? convertPickFiles() : convertPickFolder());
      if (added.length > 0) setEntries((cur) => mergeEntries(cur, added));
    } catch (e) {
      fail(e);
    }
  }

  async function pickOutput() {
    try {
      const s = await convertPickOutputDir();
      if (s) setSettings(s);
    } catch (e) {
      fail(e);
    }
  }

  async function removeOne(id: string) {
    setEntries((cur) => cur.filter((e) => e.id !== id));
    try {
      await convertRemove([id]);
    } catch (e) {
      fail(e);
    }
  }

  async function clearAll() {
    setEntries([]);
    setResults({});
    setFinished(null);
    setProgress(null);
    try {
      await convertClear();
    } catch (e) {
      fail(e);
    }
  }

  async function run() {
    if (!settings) return;
    const ids = runnableEntries(entries).map((e) => e.id);
    startingRef.current = true;
    finishedEarlyRef.current = false;
    setStarting(true);
    setResults({});
    setFinished(null);
    setProgress({ runId: "", done: 0, total: ids.length });
    try {
      const { runId: id } = await convertRun({
        ids,
        format: settings.format,
        quality,
        webpLossless: settings.webpLossless,
        pngCompression: settings.pngCompression,
        background: settings.background,
        overwrite: settings.overwrite,
      });
      // 応答より先に完了通知が届いていた場合は、実行中に戻さない
      if (!finishedEarlyRef.current) {
        runIdRef.current = id;
        setRunId(id);
      }
    } catch (e) {
      setProgress(null);
      fail(e);
    } finally {
      startingRef.current = false;
      setStarting(false);
    }
  }

  async function cancel() {
    if (runId) await convertCancel(runId).catch(fail);
  }

  const block = runBlockReason(entries, settings, running);
  const runnable = runnableEntries(entries).length;
  const showQuality = settings ? qualityApplies(settings.format, settings.webpLossless) : true;
  const done = progress?.done ?? 0;
  const total = progress?.total ?? 0;

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-5 p-6">
      <h1 className="sr-only">{t("convert.title")}</h1>

      {/* ドロップ領域 */}
      <div
        className={cn(
          "flex flex-col items-center gap-3 rounded-xl border-2 border-dashed p-8 text-center transition-colors",
          dragOver ? "border-primary bg-primary/10" : "border-border",
        )}
      >
        <FileImage className="size-8 text-muted-foreground" aria-hidden />
        <p className="text-sm">{t("convert.drop")}</p>
        <p className="text-xs text-muted-foreground">{t("convert.drop.hint")}</p>
        <div className="flex gap-2">
          <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => void pick("files")}>
            <FileImage />
            {t("convert.pickFiles")}
          </Button>
          <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => void pick("folder")}>
            <FolderSearch />
            {t("convert.pickFolder")}
          </Button>
        </div>
      </div>

      {/* 一覧 */}
      {entries.length > 0 && (
        <section className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-medium">
              {t("convert.list").replace("{n}", String(entries.length))}
            </h2>
            <Button type="button" variant="ghost" size="sm" disabled={running} onClick={() => void clearAll()}>
              <Trash2 />
              {t("convert.clear")}
            </Button>
          </div>
          <ul className="flex max-h-72 flex-col overflow-y-auto rounded-xl border text-sm">
            {entries.map((e) => {
              const r = results[e.id];
              return (
                <li key={e.id} className="flex items-center gap-3 border-b px-3 py-1.5 last:border-b-0">
                  <span className="min-w-0 flex-1 truncate" title={e.name}>
                    {e.name}
                  </span>
                  {e.error ? (
                    <span className="shrink-0 text-xs text-destructive">⚠ {e.error}</span>
                  ) : (
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {formatLabel(e.detectedFormat)} {dimensionsLabel(e)} {formatBytes(e.bytes)}
                    </span>
                  )}
                  {r && (
                    <span className={cn("shrink-0 text-xs", r.ok ? "text-primary" : "text-destructive")}>
                      {r.ok
                        ? `✓ ${r.outputName ?? ""} (${formatBytes(r.outBytes ?? 0)})`
                        : `✕ ${r.error ?? ""}`}
                    </span>
                  )}
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    disabled={running}
                    aria-label={t("convert.remove")}
                    onClick={() => void removeOne(e.id)}
                  >
                    <X />
                  </Button>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {/* 出力設定 */}
      {settings && (
        <section className="flex flex-col gap-4 rounded-xl border p-4">
          <div className="flex items-center gap-3">
            <Label className="w-24 shrink-0">{t("convert.format")}</Label>
            <div role="radiogroup" aria-label={t("convert.format")} className="flex gap-1">
              {FORMATS.map((f) => (
                <Button
                  key={f.value}
                  type="button"
                  role="radio"
                  aria-checked={settings.format === f.value}
                  size="sm"
                  variant={settings.format === f.value ? "default" : "outline"}
                  disabled={running}
                  onClick={() => void update({ format: f.value })}
                >
                  {f.label}
                </Button>
              ))}
            </div>
          </div>

          {settings.format === "png" ? (
            <div className="flex items-center gap-3">
              <Label className="w-24 shrink-0">{t("convert.png.level")}</Label>
              <div role="radiogroup" aria-label={t("convert.png.level")} className="flex gap-1">
                {PNG_LEVELS.map((l) => (
                  <Button
                    key={l.value}
                    type="button"
                    role="radio"
                    aria-checked={settings.pngCompression === l.value}
                    size="sm"
                    variant={settings.pngCompression === l.value ? "default" : "outline"}
                    disabled={running}
                    onClick={() => void update({ pngCompression: l.value })}
                  >
                    {t(l.key)}
                  </Button>
                ))}
              </div>
              <span className="text-xs text-muted-foreground">{t("convert.png.lossless")}</span>
            </div>
          ) : (
            <div className="flex items-center gap-3">
              <Label className="w-24 shrink-0">{t("convert.quality")}</Label>
              <Slider
                className="max-w-xs"
                min={1}
                max={100}
                step={1}
                value={[quality]}
                disabled={running || !showQuality}
                onValueChange={(v) => setQuality(Array.isArray(v) ? v[0] : v)}
                onValueCommitted={(v) => void update({ quality: Array.isArray(v) ? v[0] : v })}
              />
              <span className="w-8 text-right text-sm tabular-nums">{showQuality ? quality : "—"}</span>
              {settings.format === "webp" && (
                <div className="flex items-center gap-2">
                  <Switch
                    id="convert-lossless"
                    checked={settings.webpLossless}
                    disabled={running}
                    onCheckedChange={(v) => void update({ webpLossless: v })}
                  />
                  <Label htmlFor="convert-lossless">{t("convert.webp.lossless")}</Label>
                </div>
              )}
            </div>
          )}
          {settings.format !== "png" && (
            <p className="pl-27 text-xs text-muted-foreground">{t("convert.quality.hint")}</p>
          )}

          {backgroundApplies(settings.format) && (
            <div className="flex items-center gap-3">
              <Label htmlFor="convert-bg" className="w-24 shrink-0">
                {t("convert.background")}
              </Label>
              <input
                id="convert-bg"
                type="color"
                className="h-8 w-12 cursor-pointer rounded border bg-transparent"
                value={settings.background}
                disabled={running}
                onChange={(e) => setSettings({ ...settings, background: e.target.value })}
                onBlur={(e) => void update({ background: e.target.value })}
              />
              <span className="text-xs text-muted-foreground">{t("convert.background.hint")}</span>
            </div>
          )}

          <div className="flex items-center gap-3">
            <Label className="w-24 shrink-0">{t("convert.outputDir")}</Label>
            <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 text-xs" title={settings.outputDir}>
              {settings.outputDir}
            </code>
            <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => void pickOutput()}>
              <FolderOpen />
              {t("convert.outputDir.change")}
            </Button>
          </div>

          <div className="flex items-center gap-3">
            <Switch
              id="convert-overwrite"
              checked={settings.overwrite}
              disabled={running}
              onCheckedChange={(v) => void update({ overwrite: v })}
            />
            <Label htmlFor="convert-overwrite">{t("convert.overwrite")}</Label>
          </div>

          <p className="text-xs text-muted-foreground">{t("convert.metadata.note")}</p>
        </section>
      )}

      {/* 実行 */}
      <section className="flex flex-col gap-3">
        <div className="flex items-center gap-3">
          {running ? (
            <Button type="button" variant="outline" onClick={() => void cancel()} disabled={!runId}>
              <Square />
              {t("convert.cancel")}
            </Button>
          ) : (
            <Button type="button" disabled={block !== null} onClick={() => void run()}>
              <Play />
              {t("convert.run").replace("{n}", String(runnable))}
            </Button>
          )}
          {block && block !== "running" && (
            <span className="text-xs text-muted-foreground">{t(`convert.block.${block}` as MessageKey)}</span>
          )}
          {running && <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden />}
        </div>

        {running && total > 0 && (
          <div className="flex flex-col gap-1" aria-live="polite">
            <div className="h-1.5 overflow-hidden rounded-full bg-muted">
              <div className="h-full bg-primary transition-[width]" style={{ width: `${(done / total) * 100}%` }} />
            </div>
            <span className="text-xs text-muted-foreground tabular-nums">
              {done} / {total}
            </span>
          </div>
        )}

        {finished && (
          <div className="flex flex-wrap items-center gap-3 rounded-xl border p-3 text-sm" aria-live="polite">
            <span>
              {t("convert.result")
                .replace("{ok}", String(finished.ok))
                .replace("{failed}", String(finished.failed))}
              {finished.cancelled && ` (${t("convert.cancelled")})`}
            </span>
            {finished.ok > 0 && (
              <span className="text-xs text-muted-foreground">
                {formatBytes(finished.inBytes)} → {formatBytes(finished.outBytes)}
              </span>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void convertOpenOutputDir().catch(fail)}
            >
              <FolderOpen />
              {t("convert.openOutput")}
            </Button>
          </div>
        )}
      </section>
    </div>
  );
}
