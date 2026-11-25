"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { imageSrc } from "@/lib/image-src";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { isActiveJob } from "@/lib/queue";
import {
  DEFAULT_CHARACTER_THRESHOLD,
  DEFAULT_THRESHOLD,
  applyTags,
  clampThreshold,
  pickTaggerModel,
  prefersNaturalLanguage,
} from "@/lib/tagger/logic";
import { toAppError } from "@/lib/tauri/errors";
import { imageList } from "@/lib/tauri/images";
import { inputImagePick } from "@/lib/tauri/inputs";
import type { ImageItem } from "@/lib/tauri/jobs";
import { imageTag } from "@/lib/tauri/tagger";
import type { ApplyMode } from "@/lib/translate/apply";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { useJobStore } from "@/stores/job-store";
import { useTaggerStore } from "@/stores/tagger-store";

/**
 * 「画像からタグを作る」(docs/39 §4.1)。画像を ComfyUI のタガーで解析し、結果のタグ列を
 * 確認・編集したうえで、プロンプト欄へ「置き換え」または「末尾に追加」で入れる。
 * 結果は自動では反映しない(Q-294)。画像は外部へ送られない。
 */
export function ImageTagDialog() {
  const open = useTaggerStore((s) => s.open);
  const image = useTaggerStore((s) => s.image);
  // 開くたび・画像が変わるたびに作り直し、結果や実行中の状態を持ち越さない
  if (!open) return null;
  return <ImageTagDialogBody key={image?.path ?? ""} />;
}

