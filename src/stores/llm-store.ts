import { create } from "zustand";

import { toAppError } from "@/lib/tauri/errors";
import {
  type LlmInstallState,
  type LlmState,
  type LlmStatus,
  llmInstall,
  llmInstallCancel,
  llmLogs,
  llmRemove,
  llmStart,
  llmStatus,
  llmStop,
  llmUpdateSettings,
} from "@/lib/tauri/llm";

interface LlmStoreState {
  status: LlmStatus | null;
  logs: string[];
  /** 起動・停止・削除などの操作中。 */
  busy: boolean;

  load: () => Promise<void>;
  loadLogs: () => Promise<void>;
  setStatus: (s: LlmStatus) => void;
  /** `llm://state`(起動・停止の遷移)を反映する。未導入の状態は `status` から決まるため、導入済みのときだけ。 */
  applyState: (state: LlmState, message: string | null) => void;
  /** `llm://install-progress` を反映する。完了したら導入状態を取り直す。 */
  applyInstall: (s: LlmInstallState) => void;
  /** 失敗時は `AppError` を throw する(`LLM_BUSY` など)。 */
  install: () => Promise<void>;
  cancelInstall: () => Promise<void>;
  start: () => Promise<void>;
  /** 停止してメモリを解放する。 */
  stop: () => Promise<void>;
  remove: () => Promise<void>;
  updateSettings: (patch: { idleMinutes?: number; releaseBeforeGeneration?: boolean }) => Promise<void>;
}

export const useLlmStore = create<LlmStoreState>((set, get) => {
  /** 操作中フラグを立てて実行し、結果の状態を反映する。 */
  async function run(op: () => Promise<LlmStatus>) {
    set({ busy: true });
    try {
      set({ status: await op() });
    } finally {
      set({ busy: false });
    }
  }

  return {
    status: null,
    logs: [],
    busy: false,

    load: async () => {
      try {
        set({ status: await llmStatus() });
      } catch (e) {
        // 取得に失敗しても、他の画面を妨げない
        console.warn("llm_status failed", toAppError(e));
      }
    },

    loadLogs: async () => {
      try {
        set({ logs: await llmLogs() });
      } catch {
        // ログの取得失敗は無視
      }
    },

    setStatus: (status) => set({ status }),

    applyState: (state, message) => {
      const cur = get().status;
      if (!cur || !cur.installed) return;
      set({ status: { ...cur, state, message } });
    },

    applyInstall: (install) => {
      const cur = get().status;
      if (cur) set({ status: { ...cur, install } });
      // 完了・失敗・中止のあとは、導入済みかどうかを含めて取り直す
      if (install.status === "done" || install.status === "failed" || install.status === "cancelled") {
        void get().load();
      }
    },

    install: async () => {
      await llmInstall();
    },

    cancelInstall: async () => {
      const id = get().status?.install.installId;
      if (id) await llmInstallCancel(id);
    },

    start: () => run(llmStart),
    stop: () => run(llmStop),
    remove: () => run(llmRemove),

    updateSettings: async (patch) => {
      set({ status: await llmUpdateSettings(patch) });
    },
  };
});
