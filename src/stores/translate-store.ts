import { create } from "zustand";

import {
  type TranslatePatch,
  type TranslateStatus,
  translateStatus,
  translateUpdate,
} from "@/lib/tauri/translate";

interface TranslateState {
  status: TranslateStatus | null;
  loadStatus: () => Promise<void>;
  /** 設定を即時に保存する。失敗時は `AppError` を throw する。 */
  update: (patch: TranslatePatch) => Promise<void>;
}

/** プロンプト翻訳の設定と実行可否(docs/34)。AI サポートのストアとは別。 */
export const useTranslateStore = create<TranslateState>((set) => ({
  status: null,
  loadStatus: async () => {
    try {
      set({ status: await translateStatus() });
    } catch {
      // Tauri 外・取得失敗時は未取得のまま(ボタンは実行時のエラーで案内する)
    }
  },
  update: async (patch) => {
    set({ status: await translateUpdate(patch) });
  },
}));
