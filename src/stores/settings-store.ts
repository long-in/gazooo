import { create } from "zustand";

import type { AppError } from "@/lib/tauri/errors";
import { toAppError } from "@/lib/tauri/errors";
import {
  defaultSettings,
  type Settings,
  type SettingsPatch,
  settingsGet,
  settingsUpdate,
} from "@/lib/tauri/settings";

interface SettingsState {
  settings: Settings;
  loaded: boolean;
  loadError: AppError | null;
  load: () => Promise<void>;
  /** 失敗時は `AppError` を throw する(呼び出し側で表示)。 */
  update: (patch: SettingsPatch) => Promise<Settings>;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  settings: defaultSettings,
  loaded: false,
  loadError: null,
  load: async () => {
    try {
      const settings = await settingsGet();
      set({ settings, loaded: true, loadError: null });
    } catch (e) {
      set({ loaded: true, loadError: toAppError(e) });
    }
  },
  update: async (patch) => {
    const settings = await settingsUpdate(patch);
    set({ settings });
    return settings;
  },
}));
