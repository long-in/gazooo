"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
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
import { Progress } from "@/components/ui/progress";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { canRelease, canStart, formatMemory, installPercent, isInstalling } from "@/lib/llm/logic";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { assistantLocalCheck, assistantUpdate, type AssistantStatus, type LocalCheck } from "@/lib/tauri/assistant";
import type { SettingsScope } from "@/lib/tauri/settings";
import { toAppError } from "@/lib/tauri/errors";
import { IDLE_MINUTES_CHOICES, type LlmState } from "@/lib/tauri/llm";
import { useLlmStore } from "@/stores/llm-store";

const STATE_LABEL: Record<LlmState, MessageKey> = {
  not_installed: "llm.state.not_installed",
  stopped: "llm.state.stopped",
  starting: "llm.state.starting",
  ready: "llm.state.ready",
  stopping: "llm.state.stopping",
  error: "llm.state.error",
};

const STEP_LABEL: Record<string, MessageKey> = {
  runtime: "llm.install.step.runtime",
  extract: "llm.install.step.extract",
  model: "llm.install.step.model",
};

const idleLabel = (m: number): MessageKey => (m === 0 ? "llm.idle.never" : `llm.idle.${m}`) as MessageKey;

/**
 * アプリ管理のローカル LLM(docs/30 §7)。導入・起動・メモリ解放の操作と、自動で解放するまでの時間。
 * 通信は、利用者が同意して導入を始めたときのダウンロードだけ(導入後の相談は外部に何も送らない)。
 */
