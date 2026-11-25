import { create } from "zustand";

import { toAppError } from "@/lib/tauri/errors";
import {
  type InstallState,
  type InstallView,
  type Preflight,
  engineInstallCancel,
  engineInstallLogs,
  engineInstallPreflight,
  engineInstallRemove,
  engineInstallStart,
  engineInstallStatus,
} from "@/lib/tauri/engine-install";
import { useEngineProcessStore } from "@/stores/engine-process-store";

/** 画面で保持するログの行数(Rust 側と同じ)。 */
export const INSTALL_LOG_LINES = 500;

interface EngineInstallStoreState {
  view: InstallView | null;
  /** 導入の状態(イベントで更新)。 */
  state: InstallState | null;
  logs: string[];
  preflight: Preflight | null;
  preflightLoading: boolean;

  load: () => Promise<void>;
  loadLogs: () => Promise<void>;
  applyState: (s: InstallState) => void;
  appendLog: (line: string) => void;
  /** 選択したコンポーネントで事前チェックする(何もダウンロードしない)。 */
  runPreflight: (components: string[]) => Promise<void>;
  /** 失敗時は `AppError` を throw する(`ENGINE_BUSY`・`ENGINE_INSTALL_DISK_FULL` など)。 */
  start: (components: string[]) => Promise<void>;
  cancel: () => Promise<void>;
  remove: (keepData: boolean) => Promise<void>;
}

export const useEngineInstallStore = create<EngineInstallStoreState>((set, get) => ({
  view: null,
  state: null,
  logs: [],
  preflight: null,
  preflightLoading: false,

  load: async () => {
    try {
      const view = await engineInstallStatus();
      set({ view, state: view.state });
    } catch (e) {
      // 取得に失敗しても、他の画面を妨げない
      console.warn("engine_install_status failed", toAppError(e));
    }
  },

  loadLogs: async () => {
    try {
      set({ logs: (await engineInstallLogs(INSTALL_LOG_LINES)).lines });
    } catch {
      /* 次のイベントで回復する */
    }
  },

  applyState: (s) => {
    const prev = get().state?.status;
    set({ state: s });
    // 完了したら、導入の記録と、起動管理の設定(自動で設定される)を取り直す
    if (s.status === "done" && prev !== "done") {
      void get().load();
      void useEngineProcessStore.getState().load();
    }
  },

  appendLog: (line) => set((st) => ({ logs: [...st.logs, line].slice(-INSTALL_LOG_LINES) })),

  runPreflight: async (components) => {
    set({ preflightLoading: true });
    try {
      set({ preflight: await engineInstallPreflight(components) });
    } finally {
      set({ preflightLoading: false });
    }
  },

  start: async (components) => {
    set({ logs: [] });
    const s = await engineInstallStart(components, true);
    set({ state: s });
  },

  cancel: async () => {
    await engineInstallCancel();
  },

  remove: async (keepData) => {
    const view = await engineInstallRemove(keepData);
    set({ view, state: view.state, logs: [] });
    void useEngineProcessStore.getState().load();
  },
}));
