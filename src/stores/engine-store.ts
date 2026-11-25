import { create } from "zustand";

import {
  type EngineModels,
  type EngineStatus,
  engineListModels,
  engineReconnect,
  engineStatus,
} from "@/lib/tauri/engine";
import { type AppError, toAppError } from "@/lib/tauri/errors";

interface EngineState {
  status: EngineStatus | null;
  models: EngineModels | null;
  modelsLoading: boolean;
  modelsError: AppError | null;
  reconnecting: boolean;
  /** `engine://status` イベントや初期取得の結果を反映する。 */
  applyStatus: (status: EngineStatus) => void;
  refresh: () => Promise<void>;
  reconnect: () => Promise<void>;
  loadModels: () => Promise<void>;
}

export const useEngineStore = create<EngineState>((set, get) => ({
  status: null,
  models: null,
  modelsLoading: false,
  modelsError: null,
  reconnecting: false,

  applyStatus: (status) => {
    const prev = get().status;
    set({ status });
    // 接続が回復した、または接続先が変わったらモデル一覧を取り直す
    if (status.connected && (!prev?.connected || prev.url !== status.url)) {
      void get().loadModels();
    }
    if (!status.connected) {
      set({ models: null, modelsError: null });
    }
  },

  refresh: async () => {
    try {
      get().applyStatus(await engineStatus());
    } catch {
      /* 状態取得の失敗は次回のイベントで回復する */
    }
  },

  reconnect: async () => {
    set({ reconnecting: true });
    try {
      get().applyStatus(await engineReconnect());
    } catch {
      /* 同上 */
    } finally {
      set({ reconnecting: false });
    }
  },

  loadModels: async () => {
    set({ modelsLoading: true, modelsError: null });
    try {
      set({ models: await engineListModels(), modelsLoading: false });
    } catch (e) {
      set({ modelsError: toAppError(e), modelsLoading: false });
    }
  },
}));
