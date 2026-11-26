import { invoke } from "./invoke";
import type { SettingsScope } from "./settings";

/** 画像の形式変換(docs/43)。パスは Rust 側が持ち、フロントは `id` だけを扱う。 */

export type InFormat = "webp" | "png" | "jpeg" | "gif" | "bmp" | "tiff" | "heic";
export type OutFormat = "png" | "jpeg" | "webp";
export type PngCompression = "fast" | "default" | "best";

export interface ConvertEntry {
  id: string;
  name: string;
  /** 中身から判定した形式。判定できなければ null。 */
  detectedFormat: InFormat | null;
  width: number;
  height: number;
  bytes: number;
  /** あれば変換の対象外(理由)。 */
  error: string | null;
}

export interface ConvertSettings {
  format: OutFormat;
  quality: number;
  webpLossless: boolean;
  pngCompression: PngCompression;
  /** `#rrggbb` */
  background: string;
  overwrite: boolean;
  /** 実際に使う出力先(未設定なら既定の場所)。 */
  outputDir: string;
}

export type ConvertSettingsPatch = Partial<Omit<ConvertSettings, "outputDir">>;

export interface ConvertRunRequest {
  ids: string[];
  format: OutFormat;
  quality: number;
  webpLossless: boolean;
  pngCompression: PngCompression;
  background: string;
  overwrite: boolean;
}

export interface ConvertItemResult {
  id: string;
  ok: boolean;
  outputName: string | null;
  inBytes: number;
  outBytes: number | null;
  error: string | null;
}

export interface ConvertProgress {
  runId: string;
  done: number;
  total: number;
}

export interface ConvertFinished {
  runId: string;
  ok: number;
  failed: number;
  inBytes: number;
  outBytes: number;
  cancelled: boolean;
  outputDir: string;
}

export const convertSettingsGet = (scope?: SettingsScope) =>
  invoke<ConvertSettings>("convert_settings_get", { scope: scope ?? null });

export const convertSettingsUpdate = (patch: ConvertSettingsPatch, scope?: SettingsScope) =>
  invoke<ConvertSettings>("convert_settings_update", { patch, scope: scope ?? null });

/** 出力先をダイアログで選んで保存する(Rust 側)。キャンセルなら null。 */
export const convertPickOutputDir = (scope?: SettingsScope) =>
  invoke<ConvertSettings | null>("convert_pick_output_dir", { scope: scope ?? null });

export const convertPickFiles = () => invoke<ConvertEntry[]>("convert_pick_files");

export const convertPickFolder = () => invoke<ConvertEntry[]>("convert_pick_folder");

export const convertList = () => invoke<ConvertEntry[]>("convert_list");

export const convertRemove = (ids: string[]) => invoke<void>("convert_remove", { ids });

export const convertClear = () => invoke<void>("convert_clear");

/** 変換ページが開いている間だけ、ドロップを受け付ける。 */
export const convertSetAccepting = (on: boolean) => invoke<void>("convert_set_accepting", { on });

export const convertRun = (request: ConvertRunRequest) =>
  invoke<{ runId: string }>("convert_run", { request });

export const convertCancel = (runId: string) => invoke<void>("convert_cancel", { runId });

export const convertOpenOutputDir = () => invoke<void>("convert_open_output_dir");
