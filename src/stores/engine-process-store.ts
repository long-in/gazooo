import { create } from "zustand";

import { toAppError } from "@/lib/tauri/errors";
import {
  type EngineProcState,
  type EngineSuggestions,
  type ManagedEngineConfig,
  type ManagedView,
  engineManagedGet,
  engineManagedSet,
  engineManagedSuggest,
  engineProcessLogs,
  engineProcessRestart,
  engineProcessStart,
  engineProcessStop,
} from "@/lib/tauri/engine-process";

/** 画面で保持するログの行数(Rust 側と同じ)。 */
export const LOG_LINES = 500;

interface EngineProcessStoreState {
  view: ManagedView | null;
  /** 起動管理の状態(イベントで更新)。 */
  state: EngineProcState | null;
  /** 入力候補。`null` は探索中(docs/32)。失敗・見つからないときは `{}`。 */
  suggestions: EngineSuggestions | null;
  logs: string[];
  /** 起動・停止・再起動の操作中。 */
  busy: boolean;

  load: () => Promise<void>;
  loadSuggestions: () => Promise<void>;
  loadLogs: () => Promise<void>;
  applyState: (s: EngineProcState) => void;
  appendLog: (line: string) => void;
  /** 失敗時は `AppError` を throw する(検証エラー `VALIDATION` など)。 */
  save: (config: ManagedEngineConfig) => Promise<void>;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  /** 実行中のジョブがあり `force` でなければ `ENGINE_BUSY` を throw する。 */
  restart: (force?: boolean) => Promise<void>;
}

export const useEngineProcessStore = create<EngineProcessStoreState>((set, get) => {
  /** 操作中フラグを立てて実行し、結果の状態を反映する。 */
  async function run(op: () => Promise<EngineProcState>) {
    set({ busy: true });
    try {
      get().applyState(await op());
    } finally {
      set({ busy: false });
    }
  }

  return {
    view: null,
    state: null,
    suggestions: null,
    logs: [],
    busy: false,

    load: async () => {
      try {
        const view = await engineManagedGet();
        set({ view, state: view.state });
      } catch (e) {
        // 起動管理の取得失敗は、他の画面を妨げない(設定画面で再取得できる)
        console.warn("engine_managed_get failed", toAppError(e));
      }
    },

    loadSuggestions: async () => {
      try {
        set({ suggestions: await engineManagedSuggest() });
      } catch {
        // 候補の探索失敗は「候補なし」として扱う(他の画面を妨げない)
        set({ suggestions: {} });
      }
    },

    loadLogs: async () => {
      try {
        set({ logs: (await engineProcessLogs(LOG_LINES)).lines });
      } catch {
        /* ログの取得失敗は、次のイベントで回復する */
      }
    },

    applyState: (s) => set({ state: s }),

    appendLog: (line) =>
      set((st) => ({ logs: [...st.logs, line].slice(-LOG_LINES) })),

    save: async (config) => {
      const view = await engineManagedSet(config);
      set({ view, state: view.state });
    },

    start: () => run(engineProcessStart),
    stop: () => run(engineProcessStop),
    restart: (force) => run(() => engineProcessRestart(force)),
  };
});
