"use client";

import { FolderOpen, Play, Square } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { RestartEngineButton } from "@/components/engine/restart-engine-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import {
  type ManagedEngineConfig,
  applySuggestions,
  isTransient,
  pickFile,
  pickFolder,
} from "@/lib/tauri/engine-process";
import { toAppError } from "@/lib/tauri/errors";
import { useEngineProcessStore } from "@/stores/engine-process-store";

const DEFAULT_CONFIG: ManagedEngineConfig = {
  enabled: false,
  mainPy: "",
  python: "",
  baseDirectory: null,
  port: 8188,
  autoStart: false,
  autoRecover: false,
};

/**
 * ComfyUI の起動管理(docs/15 §9)。有効にすると、この PC の ComfyUI をアプリから起動・停止・再起動できる。
 * 設定の保存は、フォームの「保存」とは別に、このセクションの「保存」で行う。
 */
export function EngineProcessSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const view = useEngineProcessStore((s) => s.view);
  const state = useEngineProcessStore((s) => s.state);
  const logs = useEngineProcessStore((s) => s.logs);
  const busy = useEngineProcessStore((s) => s.busy);
  const loadedSuggestions = useEngineProcessStore((s) => s.suggestions);

  // 編集中の値(未編集なら保存済みの設定をそのまま使う)
  const [edits, setEdits] = useState<ManagedEngineConfig | null>(null);
  // ユーザーが触れた項目(候補の自動入力で上書きしない。docs/32 Q-182)
  const [touched, setTouched] = useState<
    ReadonlySet<keyof ManagedEngineConfig>
  >(new Set());
  const [fieldError, setFieldError] = useState<{
    field?: string;
    message: string;
  } | null>(null);

  useEffect(() => {
    // 設定は即時に表示し、候補は遅れて届いたら反映する(docs/32)
    void useEngineProcessStore.getState().load();
    void useEngineProcessStore.getState().loadSuggestions();
    void useEngineProcessStore.getState().loadLogs();
  }, []);

  const suggestions = loadedSuggestions ?? {};
  const searching = loadedSuggestions === null;
  const saved = view?.config;
  // 保存済みの設定の空欄を候補で補ったもの(「未保存」の比較の基準。保存はユーザーの操作で行う)
  const effective = saved ? applySuggestions(saved, suggestions) : undefined;
  const draft: ManagedEngineConfig = edits
    ? {
        ...edits,
        ...(effective && !touched.has("mainPy")
          ? { mainPy: effective.mainPy }
          : {}),
        ...(effective && !touched.has("python")
          ? { python: effective.python }
          : {}),
        ...(effective && !touched.has("baseDirectory")
          ? { baseDirectory: effective.baseDirectory }
          : {}),
      }
    : (effective ?? DEFAULT_CONFIG);

  const dirty =
    effective !== undefined &&
    JSON.stringify(effective) !== JSON.stringify(draft);
  const hasSuggestions = Boolean(
    suggestions.mainPy || suggestions.python || suggestions.baseDirectory,
  );
  const status = state?.status ?? "stopped";
  const transient = isTransient(status);
  const canOperate = saved?.enabled === true && !dirty && !busy && !transient;

  function set<K extends keyof ManagedEngineConfig>(
    key: K,
    value: ManagedEngineConfig[K],
  ) {
    setFieldError(null);
    setTouched((t) => new Set(t).add(key));
    setEdits({ ...draft, [key]: value });
  }

  async function pick(key: "mainPy" | "python" | "baseDirectory") {
    try {
      const p = key === "baseDirectory" ? await pickFolder() : await pickFile();
      if (p) set(key, p);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  function fillSuggestions() {
    setFieldError(null);
    setTouched((t) =>
      new Set(t).add("mainPy").add("python").add("baseDirectory"),
    );
    setEdits({
      ...draft,
      mainPy: draft.mainPy || suggestions.mainPy || "",
      python: draft.python || suggestions.python || "",
      baseDirectory: draft.baseDirectory || suggestions.baseDirectory || null,
    });
  }

  async function save() {
    try {
      await useEngineProcessStore.getState().save({
        ...draft,
        baseDirectory: draft.baseDirectory?.trim() ? draft.baseDirectory : null,
      });
      setFieldError(null);
      setEdits(null);
      setTouched(new Set());
      toast.success(t("engine.proc.saved"));
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "VALIDATION") {
        setFieldError({ field: err.details?.field, message: err.message });
      } else {
        toast.error(errorMessage(err));
      }
    }
  }

  async function operate(op: "start" | "stop") {
    try {
      await useEngineProcessStore.getState()[op]();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  function pathField(
    key: "mainPy" | "python" | "baseDirectory",
    label: MessageKey,
    hint?: MessageKey,
  ) {
    const value = draft[key] ?? "";
    return (
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`proc-${key}`}>{t(label)}</Label>
        <div className="flex gap-2">
          <Input
            id={`proc-${key}`}
            value={value}
            aria-invalid={fieldError?.field === key}
            onChange={(e) => set(key, e.target.value as never)}
          />
          <Button
            type="button"
            variant="outline"
            onClick={() => void pick(key)}
          >
            <FolderOpen className="size-3.5" />
            {t("engine.proc.choose")}
          </Button>
        </div>
        {hint && <p className="text-xs text-muted-foreground">{t(hint)}</p>}
        {fieldError?.field === key && (
          <p role="alert" className="text-sm text-destructive">
            {fieldError.message}
          </p>
        )}
      </div>
    );
  }

  return (
    <section
      className="flex flex-col gap-3"
      aria-labelledby="engine-proc-title"
    >
      <h2 id="engine-proc-title" className="text-lg font-medium">
        {t("engine.proc.title")}
      </h2>
      <p className="text-xs text-muted-foreground">{t("engine.proc.desc")}</p>

      <div className="flex items-center gap-3">
        <Switch
          id="proc-enabled"
          checked={draft.enabled}
          onCheckedChange={(v) => set("enabled", v)}
          disabled={!view}
        />
        <Label htmlFor="proc-enabled">{t("engine.proc.enable")}</Label>
      </div>

      {draft.enabled && (
        <div className="flex flex-col gap-3 rounded-lg border p-3">
          {searching && (
            <p className="text-xs text-muted-foreground">
              {t("engine.proc.searching")}
            </p>
          )}
          {hasSuggestions && (
            <div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={fillSuggestions}
              >
                {t("engine.proc.fill")}
              </Button>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("engine.proc.fillHint")}
              </p>
            </div>
          )}
          {pathField("mainPy", "engine.proc.mainPy")}
          {pathField("python", "engine.proc.python", "engine.proc.pythonHint")}
          {pathField(
            "baseDirectory",
            "engine.proc.baseDir",
            "engine.proc.baseDirHint",
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="proc-port">{t("engine.proc.port")}</Label>
            <Input
              id="proc-port"
              type="number"
              className="w-32"
              min={1024}
              max={65535}
              value={draft.port}
              aria-invalid={fieldError?.field === "port"}
              onChange={(e) => set("port", Number(e.target.value))}
            />
            {fieldError?.field === "port" && (
              <p role="alert" className="text-sm text-destructive">
                {fieldError.message}
              </p>
            )}
          </div>
          <div className="flex items-center gap-3">
            <Switch
              id="proc-auto"
              checked={draft.autoStart}
              onCheckedChange={(v) => set("autoStart", v)}
            />
            <Label htmlFor="proc-auto">{t("engine.proc.autoStart")}</Label>
          </div>
          <div className="flex flex-col gap-1">
            <div className="flex items-center gap-3">
              <Switch
                id="proc-recover"
                checked={draft.autoRecover}
                onCheckedChange={(v) => set("autoRecover", v)}
              />
              <Label htmlFor="proc-recover">
                {t("engine.proc.autoRecover")}
              </Label>
            </div>
            <p className="pl-12 text-xs text-muted-foreground">
              {t("engine.proc.autoRecoverHint")}
            </p>
          </div>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          disabled={!dirty || busy}
          onClick={() => void save()}
        >
          {t("engine.proc.save")}
        </Button>
        {dirty && (
          <span className="text-xs text-muted-foreground">
            {t("engine.proc.unsaved")}
          </span>
        )}
      </div>

      {saved?.enabled ? (
        <div className="flex flex-col gap-2 rounded-lg border p-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">
              {t("engine.proc.state")}
            </span>
            <Badge
              variant={status === "crashed" ? "destructive" : "outline"}
              role="status"
            >
              {t(`engine.proc.status.${status}` as MessageKey)}
            </Badge>
            <div className="ml-auto flex gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={!canOperate || status === "running"}
                onClick={() => void operate("start")}
              >
                <Play className="size-3.5" />
                {t("engine.proc.start")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={
                  !canOperate || (status !== "running" && status !== "crashed")
                }
                onClick={() => void operate("stop")}
              >
                <Square className="size-3.5" />
                {t("engine.proc.stop")}
              </Button>
              {canOperate && <RestartEngineButton />}
            </div>
          </div>
          {state?.message && status === "crashed" && (
            <p role="alert" className="text-xs text-destructive">
              {state.message}
            </p>
          )}
          <details
            className="text-xs"
            onToggle={() => void useEngineProcessStore.getState().loadLogs()}
          >
            <summary className="cursor-pointer text-muted-foreground">
              {t("engine.proc.logs")}
            </summary>
            <pre className="mt-2 max-h-64 overflow-auto rounded bg-muted p-2 font-mono text-[11px] leading-snug whitespace-pre-wrap">
              {logs.length > 0 ? logs.join("\n") : t("engine.proc.noLogs")}
            </pre>
            {logs.length > 0 && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-1"
                onClick={() =>
                  void navigator.clipboard.writeText(logs.join("\n"))
                }
              >
                {t("engine.proc.copyLogs")}
              </Button>
            )}
          </details>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          {t("engine.proc.externalNote")}
        </p>
      )}
    </section>
  );
}
