import type { GenerationParams } from "./jobs";
import { invoke } from "./invoke";

export type Theme = "light" | "dark" | "system";
export type Language = "ja" | "en";

export interface Settings {
  comfyuiUrl: string;
  /** null = アプリデータ配下の既定ディレクトリ */
  imagesDir: string | null;
  theme: Theme;
  /** null = OS のロケールに従う */
  language: Language | null;
  /** 生成フォームの下書き。null = 組み込みの既定値。 */
  generationDefaults: GenerationParams | null;
  /** 画面の状態(パネルの開閉・幅・密度。docs/29 §6)。null = 既定。 */
  uiState: Record<string, unknown> | null;
}

/** 未指定のキーは変更されない。`imagesDir` / `language` に null を渡すと既定に戻る。 */
export type SettingsPatch = Partial<Settings>;

export const defaultSettings: Settings = {
  comfyuiUrl: "http://127.0.0.1:8188",
  imagesDir: null,
  theme: "dark",
  language: null,
  generationDefaults: null,
  uiState: null,
};

export const settingsGet = () => invoke<Settings>("settings_get");

export const settingsUpdate = (patch: SettingsPatch) =>
  invoke<Settings>("settings_update", { patch });