/** ギャラリーの画像を一覧から選ぶ(docs/39 §4.1)。選ぶと対象画像が替わり、ダイアログが作り直される。 */
function GalleryPicker({ onError }: { onError: (message: string) => void }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [items, setItems] = useState<ImageItem[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [picking, setPicking] = useState(false);

  async function load(next?: string) {
    setLoading(true);
    try {
      const page = await imageList({ limit: 48, ...(next ? { cursor: next } : {}) });
      setItems((prev) => (next ? [...prev, ...page.items] : page.items));
      setCursor(page.nextCursor ?? null);
    } catch (e) {
      onError(errorMessage(toAppError(e)));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    // 開いたときに先頭の一覧を読む(外部=バックエンドとの同期)
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function choose(id: string) {
    if (picking) return;
    setPicking(true);
    try {
      await useTaggerStore.getState().openFromGallery(id);
    } catch (e) {
      onError(errorMessage(toAppError(e)));
      setPicking(false);
    }
  }

  if (!loading && items.length === 0) {
    return <p className="text-xs text-muted-foreground">{t("tagger.galleryEmpty")}</p>;
  }
  return (
    <div className="flex flex-col gap-2">
      <div className="grid max-h-56 grid-cols-4 gap-1.5 overflow-y-auto rounded-md border p-1.5 sm:grid-cols-6">
        {items.map((img) => (
          <button
            key={img.id}
            type="button"
            disabled={picking}
            title={`${img.width}×${img.height} · seed ${img.seed}`}
            aria-label={`${img.width}×${img.height} · seed ${img.seed}`}
            onClick={() => void choose(img.id)}
            className="aspect-square overflow-hidden rounded border bg-muted/30 outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <img src={imageSrc(img.thumbPath)} alt="" className="size-full object-cover" loading="lazy" draggable={false} />
          </button>
        ))}
        {loading && <span className="col-span-full text-xs text-muted-foreground">{t("strip.loading")}</span>}
      </div>
      {cursor && !loading && (
        <Button type="button" variant="outline" size="xs" className="self-start" onClick={() => void load(cursor)}>
          {t("tagger.galleryMore")}
        </Button>
      )}
    </div>
  );
}

function ImageTagDialogBody() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const router = useRouter();
  const pathname = usePathname();

  const image = useTaggerStore((s) => s.image);
  const status = useEngineStore((s) => s.status);
  const taggers = useEngineStore((s) => s.models?.taggers ?? []);
  const kind = useFormStore((s) => s.params.model.kind);
  const busyJobs = useJobStore((s) => s.jobs.some(isActiveJob));

  const [model, setModel] = useState<string | null>(null);
  const [threshold, setThreshold] = useState(String(DEFAULT_THRESHOLD));
  const [characterThreshold, setCharacterThreshold] = useState(String(DEFAULT_CHARACTER_THRESHOLD));
  const [excludeTags, setExcludeTags] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showGallery, setShowGallery] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [mode, setMode] = useState<ApplyMode>("append");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 実行ごとの識別子。キャンセル・画像の変更で進める(古い実行の結果は捨てる。Q-295)
  const token = useRef(0);

  const connected = status?.connected === true;
  const nodeMissing = connected && status?.features.imageTag !== true;
  const selectedModel = pickTaggerModel(taggers, model ?? undefined);

  function discardRun() {
    token.current += 1;
    setRunning(false);
  }

  function close() {
    discardRun();
    useTaggerStore.getState().close();
  }

  async function pick() {
    try {
      const picked = await inputImagePick();
      if (picked) useTaggerStore.getState().setImage(picked);
    } catch (e) {
      setError(errorMessage(toAppError(e)));
    }
  }

  async function run() {
    if (!image || running || !selectedModel) return;
    const id = ++token.current;
    setRunning(true);
    setError(null);
    try {
      const r = await imageTag(image.path, {
        model: selectedModel,
        threshold: clampThreshold(Number(threshold), DEFAULT_THRESHOLD),
        characterThreshold: clampThreshold(Number(characterThreshold), DEFAULT_CHARACTER_THRESHOLD),
        excludeTags,
      });
      if (id !== token.current) return; // キャンセル済み
      setResult(r.tags);
      if (r.tags === "") setError(t("tagger.empty"));
    } catch (e) {
      if (id !== token.current) return;
      const err = toAppError(e);
      setError(
        err.code === "ENGINE_BUSY"
          ? t("tagger.busy")
          : err.code === "ENGINE_NODE_MISSING"
            ? t("tagger.nodeMissing")
            : errorMessage(err),
      );
    } finally {
      if (id === token.current) setRunning(false);
    }
  }

  function apply() {
    if (result === null || result.trim() === "") return;
    const previous = useFormStore.getState().params.prompt;
    useFormStore.getState().patch({ prompt: applyTags(previous, result, mode) });
    toast(t("tagger.done"), {
      duration: 8000,
      action: {
        label: t("tagger.undo"),
        onClick: () => useFormStore.getState().patch({ prompt: previous }),
      },
    });
    useTaggerStore.getState().close();
    if (pathname !== "/") router.push("/");
  }

  const blocked = !connected
    ? t("tagger.disconnected")
    : nodeMissing
      ? t("tagger.nodeMissingHint")
      : busyJobs
        ? t("tagger.busy")
        : null;
  const canRun = image !== null && !running && blocked === null && selectedModel !== null;

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (next) return;
        // 解析中の Esc・× は、先に解析のキャンセルだけを行う(docs/39 §6.2)
        if (running) discardRun();
        else close();
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("tagger.title")}</DialogTitle>
          <DialogDescription>{t("tagger.desc")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            {image ? (
              <img
                src={imageSrc(image.filePath)}
                alt=""
                className="size-20 shrink-0 rounded-md border bg-muted/30 object-cover"
              />
            ) : (
              <div className="flex size-20 shrink-0 items-center justify-center rounded-md border border-dashed text-xs text-muted-foreground">
                {t("tagger.noImage")}
              </div>
            )}
            <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => void pick()}>
              {image ? t("tagger.change") : t("tagger.pickFile")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={running}
              aria-expanded={showGallery}
              onClick={() => setShowGallery((v) => !v)}
            >
              {t("tagger.pickGallery")}
            </Button>
          </div>
          {showGallery && !running && <GalleryPicker onError={(m) => setError(m)} />}

          <div>
            <button
              type="button"
              className="text-xs text-muted-foreground underline-offset-2 hover:underline"
              aria-expanded={showAdvanced}
              onClick={() => setShowAdvanced((v) => !v)}
            >
              {showAdvanced ? "▾ " : "▸ "}
              {t("tagger.advanced")}
            </button>
            {showAdvanced && (
              <div className="mt-2 grid grid-cols-2 gap-2">
                <div className="col-span-2 flex flex-col gap-1">
                  <Label htmlFor="tagger-model">{t("tagger.model")}</Label>
                  <select
                    id="tagger-model"
                    className="h-(--ctl-h) rounded-md border border-input bg-input-bg px-2 text-sm"
                    value={selectedModel ?? ""}
                    disabled={running || taggers.length === 0}
                    onChange={(e) => setModel(e.target.value)}
                  >
                    {taggers.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="flex flex-col gap-1">
                  <Label htmlFor="tagger-threshold">{t("tagger.threshold")}</Label>
                  <Input
                    id="tagger-threshold"
                    type="number"
                    min={0.05}
                    max={1}
                    step={0.05}
                    value={threshold}
                    disabled={running}
                    onChange={(e) => setThreshold(e.target.value)}
                  />
                </div>
                <div className="flex flex-col gap-1">
                  <Label htmlFor="tagger-char-threshold">{t("tagger.characterThreshold")}</Label>
                  <Input
                    id="tagger-char-threshold"
                    type="number"
                    min={0.05}
                    max={1}
                    step={0.05}
                    value={characterThreshold}
                    disabled={running}
                    onChange={(e) => setCharacterThreshold(e.target.value)}
                  />
                </div>
                <div className="col-span-2 flex flex-col gap-1">
                  <Label htmlFor="tagger-exclude">{t("tagger.excludeTags")}</Label>
                  <Input
                    id="tagger-exclude"
                    value={excludeTags}
                    maxLength={1000}
                    placeholder="blurry, text"
                    disabled={running}
                    onChange={(e) => setExcludeTags(e.target.value)}
                  />
                </div>
              </div>
            )}
          </div>

          <div className="flex items-center gap-2">
            <Button type="button" size="sm" disabled={!canRun} onClick={() => void run()}>
              {running ? t("tagger.running") : t("tagger.run")}
            </Button>
            {running && (
              <Button type="button" size="sm" variant="outline" onClick={discardRun}>
                {t("tagger.cancel")}
              </Button>
            )}
          </div>
          {running && <p className="text-xs text-muted-foreground">{t("tagger.firstRun")}</p>}
          {blocked && !running && (
            <p role="status" className="text-xs text-destructive">
              {blocked}
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}

          {result !== null && result !== "" && (
            <div className="flex flex-col gap-2">
              <Label htmlFor="tagger-result">{t("tagger.result")}</Label>
              <Textarea
                id="tagger-result"
                rows={5}
                value={result}
                onChange={(e) => setResult(e.target.value)}
                onKeyDown={(e) => {
                  // Enter で確定しない(改行のみ)。反映は明示のボタン操作だけ
                  if (e.key === "Enter") e.stopPropagation();
                }}
              />
              <div className="flex items-center gap-2">
                <div className="flex overflow-hidden rounded-md border text-xs" role="radiogroup">
                  {(["replace", "append"] as const).map((m) => (
                    <button
                      key={m}
                      type="button"
                      role="radio"
                      aria-checked={mode === m}
                      className={`px-2 py-1 ${mode === m ? "bg-primary text-primary-foreground" : "bg-background"}`}
                      onClick={() => setMode(m)}
                    >
                      {t(m === "replace" ? "tagger.mode.replace" : "tagger.mode.append")}
                    </button>
                  ))}
                </div>
                <Button type="button" size="sm" className="ml-auto" onClick={apply}>
                  {t("tagger.apply")}
                </Button>
              </div>
            </div>
          )}

          {prefersNaturalLanguage(kind) && (
            <p className="text-xs text-muted-foreground">{t("tagger.naturalNote")}</p>
          )}
          <p className="text-xs text-muted-foreground">{t("tagger.local")}</p>
        </div>

        <DialogFooter showCloseButton={false}>
          <Button type="button" variant="outline" onClick={close}>
            {t("tagger.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