export function LocalLlmSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const llm = useLlmStore((s) => s.status);
  const busy = useLlmStore((s) => s.busy);
  const logs = useLlmStore((s) => s.logs);
  const [consentOpen, setConsentOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [showLogs, setShowLogs] = useState(false);

  useEffect(() => {
    void useLlmStore.getState().load();
  }, []);

  // ログは開いている間だけ、状態が変わるたびに取り直す
  const stateKey = llm?.state;
  useEffect(() => {
    if (showLogs) void useLlmStore.getState().loadLogs();
  }, [showLogs, stateKey]);

  if (!llm) return null;

  const st = () => useLlmStore.getState();
  const run = async (fn: () => Promise<void>) => {
    try {
      await fn();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  };

  const installing = isInstalling(llm.install);
  const failed = llm.install.status === "failed" ? llm.install.error : null;
  const sizeLabel = formatMemory(llm.downloadMb);

  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="font-medium">{t("llm.title")}</h3>
        <span className="rounded-full bg-muted px-2 py-0.5 text-xs" data-state={llm.state} aria-label={t("llm.status")}>
          {t(STATE_LABEL[llm.state])}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{t("llm.desc")}</p>

      {!llm.supported ? (
        <p className="text-sm text-muted-foreground">{t("llm.unsupported")}</p>
      ) : !llm.installed ? (
        <div className="flex flex-col gap-2">
          {installing ? (
            <InstallProgress />
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <Button onClick={() => setConsentOpen(true)} disabled={busy}>
                  {t("llm.install.button").replace("{size}", sizeLabel)}
                </Button>
                {llm.install.status === "cancelled" && (
                  <span className="text-sm text-muted-foreground">{t("llm.install.cancelledHint")}</span>
                )}
              </div>
              {failed && (
                <p role="alert" className="text-sm text-destructive">
                  {errorMessage({ code: failed.code, message: failed.message })}
                </p>
              )}
            </>
          )}
        </div>
      ) : (
        <>
          <p className="text-sm">
            {t("llm.installed")
              .replace("{model}", llm.model ?? "")
              .replace("{runtime}", llm.runtime ?? "")}
            {llm.updateAvailable && <span className="ml-2 text-primary">{t("llm.updateAvailable")}</span>}
          </p>
          {installing && <InstallProgress />}
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" disabled={busy || !canStart(llm)} onClick={() => void run(() => st().start())}>
              {t("llm.start")}
            </Button>
            <Button
              variant="outline"
              disabled={busy || !canRelease(llm.state)}
              onClick={() => void run(() => st().stop())}
            >
              {t("llm.release")}
            </Button>
            {llm.memoryHintMb !== null && (
              <span className="text-xs text-muted-foreground">
                {t("llm.memoryHint").replace("{size}", formatMemory(llm.memoryHintMb))}
              </span>
            )}
          </div>
          {llm.state === "error" && llm.message && (
            <p role="alert" className="text-sm text-destructive">
              {llm.message}
            </p>
          )}

          <div className="flex flex-col gap-2">
            <Label>{t("llm.idle.label")}</Label>
            <Select
              value={String(llm.idleMinutes)}
              items={IDLE_MINUTES_CHOICES.map((m) => ({ value: String(m), label: t(idleLabel(m)) }))}
              onValueChange={(v) => v && void run(() => st().updateSettings({ idleMinutes: Number(v) }))}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {IDLE_MINUTES_CHOICES.map((m) => (
                  <SelectItem key={m} value={String(m)}>
                    {t(idleLabel(m))}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">{t("llm.idle.hint")}</p>
          </div>

          <div className="flex items-start gap-3">
            <Switch
              id="llm-release-before-gen"
              checked={llm.releaseBeforeGeneration}
              onCheckedChange={(v) => void run(() => st().updateSettings({ releaseBeforeGeneration: v }))}
            />
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="llm-release-before-gen">{t("llm.releaseBeforeGeneration")}</Label>
              <p className="text-xs text-muted-foreground">{t("llm.releaseBeforeGeneration.hint")}</p>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button variant="ghost" size="sm" onClick={() => setShowLogs((v) => !v)}>
              {showLogs ? t("llm.logs.hide") : t("llm.logs.show")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || installing || llm.state === "starting" || llm.state === "ready"}
              onClick={() => setRemoveOpen(true)}
            >
              {t("llm.remove")}
            </Button>
            {llm.updateAvailable && (
              <Button
                variant="outline"
                size="sm"
                disabled={busy || installing || llm.state === "starting" || llm.state === "ready"}
                onClick={() => setConsentOpen(true)}
              >
                {t("llm.update")}
              </Button>
            )}
          </div>
          {showLogs && (
            <pre className="max-h-48 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap">
              {logs.length === 0 ? t("llm.logs.empty") : logs.join("\n")}
            </pre>
          )}
        </>
      )}

      {consentOpen && (
        <ConsentDialog
          onOpenChange={setConsentOpen}
          onStart={async () => {
            await st().install();
            setConsentOpen(false);
          }}
        />
      )}
      <ConfirmDialog
        open={removeOpen}
        title={t("llm.remove.title")}
        description={t("llm.remove.desc")}
        confirmLabel={t("llm.remove.confirm")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setRemoveOpen}
        onConfirm={() => {
          setRemoveOpen(false);
          void run(() => st().remove());
        }}
      />
    </div>
  );
}

function InstallProgress() {
  const t = useT();
  const llm = useLlmStore((s) => s.status);
  if (!llm) return null;
  const pct = installPercent(llm.install);
  const step = llm.install.stepId ? STEP_LABEL[llm.install.stepId] : undefined;
  return (
    <div className="flex flex-col gap-1">
      <Progress value={pct ?? 0}>
        <span className="text-sm">
          {step ? t(step) : t("llm.install.preparing")}
          {pct !== null ? `  ${pct}%` : ""}
        </span>
      </Progress>
      <div>
        <Button variant="outline" size="sm" onClick={() => void useLlmStore.getState().cancelInstall()}>
          {t("llm.install.cancel")}
        </Button>
      </div>
    </div>
  );
}

/** 導入前に、取得元・サイズ・ライセンスを示して同意を得る(docs/30 §7)。 */
function ConsentDialog({
  onOpenChange,
  onStart,
}: {
  onOpenChange: (open: boolean) => void;
  onStart: () => Promise<void>;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const llm = useLlmStore((s) => s.status);
  const [accepted, setAccepted] = useState(false);
  const [starting, setStarting] = useState(false);
  if (!llm) return null;

  async function start() {
    setStarting(true);
    try {
      await onStart();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setStarting(false);
    }
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("llm.consent.title")}</DialogTitle>
          <DialogDescription>{t("llm.consent.body").replace("{size}", formatMemory(llm.downloadMb))}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2 text-sm">
          <p className="font-medium">{t("llm.consent.hosts")}</p>
          <ul className="list-disc pl-5">
            <li>github.com(llama.cpp)</li>
            <li>huggingface.co(Qwen3.5-9B)</li>
          </ul>
          <p className="text-xs text-muted-foreground">{t("llm.consent.hostsNote")}</p>
          <p className="font-medium">{t("llm.consent.licenses")}</p>
          <ul className="list-disc pl-5">
            <li>llama.cpp — {llm.runtimeLicense}</li>
            <li>Qwen3.5-9B — {llm.modelLicense}</li>
          </ul>
          <p className="text-xs text-muted-foreground">{t("llm.consent.vram")}</p>
          <label className="flex items-center gap-2">
            <Switch checked={accepted} onCheckedChange={setAccepted} />
            {t("llm.consent.accept")}
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={() => void start()} disabled={!accepted || starting}>
            {t("llm.consent.start")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * 外部サーバ(Ollama・llama-server・LM Studio 等。docs/11 §13)の接続先。ループバックのみ。
 * アプリは起動・メモリ管理をしない。
 */
export function ExternalServerSection({
  status,
  scope,
  onSaved,
}: {
  status: AssistantStatus;
  scope: SettingsScope;
  onSaved: () => Promise<void>;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [url, setUrl] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);
  const [check, setCheck] = useState<LocalCheck | null>(null);
  const [busy, setBusy] = useState(false);

  const urlValue = url ?? status.localBaseUrl;
  const modelValue = model ?? status.localModel;
  const dirty = urlValue !== status.localBaseUrl || modelValue !== status.localModel;

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      await assistantUpdate({ localBaseUrl: urlValue, localModel: modelValue, scope });
      await onSaved();
      setUrl(null);
      setModel(null);
      setCheck(null);
      toast.success(t("llm.external.saved"));
    });

  // 確認は、入力中の値(未保存でも)で行う
  const runCheck = () =>
    run(async () => {
      setCheck(await assistantLocalCheck(urlValue, modelValue));
    });

  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <h3 className="font-medium">{t("llm.external.title")}</h3>
      <p className="text-xs text-muted-foreground">{t("llm.external.desc")}</p>
      <Label htmlFor="llm-external-url">{t("llm.external.url")}</Label>
      <Input
        id="llm-external-url"
        value={urlValue}
        spellCheck={false}
        onChange={(e) => {
          setUrl(e.target.value);
          setCheck(null);
        }}
      />
      <Label htmlFor="llm-external-model">{t("llm.external.model")}</Label>
      <Input
        id="llm-external-model"
        value={modelValue}
        spellCheck={false}
        onChange={(e) => {
          setModel(e.target.value);
          setCheck(null);
        }}
      />
      <p className="text-xs text-muted-foreground">{t("llm.external.hint")}</p>
      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={save} disabled={busy || !dirty}>
          {t("llm.external.save")}
        </Button>
        <Button variant="outline" onClick={runCheck} disabled={busy}>
          {t("llm.external.check")}
        </Button>
      </div>
      {check && (
        <p role="status" className={check.reachable && check.modelFound ? "text-sm" : "text-sm text-destructive"}>
          {!check.reachable
            ? t("llm.external.unreachable")
            : check.modelFound
              ? t("llm.external.ok")
              : t("llm.external.modelMissing").replace("{model}", modelValue)}
        </p>
      )}
      {check?.reachable && !check.modelFound && check.models.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {t("llm.external.available").replace("{models}", check.models.slice(0, 8).join(", "))}
        </p>
      )}
      <p className="text-xs text-muted-foreground">{t("llm.external.memoryNote")}</p>
    </div>
  );
}
