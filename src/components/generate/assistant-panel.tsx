"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Textarea } from "@/components/ui/textarea";
import { diffRows, listChanges, usableChanges } from "@/lib/generation/suggestions";
import { canRelease, formatMemory, panelGate, sendsExternally } from "@/lib/llm/logic";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useLanguage, useT } from "@/lib/i18n/use-t";
import { switchTarget } from "@/lib/models/profile";
import { assistantUpdate, type Suggestion } from "@/lib/tauri/assistant";
import { toAppError } from "@/lib/tauri/errors";
import { type ChatMessage, useAssistantStore } from "@/stores/assistant-store";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { useLlmStore } from "@/stores/llm-store";
import { useModelsStore } from "@/stores/models-store";

/** AI に相談するパネル(docs/11-ai-assistant.md §2)。 */
export function AssistantPanel({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const t = useT();
  const status = useAssistantStore((s) => s.status);
  const llm = useLlmStore((s) => s.status);
  const gate = panelGate(status, llm);

  useEffect(() => {
    if (open) {
      void useAssistantStore.getState().loadStatus();
      void useAssistantStore.getState().loadConversations();
      void useLlmStore.getState().load();
    }
  }, [open]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full sm:max-w-lg">
        <SheetHeader className="pb-0">
          <SheetTitle>{t("assistant.title")}</SheetTitle>
          <SheetDescription>{t("assistant.desc")}</SheetDescription>
        </SheetHeader>
        {gate === "noKey" ? (
          <NoKey />
        ) : gate === "consent" ? (
          <Consent />
        ) : gate === "notInstalled" ? (
          <LocalNotInstalled />
        ) : gate === "chat" ? (
          <Chat />
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

function NoKey() {
  const t = useT();
  return (
    <div className="flex flex-col gap-3 px-4">
      <p className="font-medium">{t("assistant.noKey.title")}</p>
      <p className="text-sm text-muted-foreground">{t("assistant.noKey.desc")}</p>
      <div>
        <Button render={<Link href="/settings?tab=assistant" />} nativeButton={false} variant="outline">
          {t("assistant.noKey.link")}
        </Button>
      </div>
    </div>
  );
}

/** アプリ管理のローカル LLM が未導入のときの案内(docs/30 §6)。 */
function LocalNotInstalled() {
  const t = useT();
  return (
    <div className="flex flex-col gap-3 px-4">
      <p className="font-medium">{t("assistant.local.notInstalled.title")}</p>
      <p className="text-sm text-muted-foreground">{t("assistant.local.notInstalled.desc")}</p>
      <div>
        <Button render={<Link href="/settings?tab=assistant" />} nativeButton={false} variant="outline">
          {t("assistant.noKey.link")}
        </Button>
      </div>
    </div>
  );
}

/** ローカル LLM の状態(読み込み中の案内と、メモリの解放)。ローカルを使うときだけ出す。 */
function LocalStatusBar() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const provider = useAssistantStore((s) => s.status?.provider);
  const llm = useLlmStore((s) => s.status);
  const busy = useLlmStore((s) => s.busy);
  if (provider !== "local" || !llm) return null;

  const label =
    llm.state === "starting"
      ? t("assistant.local.loading")
      : llm.state === "ready"
        ? t("assistant.local.ready").replace("{size}", formatMemory(llm.memoryHintMb ?? 0))
        : llm.state === "error"
          ? (llm.message ?? t("llm.state.error"))
          : t("assistant.local.idle");

  return (
    <div className="flex items-center justify-between gap-2 rounded-md bg-muted px-3 py-1.5 text-xs">
      <span role="status">{label}</span>
      {canRelease(llm.state) && (
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() =>
            void useLlmStore
              .getState()
              .stop()
              .catch((e) => toast.error(errorMessage(toAppError(e))))
          }
        >
          {t("llm.release")}
        </Button>
      )}
    </div>
  );
}

/** 初回の送信前に、外部へ送られる内容を示して同意を得る(docs/11 §5)。 */
function Consent() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const setStatus = useAssistantStore((s) => s.setStatus);
  const [busy, setBusy] = useState(false);

  async function agree() {
    setBusy(true);
    try {
      setStatus(await assistantUpdate({ consent: true }));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-3 px-4">
      <p className="font-medium">{t("assistant.consent.title")}</p>
      <p className="text-sm">{t("assistant.consent.body1")}</p>
      <ul className="list-disc pl-5 text-sm">
        <li>{t("assistant.consent.item1")}</li>
        <li>{t("assistant.consent.item2")}</li>
        <li>{t("assistant.consent.item3")}</li>
      </ul>
      <p className="text-sm text-muted-foreground">{t("assistant.consent.body2")}</p>
      <p className="text-xs text-muted-foreground">{t("assistant.cost")}</p>
      <div>
        <Button onClick={agree} disabled={busy}>
          {t("assistant.consent.agree")}
        </Button>
      </div>
    </div>
  );
}

function Chat() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const messages = useAssistantStore((s) => s.messages);
  const conversations = useAssistantStore((s) => s.conversations);
  const conversationId = useAssistantStore((s) => s.conversationId);
  const requestId = useAssistantStore((s) => s.requestId);
  const error = useAssistantStore((s) => s.error);
  const text = useAssistantStore((s) => s.draft);
  const setText = (v: string) => useAssistantStore.getState().setDraft(v);
  const endRef = useRef<HTMLDivElement>(null);
  const running = requestId !== null;
  const external = sendsExternally(useAssistantStore((s) => s.status));

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [messages]);

  async function send() {
    const q = text.trim();
    if (!q || running) return;
    try {
      await useAssistantStore.getState().send(q, useFormStore.getState().params);
      setText("");
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
      void useAssistantStore.getState().loadStatus();
    }
  }

  const historyItems = conversations.map((c) => ({ value: c.id, label: c.title }));

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 px-4 pb-4">
      <LocalStatusBar />
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={running || messages.length === 0}
          onClick={() => useAssistantStore.getState().newConversation()}
        >
          {t("assistant.new")}
        </Button>
        <Select
          value={conversationId ?? ""}
          items={historyItems}
          disabled={running || conversations.length === 0}
          onValueChange={(id) => id && void useAssistantStore.getState().openConversation(id)}
        >
          <SelectTrigger className="min-w-0 flex-1" aria-label={t("assistant.history")}>
            <SelectValue
              placeholder={conversations.length === 0 ? t("assistant.history.none") : t("assistant.history")}
            />
          </SelectTrigger>
          <SelectContent>
            {conversations.map((c) => (
              <SelectItem key={c.id} value={c.id}>
                {c.title}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto" aria-live="polite">
        {messages.length === 0 && <p className="text-sm text-muted-foreground">{t("assistant.empty")}</p>}
        {messages.map((m) => (
          <Message key={m.id} message={m} />
        ))}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(error)}
          </p>
        )}
        <div ref={endRef} />
      </div>

      <div className="flex flex-col gap-2">
        <Textarea
          rows={3}
          value={text}
          placeholder={t("assistant.input.placeholder")}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              void send();
            }
          }}
        />
        <div className="flex items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">{external ? t("assistant.cost") : t("assistant.localNote")}</p>
          {running ? (
            <Button variant="outline" onClick={() => void useAssistantStore.getState().cancel()}>
              {t("assistant.stop")}
            </Button>
          ) : (
            <Button onClick={() => void send()} disabled={text.trim() === ""}>
              {t("assistant.send")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

function Message({ message }: { message: ChatMessage }) {
  const t = useT();
  const mine = message.role === "user";
  return (
    <div className={mine ? "flex justify-end" : "flex flex-col items-start gap-2"}>
      <div
        className={
          mine
            ? "max-w-[85%] rounded-lg bg-primary px-3 py-2 text-sm whitespace-pre-wrap text-primary-foreground"
            : "max-w-full rounded-lg bg-muted px-3 py-2 text-sm whitespace-pre-wrap"
        }
      >
        {message.text || (message.pending ? t("assistant.thinking") : "")}
      </div>
      {!mine &&
        message.suggestions.map((s, i) =>
          s.model ? (
            <ModelSuggestionCard
              key={i}
              messageId={message.id}
              index={i}
              suggestion={s}
              disabled={message.pending === true}
            />
          ) : (
            <SuggestionCard
              key={i}
              messageId={message.id}
              index={i}
              suggestion={s}
              disabled={message.pending === true}
            />
          ),
        )}
    </div>
  );
}

/** 提案を差分で見せ、押したときだけフォームへ適用する(docs/11 §6)。 */
function SuggestionCard({
  messageId,
  index,
  suggestion,
  disabled,
}: {
  messageId: string;
  index: number;
  suggestion: Suggestion;
  disabled: boolean;
}) {
  const t = useT();
  const params = useFormStore((s) => s.params);
  const undoAvailable = useFormStore((s) => s.suggestionUndo !== null);
  const models = useEngineStore((s) => s.models);
  const [lastApplied, setLastApplied] = useState(false);

  const usable = usableChanges(params, suggestion.changes, models);
  const rows = diffRows(params, usable);
  const appliedRows = listChanges(suggestion.changes);

  function apply() {
    if (!useFormStore.getState().applySuggestion(suggestion.changes, models)) {
      toast.info(t("assistant.suggest.nothing"));
      return;
    }
    setLastApplied(true);
    void useAssistantStore.getState().markApplied(messageId, index);
    toast.success(t("assistant.suggest.appliedToast"));
  }

  function undo() {
    useFormStore.getState().undoSuggestion();
    setLastApplied(false);
    toast.info(t("assistant.suggest.undone"));
  }

  // 適用済みの提案で、まだ元に戻せるのは直近に適用した 1 件だけ
  const canUndo = lastApplied && undoAvailable;

  return (
    <div className="w-full rounded-lg border p-3 text-sm">
      <p className="font-medium">{suggestion.summary || t("assistant.suggest.apply")}</p>
      <ul className="mt-1 flex flex-col gap-0.5 text-xs">
        {suggestion.applied
          ? appliedRows.map((r) => (
              <Row key={r.field} label={t(`assistant.field.${r.field}` as MessageKey)} value={truncate(r.to)} />
            ))
          : rows.map((r) => (
              <Row
                key={r.field}
                label={t(`assistant.field.${r.field}` as MessageKey)}
                value={`${truncate(r.from)} → ${truncate(r.to)}`}
              />
            ))}
      </ul>
      <div className="mt-2 flex gap-2">
        {suggestion.applied ? (
          <>
            <span className="self-center text-xs text-muted-foreground">{t("assistant.suggest.applied")}</span>
            {canUndo && (
              <Button size="xs" variant="outline" onClick={undo}>
                {t("assistant.suggest.undo")}
              </Button>
            )}
          </>
        ) : (
          <Button size="xs" disabled={disabled || rows.length === 0} onClick={apply}>
            {t("assistant.suggest.apply")}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * モデルの提案(docs/18 §6.3)。導入済みなら [このモデルに切り替える]、未導入なら [モデル取得ページで開く]。
 * 切り替えは、必ずユーザーがボタンを押したときだけ行う。
 */
function ModelSuggestionCard({
  messageId,
  index,
  suggestion,
  disabled,
}: {
  messageId: string;
  index: number;
  suggestion: Suggestion;
  disabled: boolean;
}) {
  const t = useT();
  const lang = useLanguage();
  const catalog = useModelsStore((s) => s.view?.catalog);
  const models = useEngineStore((s) => s.models);

  // カタログが未取得なら取り直す(提案の表示に、モデル名と導入状況が要る)
  useEffect(() => {
    if (!useModelsStore.getState().view) void useModelsStore.getState().load();
  }, []);

  const model = suggestion.model;
  if (!model) return null;
  const family = catalog?.families.find((f) => f.id === model.familyId);
  const target = family ? switchTarget(family, models) : null;

  function switchTo() {
    if (!target) return;
    useFormStore.getState().switchModel(target.kind, target.name, models);
    void useAssistantStore.getState().markApplied(messageId, index);
    toast.success(t("assistant.model.switched"));
  }

  return (
    <div className="w-full rounded-lg border p-3 text-sm">
      <p className="text-xs text-muted-foreground">{t("assistant.model.title")}</p>
      <p className="font-medium">{family ? family.name[lang] : model.familyId}</p>
      <p className="mt-1 text-xs whitespace-pre-wrap">{model.reason}</p>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        {suggestion.applied ? (
          <span className="text-xs text-muted-foreground">{t("assistant.model.done")}</span>
        ) : target ? (
          <Button size="xs" disabled={disabled} onClick={switchTo}>
            {t("assistant.model.switch")}
          </Button>
        ) : (
          <>
            <span className="text-xs text-muted-foreground">{t("assistant.model.notInstalled")}</span>
            <Button
              size="xs"
              variant="outline"
              render={<Link href={`/models?family=${encodeURIComponent(model.familyId)}`} />}
              nativeButton={false}
            >
              {t("assistant.model.open")}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

const truncate = (s: string, n = 60) => (s.length > n ? `${s.slice(0, n)}…` : s);

function Row({ label, value }: { label: string; value: string }) {
  return (
    <li className="flex gap-2">
      <span className="w-28 shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 break-words">{value}</span>
    </li>
  );
}
