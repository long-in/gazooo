import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

import { invoke } from "./invoke";

/** モデル取得(docs/12-model-downloader.md、docs/04-ipc-api.md「モデル取得」)。型は Rust 側が正。 */

export interface Localized {
  ja: string;
  en: string;
}

export type ModelCategory = "checkpoint" | "controlnet" | "detector" | "flux" | "anima" | "qwen_image" | "krea2";

/** 生成画面で選べる拡散モデル本体(`unet/` に置くもの)の種別(docs/17 §5)。 */
export type FamilyKind = "flux_gguf" | "anima" | "qwen_image" | "krea2";

/** 「まとめて取得」で一緒に取得する、必要なファイル(別ファミリーの variant)。 */
export interface ModelRequirement {
  familyId: string;
  variantId: string;
}
export type BaseModel = "sd15" | "sdxl";

/** 取得ページの用途フィルタ用のタグ(docs/16 §3.1)。 */
export type ModelTag = "anime";

export type ModelStyle = "anime" | "realistic" | "illustration" | "general";
/** 規制の強さ(docs/18 §10)。配布元の説明で確認できたものだけ段階を付け、確認できなければ `unknown`。 */
export type ModerationLevel = "strict" | "moderate" | "loose" | "unknown";

/** モデルの説明(docs/18 §3)。生成画面の説明パネル・取得ページ・AI のモデルガイドの単一の情報源。 */
export interface ModelProfile {
  summary: Localized;
  style: ModelStyle[];
  bestFor: { ja: string[]; en: string[] };
  weakAt: { ja: string[]; en: string[] };
  promptStyle: { kind: "tags" | "natural" | "both"; language?: "en" | "multilingual"; hint: Localized };
  recommended: {
    steps: number;
    cfg: number;
    sampler?: string;
    scheduler?: string;
    size: { width: number; height: number };
  };
  /** `vramGb` は必要な VRAM の最小の目安(GB)。 */
  resources: { vramGb: number; speed: "fast" | "medium" | "slow" };
  moderation: { level: ModerationLevel; note: Localized };
  sourceNote: Localized;
}

export interface ModelVariant {
  id: string;
  label: Localized;
  baseModel?: BaseModel;
  precision?: "fp8" | "fp16" | "fp32";
  recommended: boolean;
  source: { url: string; repo: string };
  sizeBytes: number;
  sha256: string;
  destDir: "checkpoints" | "controlnet" | "ultralytics/bbox" | "unet" | "clip" | "vae" | "loras";
  fileName: string;
  license: { name: string; url: string; commercialUse: boolean | "see-license" };
  requiresConsent: boolean;
  note?: Localized;
}

export interface ModelFamily {
  id: string;
  category: ModelCategory;
  name: Localized;
  description: Localized;
  usedBy: string[];
  /** 用途フィルタ用(docs/16 §3.1)。無ければ空。 */
  tags: ModelTag[];
  /** 拡散モデル本体のファミリーの種別。あれば生成画面で選べるモデルで、`profile` もある。 */
  kind?: FamilyKind;
  /** 一緒に取得が必要なファイル(テキストエンコーダ・VAE)。無ければ空。 */
  requires?: ModelRequirement[];
  /** モデルの説明。生成画面で選べるモデル(チェックポイント・Flux 本体)には必ずある。 */
  profile?: ModelProfile;
  variants: ModelVariant[];
}

export interface ModelCatalog {
  schemaVersion: number;
  families: ModelFamily[];
}

export interface InstalledStatus {
  familyId: string;
  variantId: string;
  installed: boolean;
  /** ファイルサイズがカタログと一致するか(一致しなければ破損・別物の可能性)。 */
  sizeMatches: boolean;
}

export type DirSource = "setting" | "detected" | "unknown";

export interface CatalogView {
  catalog: ModelCatalog;
  installed: InstalledStatus[];
  modelsDir: string | null;
  modelsDirSource: DirSource;
  freeBytes: number | null;
}

export type DownloadStatus = "queued" | "downloading" | "verifying" | "done" | "failed" | "cancelled";

export interface DownloadState {
  downloadId: string;
  familyId: string;
  variantId: string;
  status: DownloadStatus;
  receivedBytes: number;
  totalBytes: number;
  bytesPerSec?: number;
  error?: { code: string; message: string };
}

export const modelsCatalog = () => invoke<CatalogView>("models_catalog");

export const modelsDownloadStart = (familyId: string, variantId: string, consent?: boolean) =>
  invoke<{ downloadId: string }>("models_download_start", { familyId, variantId, consent });

export const modelsDownloadCancel = (downloadId: string, discard?: boolean) =>
  invoke<void>("models_download_cancel", { downloadId, discard });

export const modelsDownloadList = () => invoke<DownloadState[]>("models_download_list");

export const modelsVerify = (familyId: string, variantId: string) =>
  invoke<{ ok: boolean }>("models_verify", { familyId, variantId });

export const modelsSetDir = (path: string | null) =>
  invoke<{ modelsDir: string | null; source: DirSource }>("models_set_dir", { path });

export type ApplyBlock = "remote" | "engine_unknown" | "no_setting" | "foreign";

/** 保存先の ComfyUI への反映状況(docs/08 §6.2)。 */
export interface EngineDirStatus {
  engineModelsDir: string | null;
  applied: boolean;
  /** 保存先が ComfyUI の参照先と異なり、未反映。 */
  mismatch: boolean;
  hasCustomConfig: boolean;
  canApply: boolean;
  reason?: ApplyBlock;
}

export const modelsEngineDirStatus = () => invoke<EngineDirStatus>("models_engine_dir_status");

/** 保存先を ComfyUI に反映する(`clear` で解除)。ComfyUI の再起動は別途必要。 */
export const modelsApplyDirToEngine = (clear?: boolean) =>
  invoke<EngineDirStatus>("models_apply_dir_to_engine", { clear });

/** フォルダ選択ダイアログ。キャンセルなら null。 */
export async function pickModelsDir(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

/** ライセンスなどの外部ページを既定のブラウザで開く(http / https のみ)。 */
export async function openExternal(url: string): Promise<void> {
  if (!/^https:\/\//.test(url)) return;
  await openUrl(url);
}
