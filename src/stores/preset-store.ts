import { create } from "zustand";

import { type GenerationParams } from "@/lib/tauri/jobs";
import { type Preset, presetDelete, presetList, presetSave } from "@/lib/tauri/presets";

interface PresetState {
  presets: Preset[];
  loaded: boolean;

  load: () => Promise<void>;
  /** 失敗時は `AppError` を throw する。同名の別プリセットがあると VALIDATION(name)。 */
  save: (args: { id?: string; name: string; params: GenerationParams }) => Promise<Preset>;
  remove: (id: string) => Promise<void>;
}

const byName = (a: Preset, b: Preset) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

export const usePresetStore = create<PresetState>((set, get) => ({
  presets: [],
  loaded: false,

  load: async () => {
    try {
      set({ presets: await presetList(), loaded: true });
    } catch {
      set({ loaded: true });
    }
  },

  save: async (args) => {
    const saved = await presetSave(args);
    set((s) => ({ presets: [...s.presets.filter((p) => p.id !== saved.id), saved].sort(byName) }));
    return saved;
  },

  remove: async (id) => {
    await presetDelete(id);
    set({ presets: get().presets.filter((p) => p.id !== id) });
  },
}));
