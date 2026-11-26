"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { AssistantPromptEditor } from "@/components/settings/assistant-prompt-editor";
import { Overridable } from "@/components/settings/overridable";
import { ExternalServerSection, LocalLlmSection } from "@/components/settings/local-llm-section";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import {
  ASSISTANT_MODELS,
  type AssistantProvider,
  assistantClearKey,
  assistantSetKey,
  assistantStatus,
  assistantUpdate,
  type AssistantPatch,
  type AssistantStatus,
  MAX_CONVERSATIONS,
} from "@/lib/tauri/assistant";
import { toAppError } from "@/lib/tauri/errors";
import type { SettingsScope } from "@/lib/tauri/settings";
import { useAssistantStore } from "@/stores/assistant-store";

export const MODEL_LABEL: Record<(typeof ASSISTANT_MODELS)[number], MessageKey> = {
  "claude-haiku-5-5": "settings.assistant.model.haiku",
  "claude-sonnet-5-5": "settings.assistant.model.sonnet",
  "claude-opus-5-5": "settings.assistant.model.opus",
};

export const PROVIDERS: AssistantProvider[] = ["anthropic", "local", "local_external"];

export const PROVIDER_LABEL: Record<AssistantProvider, MessageKey> = {
  anthropic: "settings.assistant.provider.anthropic",
  local: "settings.assistant.provider.local",
  local_external: "settings.assistant.provider.external",
};

/**
 * そのスコープの AI サポートの状態。全体設定の値は画面内だけで持ち、
 * 相談パネルが使う実際の値(ストア)には混ぜない。
 */
