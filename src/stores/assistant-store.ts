import { create } from "zustand";

import {
  type AssistantStatus,
  assistantAsk,
  assistantCancel,
  assistantConversationDelete,
  assistantConversationGet,
  assistantConversationList,
  assistantStatus,
  assistantSuggestionApplied,
  type ConversationMessage,
  type ConversationSummary,
  type ModelSuggestion,
  type SuggestedChanges,
  type Suggestion,
} from "@/lib/tauri/assistant";
import { type AppError, toAppError } from "@/lib/tauri/errors";
import { modelContext } from "@/lib/models/profile";
import type { GenerationParams } from "@/lib/tauri/jobs";
import { useEngineStore } from "@/stores/engine-store";
import { useModelsStore } from "@/stores/models-store";

/** 表示用のメッセージ。回答の生成中は `id` が確定前(`pending = true`)で、本文が増えていく。 */
export interface ChatMessage extends Omit<ConversationMessage, "createdAt"> {
  pending?: boolean;
}

interface AssistantState {
  status: AssistantStatus | null;
  conversations: ConversationSummary[];
  conversationId: string | null;
  messages: ChatMessage[];
  /** 実行中の相談(null = なし)。 */
  requestId: string | null;
  error: AppError | null;
  /** 相談パネルの開閉(パネルは生成フォームの外に置くため、ストアで共有する)。 */
  panelOpen: boolean;
  /** 入力欄の下書き(「AI に相談」ボタンから仮入力するため、ストアで持つ)。 */
  draft: string;

  setPanelOpen: (open: boolean) => void;
  setDraft: (draft: string) => void;
  /** 下書きを入れて、パネルを開く(送信はユーザー操作)。 */
  openWithDraft: (draft: string) => void;
  loadStatus: () => Promise<void>;
  setStatus: (s: AssistantStatus) => void;
  loadConversations: () => Promise<void>;
  /** 失敗時は `AppError` を throw する(同意なし・キーなしなど)。 */
  send: (text: string, params: GenerationParams) => Promise<void>;
  cancel: () => Promise<void>;
  openConversation: (id: string) => Promise<void>;
  newConversation: () => void;
  deleteConversations: (ids: string[] | null) => Promise<void>;
  markApplied: (messageId: string, index: number) => Promise<void>;

  // イベントの反映(`requestId` が現在の相談と一致するものだけ)
  onDelta: (requestId: string, text: string) => void;
  onSuggestion: (
    requestId: string,
    index: number,
    summary: string,
    changes: SuggestedChanges,
    model?: ModelSuggestion,
  ) => void;
  onDone: (requestId: string) => void;
  onError: (requestId: string, error: AppError) => void;
}


export const useAssistantStore = create<AssistantState>((set, get) => ({
  status: null,
  conversations: [],
  conversationId: null,
  messages: [],
  requestId: null,
  error: null,
  panelOpen: false,
  draft: "",

  setPanelOpen: (panelOpen) => set({ panelOpen }),
  setDraft: (draft) => set({ draft }),
  openWithDraft: (draft) => set({ draft, panelOpen: true }),

  loadStatus: async () => {
    try {
      set({ status: await assistantStatus() });
    } catch {
      // 設定画面・パネルで再試行できるため、失敗は黙って無視する
    }
  },

  setStatus: (status) => set({ status }),

  loadConversations: async () => {
    try {
      set({ conversations: await assistantConversationList() });
    } catch {
      /* 同上 */
    }
  },

  send: async (text, params) => {
    set({ error: null });
    // 選択中・導入済みのモデルは、ファミリー ID だけを添える(カタログが未取得なら添えない。docs/18 §6.2)
    const models = modelContext(useModelsStore.getState().view?.catalog, params, useEngineStore.getState().models);
    const started = await assistantAsk(get().conversationId, text, params, models);
    set((s) => ({
      conversationId: started.conversationId,
      requestId: started.requestId,
      messages: [
        ...s.messages,
        { id: `user-${started.requestId}`, role: "user", text, suggestions: [] },
        { id: started.assistantMessageId, role: "assistant", text: "", suggestions: [], pending: true },
      ],
    }));
  },

  cancel: async () => {
    const id = get().requestId;
    if (id) await assistantCancel(id).catch(() => {});
  },

  openConversation: async (id) => {
    try {
      const c = await assistantConversationGet(id);
      set({
        conversationId: c.id,
        messages: c.messages.map((m) => ({ id: m.id, role: m.role, text: m.text, suggestions: m.suggestions })),
        error: null,
      });
    } catch (e) {
      // 自動削除などで消えている場合は一覧を更新する
      set({ error: toAppError(e) });
      await get().loadConversations();
    }
  },

  newConversation: () => {
    if (get().requestId) return; // 実行中は切り替えない
    set({ conversationId: null, messages: [], error: null });
  },

  deleteConversations: async (ids) => {
    await assistantConversationDelete(ids);
    const current = get().conversationId;
    if (current && (ids === null || ids.includes(current)) && !get().requestId) {
      set({ conversationId: null, messages: [] });
    }
    await get().loadConversations();
  },

  markApplied: async (messageId, index) => {
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === messageId
          ? { ...m, suggestions: m.suggestions.map((x, i) => (i === index ? { ...x, applied: true } : x)) }
          : m,
      ),
    }));
    await assistantSuggestionApplied(messageId, index).catch(() => {});
  },

  onDelta: (requestId, text) => {
    if (get().requestId !== requestId) return;
    set((s) => ({
      messages: s.messages.map((m) => (m.pending ? { ...m, text: m.text + text } : m)),
    }));
  },

  onSuggestion: (requestId, index, summary, changes, model) => {
    if (get().requestId !== requestId) return;
    const sug: Suggestion = { summary, changes, applied: false, ...(model && { model }) };
    set((s) => ({
      messages: s.messages.map((m) => {
        if (!m.pending) return m;
        const list = [...m.suggestions];
        list[index] = sug;
        return { ...m, suggestions: list };
      }),
    }));
  },

  onDone: (requestId) => {
    if (get().requestId !== requestId) return;
    set((s) => ({
      requestId: null,
      messages: s.messages
        // 何も返ってこなかった回答は表示から外す
        .filter((m) => !(m.pending && m.text.trim() === "" && m.suggestions.length === 0))
        .map((m) => ({ ...m, pending: false })),
    }));
    void get().loadConversations();
  },

  onError: (requestId, error) => {
    if (get().requestId !== requestId) return;
    set((s) => ({
      requestId: null,
      error,
      // 途中まで届いた回答はサーバー側でも保存されるため、残す
      messages: s.messages
        .filter((m) => !(m.pending && m.text.trim() === "" && m.suggestions.length === 0))
        .map((m) => ({ ...m, pending: false })),
    }));
    void get().loadConversations();
  },
}));

