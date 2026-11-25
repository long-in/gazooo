"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { AssistantPromptEditor } from "@/components/settings/assistant-prompt-editor";
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
  MAX_CONVERSATIONS,
} from "@/lib/tauri/assistant";
import { toAppError } from "@/lib/tauri/errors";
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
 * AI サポートの設定(docs/11-ai-assistant.md)。キー・モデル・同意・自動削除は、
 * 画面下部の「保存」とは別に、操作した時点で即時に保存する。
 */
export function AssistantSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const status = useAssistantStore((s) => s.status);
  const setStatus = useAssistantStore((s) => s.setStatus);
  const conversations = useAssistantStore((s) => s.conversations);

  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<"autoDelete" | "deleteAll" | null>(null);

  useEffect(() => {
    void useAssistantStore.getState().loadStatus();
    void useAssistantStore.getState().loadConversations();
  }, []);

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

  const update = (patch: Parameters<typeof assistantUpdate>[0]) =>
    run(async () => setStatus(await assistantUpdate(patch)));

  const onAutoDelete = (on: boolean) => {
    // 有効に戻すと、上限を超えた会話がすぐ削除される。該当があるときだけ確認する
    if (on && conversations.length > MAX_CONVERSATIONS) setConfirm("autoDelete");
    else void update({ autoDelete: on });
  };

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.assistant.title")}</h2>
      <p className="text-sm text-muted-foreground">{t("settings.assistant.desc")}</p>

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

      {status.provider === "local" && <LocalLlmSection />}
      {status.provider === "local_external" && <ExternalServerSection />}

      {status.provider === "anthropic" && (
        <>
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

          <div className="flex items-start gap-3">
            <Switch
              id="assistant-consent"
              checked={status.consent}
              onCheckedChange={(v) => void update({ consent: v })}
            />
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="assistant-consent">{t("settings.assistant.consent")}</Label>
              <p className="text-xs text-muted-foreground">{t("settings.assistant.consentHint")}</p>
            </div>
          </div>
        </>
      )}

      <div className="flex items-start gap-3">
        <Switch id="assistant-autodelete" checked={status.autoDelete} onCheckedChange={onAutoDelete} />
        <div className="flex flex-col gap-0.5">
          <Label htmlFor="assistant-autodelete">{t("settings.assistant.autoDelete")}</Label>
          <p className="text-xs text-muted-foreground">{t("settings.assistant.autoDeleteHint")}</p>
        </div>
      </div>

      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {t("settings.assistant.conversations")}: {conversations.length}
        </span>
        <Button
          variant="outline"
          size="sm"
          disabled={conversations.length === 0}
          onClick={() => setConfirm("deleteAll")}
        >
          {t("settings.assistant.deleteAll")}
        </Button>
      </div>

      <AssistantPromptEditor />

      <ConfirmDialog
        open={confirm === "autoDelete"}
        title={t("settings.assistant.autoDeleteConfirmTitle")}
        description={t("settings.assistant.autoDeleteConfirmDesc")}
        confirmLabel={t("settings.assistant.autoDeleteConfirm")}
        cancelLabel={t("common.cancel")}
        onOpenChange={(o) => !o && setConfirm(null)}
        onConfirm={() => {
          setConfirm(null);
          void update({ autoDelete: true }).then(() => useAssistantStore.getState().loadConversations());
        }}
      />
      <ConfirmDialog
        open={confirm === "deleteAll"}
        title={t("settings.assistant.deleteAllConfirmTitle")}
        description={t("settings.assistant.deleteAllConfirmDesc")}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        onOpenChange={(o) => !o && setConfirm(null)}
        onConfirm={() => {
          setConfirm(null);
          void run(async () => {
            await useAssistantStore.getState().deleteConversations(null);
            toast.success(t("settings.assistant.deleteAllDone"));
          });
        }}
      />
    </section>
  );
}
