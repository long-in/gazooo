import { invoke } from "./invoke";
import type { GenerationParams } from "./jobs";

export interface Preset {
  id: string;
  name: string;
  params: GenerationParams;
  createdAt: number;
  updatedAt: number;
}

export const presetList = () => invoke<Preset[]>("preset_list");

/** `id` なしは新規、ありは上書き・名前変更。同名が別にあると VALIDATION(details.field = "name")。 */
export const presetSave = (args: { id?: string; name: string; params: GenerationParams }) =>
  invoke<Preset>("preset_save", args);

export const presetDelete = (id: string) => invoke<void>("preset_delete", { id });
