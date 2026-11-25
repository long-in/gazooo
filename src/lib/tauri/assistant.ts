import { invoke } from "./invoke";
import type { GenerationParams } from "./jobs";

/** AI サポート(docs/11-ai-assistant.md、docs/04-ipc-api.md「AI サポート」)。キー本体は取得できない。 */
export interface AssistantStatus {
  hasKey: boolean;
  verified: boolean;
  model: string;
  consent: boolean;
  autoDelete: boolean;
  provider: AssistantProvider;
  /** 外部サーバ(`local_external`)の接続先とモデル名。 */
  localBaseUrl: string;
  localModel: string;
}

/**
 * 送信先(docs/11 §13、docs/30 §6)。`anthropic` = Claude API、`local` = アプリが管理するローカル LLM、
 * `local_external` = ユーザーが起動した OpenAI 互換のローカルサーバ(Ollama 等)。
 */
export type AssistantProvider = "anthropic" | "local" | "local_external";

/** 外部サーバへの接続の確認結果(docs/11 §13.4)。 */
export interface LocalCheck {
  reachable: boolean;
  modelFound: boolean;
  models: string[];
}

export const ASSISTANT_MODELS = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"] as const;
/** 保存する会話の最大数(自動削除が有効なとき)。 */
export const MAX_CONVERSATIONS = 30;

/** 提案できる項目(Rust 側で検証済み)。変えたい項目だけが入る。 */
export interface SuggestedChanges {
  prompt?: string;
  negativePrompt?: string;
  width?: number;
  height?: number;
  steps?: number;
  cfg?: number;
  guidance?: number;
  sampler?: string;
  scheduler?: string;
  batchSize?: number;
  faceDetailer?: { enabled?: boolean; denoise?: number };
}

/** モデルの提案(docs/18 §6.3)。`familyId` はカタログにあるものだけ(Rust 側で検証済み)。 */
export interface ModelSuggestion {
  familyId: string;
  reason: string;
}

export interface Suggestion {
  summary: string;
  changes: SuggestedChanges;
  applied: boolean;
  /** あればモデルの提案(`changes` は空)。 */
  model?: ModelSuggestion;
}

/** AI へ添える選択中・導入済みのモデル。ファミリー ID だけ(ファイル名・パスは送らない。docs/18 §6.2)。 */
export interface ModelContext {
  familyId: string | null;
  installedFamilies: string[];
}

export interface ConversationSummary {
  id: string;
  title: string;
  updatedAt: number;
}

export interface ConversationMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  suggestions: Suggestion[];
  createdAt: number;
}

export interface Conversation {
  id: string;
  title: string;
  messages: ConversationMessage[];
}

export interface AskStarted {
  requestId: string;
  conversationId: string;
  assistantMessageId: string;
}

export const assistantStatus = () => invoke<AssistantStatus>("assistant_status");

export const assistantSetKey = (key: string) => invoke<{ verified: boolean }>("assistant_set_key", { key });

export const assistantClearKey = () => invoke<void>("assistant_clear_key");

export interface AssistantPatch {
  model?: string;
  consent?: boolean;
  autoDelete?: boolean;
  provider?: AssistantProvider;
  localBaseUrl?: string;
  localModel?: string;
}

export const assistantUpdate = (patch: AssistantPatch) => invoke<AssistantStatus>("assistant_update", { patch });

/** 外部サーバへの接続の確認。省略した項目は保存値を使う。届かないときもエラーにはならない(`reachable = false`)。 */
export const assistantLocalCheck = (baseUrl?: string, model?: string) =>
  invoke<LocalCheck>("assistant_local_check", { baseUrl: baseUrl ?? null, model: model ?? null });

/** システムプロンプトの編集(docs/11 §12)。 */
export interface PromptState {
  text: string;
  defaultText: string;
  customized: boolean;
  modelGuide: string;
}

/** 保存できる最大文字数(Rust 側と同じ)。 */
export const MAX_SYSTEM_PROMPT_CHARS = 30000;

export const assistantPromptGet = () => invoke<PromptState>("assistant_prompt_get");

export const assistantPromptSet = (text: string) => invoke<{ customized: boolean }>("assistant_prompt_set", { text });

export const assistantPromptReset = () => invoke<{ text: string }>("assistant_prompt_reset");

export const assistantAsk = (
  conversationId: string | null,
  text: string,
  params: GenerationParams,
  models?: ModelContext,
) => invoke<AskStarted>("assistant_ask", { conversationId, text, params, models: models ?? null });

export const assistantCancel = (requestId: string) => invoke<void>("assistant_cancel", { requestId });

export const assistantConversationList = () => invoke<ConversationSummary[]>("assistant_conversation_list");

export const assistantConversationGet = (id: string) => invoke<Conversation>("assistant_conversation_get", { id });

/** `ids = null` はすべて削除。 */
export const assistantConversationDelete = (ids: string[] | null) =>
  invoke<{ deleted: number }>("assistant_conversation_delete", { ids });

export const assistantSuggestionApplied = (messageId: string, index: number) =>
  invoke<void>("assistant_suggestion_applied", { messageId, index });
