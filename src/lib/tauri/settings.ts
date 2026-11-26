import type { GenerationParams } from "./jobs";
import { invoke } from "./invoke";

export type Theme = "light" | "dark" | "system";
export type Language = "ja" | "en";

/** 全体設定 / 個別設定(docs/52)。使う値は「個別(あれば)→ 全体 → 標準値」。 */
export type SettingsScope = "global" | "user";

/** 上書きできる項目の組(docs/52 §2 B。画像の保存先は専用の扱い)。 */
export type SettingsGroup =
  | "theme"
  | "language"
  | "assistantConnection"
  | "assistantPrompt"
  | "assistantAutoDelete"
  | "translate"
  | "convert";

export interface Settings {
  comfyuiUrl: string;
  /** 画像の保存先の個別設定。null = 全体設定を使う */
  imagesDir: string | null;
  /** 画像の保存先の全体設定。null = アプリのデータ内 */
  imagesRoot: string | null;
  theme: Theme;
  /** null = OS のロケールに従う */
  language: Language | null;
  /** 生成フォームの下書き。null = 組み込みの既定値。 */
  generationDefaults: GenerationParams | null;
  /** 画面の状態(パネルの開閉・幅・密度。docs/29 §6)。null = 既定。 */
  uiState: Record<string, unknown> | null;
}

/** 未指定のキーは変更されない。`imagesDir` / `language` に null を渡すと既定に戻る。 */
export type SettingsPatch = Partial<Omit<Settings, "imagesDir" | "imagesRoot">> & {
  /** テーマ・言語の書き込み先(省略時は個別設定)。 */
  scope?: SettingsScope;
};

export const defaultSettings: Settings = {
  comfyuiUrl: "http://127.0.0.1:8188",
  imagesDir: null,
  imagesRoot: null,
  theme: "dark",
  language: null,
  generationDefaults: null,
  uiState: null,
};

/** `scope = "global"` は全体設定の値(ログイン前でも読める)。省略時は実際に使う値。 */
export const settingsGet = (scope?: SettingsScope) =>
  invoke<Settings>("settings_get", { scope: scope ?? null });

/** 個別設定にしている組。 */
export const settingsScopeGet = () => invoke<SettingsGroup[]>("settings_scope_get");

export const settingsScopeSet = (group: SettingsGroup, overridden: boolean) =>
  invoke<void>("settings_scope_set", { group, overridden });

export const settingsScopeResetGlobal = (group: SettingsGroup) =>
  invoke<void>("settings_scope_reset_global", { group });

export const settingsUpdate = (patch: SettingsPatch) =>
  invoke<Settings>("settings_update", { patch });
