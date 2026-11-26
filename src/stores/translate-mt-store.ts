import { create } from "zustand";

import { toAppError } from "@/lib/tauri/errors";
import type { LlmInstallState, LlmState, LlmStatus } from "@/lib/tauri/llm";
import {
  type LicenseLang,
  translateMtInstall,
  translateMtInstallCancel,
  translateMtLicense,
  translateMtRemove,
  translateMtStatus,
} from "@/lib/tauri/translate";
import { translateStatus, translateUpdate } from "@/lib/tauri/translate";
import { useTranslateStore } from "@/stores/translate-store";

interface TranslateMtState {
  status: LlmStatus | null;
  /** 削除などの操作中。 */
  busy: boolean;
  load: () => Promise<void>;
  applyState: (state: LlmState, message: string | null) => void;
  /** `translate-mt://install-progress` を反映する。終わったら状態と翻訳の実行可否を取り直す。 */
  applyInstall: (s: LlmInstallState) => void;
  /** 失敗時は `AppError` を throw する。 */
  install: () => Promise<void>;
  cancelInstall: () => Promise<void>;
  remove: () => Promise<void>;
  /** ライセンス全文。 */
  readLicense: (lang: LicenseLang) => Promise<string>;
}

/** 翻訳専用モデル(docs/34 §9.13)の導入状態。本体のローカル LLM のストアとは別。 */
export const useTranslateMtStore = create<TranslateMtState>((set, get) => ({
  status: null,
  busy: false,

  load: async () => {
    try {
      set({ status: await translateMtStatus() });
    } catch (e) {
      console.warn("translate_mt_status failed", toAppError(e));
    }
  },

  applyState: (state, message) => {
    const cur = get().status;
    if (!cur || !cur.installed) return;
    set({ status: { ...cur, state, message } });
  },

  applyInstall: (install) => {
    const cur = get().status;
    if (cur) set({ status: { ...cur, install } });
    if (
      install.status === "done" ||
      install.status === "failed" ||
      install.status === "cancelled"
    ) {
      void get()
        .load()
        .then(() => useTranslateStore.getState().loadStatus());
    }
  },

  install: async () => {
    await translateMtInstall();
  },

  cancelInstall: async () => {
    const id = get().status?.install.installId;
    if (id) await translateMtInstallCancel(id);
  },

  remove: async () => {
    set({ busy: true });
    try {
      set({ status: await translateMtRemove() });
      // 削除したら、翻訳エンジンを元に戻す(未導入のまま専用モデルを選び続けない)
      // 全体の操作なので全体設定を直す。本人が個別に専用モデルを選んでいた場合だけ、個別設定も直す
      if ((await translateStatus("global")).mtModel === "dedicated")
        await translateUpdate({ scope: "global", mtModel: "general" });
      const t = useTranslateStore.getState();
      await t.loadStatus();
      if (useTranslateStore.getState().status?.mtModel === "dedicated")
        await t.update({ mtModel: "general" });
    } finally {
      set({ busy: false });
    }
  },

  readLicense: async (lang) => (await translateMtLicense(lang)).text,
}));
