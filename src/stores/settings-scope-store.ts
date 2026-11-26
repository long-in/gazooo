import { create } from "zustand";

import { type SettingsGroup, settingsScopeGet, settingsScopeResetGlobal, settingsScopeSet } from "@/lib/tauri/settings";
import { useSettingsStore } from "@/stores/settings-store";

interface SettingsScopeState {
  /** 個別設定にしている組。 */
  overridden: SettingsGroup[];
  /** 値の切り替えのたびに増える。画面はこれをキーにして値を読み直す。 */
  revision: number;
  load: () => Promise<void>;
  /** 失敗時は `AppError` を throw する。 */
  setOverride: (group: SettingsGroup, overridden: boolean) => Promise<void>;
  resetGlobal: (group: SettingsGroup) => Promise<void>;
}

export const useSettingsScopeStore = create<SettingsScopeState>((set, get) => {
  const changed = async () => {
    await get().load();
    // テーマ・言語はアプリ全体へすぐ反映する
    await useSettingsStore.getState().load();
    set((s) => ({ revision: s.revision + 1 }));
  };
  return {
    overridden: [],
    revision: 0,
    load: async () => set({ overridden: await settingsScopeGet() }),
    setOverride: async (group, overridden) => {
      await settingsScopeSet(group, overridden);
      await changed();
    },
    resetGlobal: async (group) => {
      await settingsScopeResetGlobal(group);
      await changed();
    },
  };
});