function useScopedStatus(scope: SettingsScope) {
  const errorMessage = useErrorMessage();
  const effective = useAssistantStore((s) => s.status);
  const [global, setGlobal] = useState<AssistantStatus | null>(null);

  useEffect(() => {
    if (scope === "global") void assistantStatus("global").then(setGlobal).catch(() => {});
    else void useAssistantStore.getState().loadStatus();
  }, [scope]);

  const update = async (patch: AssistantPatch) => {
    try {
      const next = await assistantUpdate({ ...patch, scope });
      if (scope === "global") {
        setGlobal(await assistantStatus("global"));
        await useAssistantStore.getState().loadStatus();
      } else {
        useAssistantStore.getState().setStatus(next);
      }
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  };
  /** 外部サーバの保存のあとに、表示中の値を取り直す。 */
  const refresh = async () => {
    if (scope === "global") setGlobal(await assistantStatus("global"));
    await useAssistantStore.getState().loadStatus();
  };

  return { status: scope === "global" ? global : effective, update, refresh };
}

/** 送信先とモデル(上書きできる組)。 */
function ConnectionFields({ scope }: { scope: SettingsScope }) {
  const t = useT();
  const { status, update, refresh } = useScopedStatus(scope);
  if (!status) return null;

  return (
    <>
      <div className="flex flex-col gap-2">
        <Label>{t("settings.assistant.provider")}</Label>
        <Select
          value={status.provider}
          items={PROVIDERS.map((p) => ({
            value: p,
            label: t(PROVIDER_LABEL[p]),
          }))}
          onValueChange={(v) => v && void update({ provider: v as AssistantProvider })}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {PROVIDERS.map((p) => (
              <SelectItem key={p} value={p}>
                {t(PROVIDER_LABEL[p])}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {status.provider !== "anthropic" && (
          <p className="text-xs text-muted-foreground">{t("settings.assistant.provider.localHint")}</p>
        )}
      </div>

      {status.provider === "local_external" && (
        <ExternalServerSection status={status} scope={scope} onSaved={refresh} />
      )}

      {status.provider === "anthropic" && (
        <div className="flex flex-col gap-2">
          <Label>{t("settings.assistant.model")}</Label>
          <Select
            value={status.model}
            items={ASSISTANT_MODELS.map((m) => ({
              value: m,
              label: t(MODEL_LABEL[m]),
            }))}
            onValueChange={(v) => v && void update({ model: v })}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ASSISTANT_MODELS.map((m) => (
                <SelectItem key={m} value={m}>
                  {t(MODEL_LABEL[m])}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
    </>
  );
}

/** API キーと外部送信の同意(本人のものだけ。全体設定にはしない)。 */
function PersonalFields() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const { status, update } = useScopedStatus("user");
  const setStatus = useAssistantStore((s) => s.setStatus);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  if (!status) return null;

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

  const saveKey = () =>
    run(async () => {
      await assistantSetKey(key);
      setKey("");
      setStatus(await assistantStatus());
      toast.success(t("settings.assistant.keySaved"));
    });

  const clearKey = () =>
    run(async () => {
      await assistantClearKey();
      setStatus(await assistantStatus());
      toast.success(t("settings.assistant.keyCleared"));
    });

  return (
    <div className="flex flex-col gap-3 rounded-lg border p-3">
      <Label htmlFor="assistant-key">{t("settings.assistant.key")}</Label>
      <p className="text-sm">
        {status.hasKey ? (
          <>
            {t("settings.assistant.keyRegistered")}{" "}
            <span className="text-muted-foreground">
              {status.verified ? t("settings.assistant.keyVerified") : t("settings.assistant.keyUnverified")}
            </span>
          </>
        ) : (
          <span className="text-muted-foreground">{t("settings.assistant.keyNone")}</span>
        )}
      </p>
      <div className="flex gap-2">
        <Input
          id="assistant-key"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={key}
          placeholder={t("settings.assistant.keyPlaceholder")}
          onChange={(e) => setKey(e.target.value)}
        />
        <Button onClick={saveKey} disabled={busy || key.trim() === ""}>
          {t("settings.assistant.keySave")}
        </Button>
        {status.hasKey && (
          <Button variant="outline" onClick={clearKey} disabled={busy}>
            {t("settings.assistant.keyClear")}
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">{t("settings.assistant.keyWarn")}</p>

      <div className="flex items-start gap-3">
        <Switch id="assistant-consent" checked={status.consent} onCheckedChange={(v) => void update({ consent: v })} />
        <div className="flex flex-col gap-0.5">
          <Label htmlFor="assistant-consent">{t("settings.assistant.consent")}</Label>
          <p className="text-xs text-muted-foreground">{t("settings.assistant.consentHint")}</p>
        </div>
      </div>
    </div>
  );
}

function AutoDeleteFields({ scope }: { scope: SettingsScope }) {
  const t = useT();
  const { status, update } = useScopedStatus(scope);
  const conversations = useAssistantStore((s) => s.conversations);
  const [confirm, setConfirm] = useState(false);
  if (!status) return null;

  const onAutoDelete = (on: boolean) => {
    // 有効に戻すと、上限を超えた会話がすぐ削除される。該当があるときだけ確認する
    if (on && conversations.length > MAX_CONVERSATIONS) setConfirm(true);
    else void update({ autoDelete: on });
  };

  return (
    <>
      <div className="flex items-start gap-3">
        <Switch id="assistant-autodelete" checked={status.autoDelete} onCheckedChange={onAutoDelete} />
        <div className="flex flex-col gap-0.5">
          <Label htmlFor="assistant-autodelete">{t("settings.assistant.autoDelete")}</Label>
          <p className="text-xs text-muted-foreground">{t("settings.assistant.autoDeleteHint")}</p>
        </div>
      </div>
      <ConfirmDialog
        open={confirm}
        title={t("settings.assistant.autoDeleteConfirmTitle")}
        description={t("settings.assistant.autoDeleteConfirmDesc")}
        confirmLabel={t("settings.assistant.autoDeleteConfirm")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setConfirm}
        onConfirm={() => {
          setConfirm(false);
          void update({ autoDelete: true }).then(() => useAssistantStore.getState().loadConversations());
        }}
      />
    </>
  );
}

function Conversations() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const conversations = useAssistantStore((s) => s.conversations);
  const [confirm, setConfirm] = useState(false);

  useEffect(() => {
    void useAssistantStore.getState().loadConversations();
  }, []);

  return (
    <div className="flex items-center gap-3">
      <span className="text-sm text-muted-foreground">
        {t("settings.assistant.conversations")}: {conversations.length}
      </span>
      <Button variant="outline" size="sm" disabled={conversations.length === 0} onClick={() => setConfirm(true)}>
        {t("settings.assistant.deleteAll")}
      </Button>
      <ConfirmDialog
        open={confirm}
        title={t("settings.assistant.deleteAllConfirmTitle")}
        description={t("settings.assistant.deleteAllConfirmDesc")}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setConfirm}
        onConfirm={() => {
          setConfirm(false);
          void useAssistantStore
            .getState()
            .deleteConversations(null)
            .then(() => toast.success(t("settings.assistant.deleteAllDone")))
            .catch((e) => toast.error(errorMessage(toAppError(e))));
        }}
      />
    </div>
  );
}

/**
 * AI サポートの設定(docs/11、docs/52)。操作した時点で即時に保存する。
 * ローカル LLM の導入・管理は PC に 1 つなので全体設定に、キー・同意・会話は個別設定に置く。
 */
export function AssistantSection({ scope }: { scope: SettingsScope }) {
  const t = useT();
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.assistant.title")}</h2>
      <p className="text-sm text-muted-foreground">{t("settings.assistant.desc")}</p>

      <Overridable group="assistantConnection" scope={scope} label={t("settings.assistant.group.connection")}>
        <ConnectionFields scope={scope} />
      </Overridable>

      {scope === "global" && <LocalLlmSection />}
      {scope === "user" && <PersonalFields />}

      <Overridable group="assistantAutoDelete" scope={scope} label={t("settings.assistant.group.autoDelete")}>
        <AutoDeleteFields scope={scope} />
      </Overridable>
      {scope === "user" && <Conversations />}

      <Overridable group="assistantPrompt" scope={scope} label={t("settings.assistant.group.prompt")}>
        <AssistantPromptEditor scope={scope} />
      </Overridable>
    </section>
  );
}
