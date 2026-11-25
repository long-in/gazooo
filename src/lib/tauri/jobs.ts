import { invoke } from "./invoke";

export type ModelKind = "sd15" | "sdxl" | "flux" | "flux_gguf" | "anima" | "qwen_image" | "krea2";

export interface ModelSpec {
  kind: ModelKind;
  name: string;
  /** Flux 系: [T5, CLIP-L] の順。SD 系は空。 */
  clip: string[];
  /** null = チェックポイント内蔵(SD 系のみ)。Flux 系は必須。 */
  vae: string | null;
}

/** 顔補正(docs/10-assist-features.md §4)。SD 1.5 / SDXL のみ。 */
export interface FaceDetailerParams {
  enabled: boolean;
  /** 検出モデル("bbox/face_yolov8m.pt" など)。 */
  detector: string;
  /** 補正の強さ(0.05〜1.0)。 */
  denoise: number;
  /** 補正用プロンプト。空なら本体のプロンプトを使う。 */
  prompt: string;
  /** 検出しきい値(0.1〜1.0)。 */
  confidence: number;
}

export type Preprocessor = "canny" | "depth" | "openpose" | "none";

/** ControlNet の 1 ユニット(docs/10-assist-features.md §5)。 */
export interface ControlnetUnit {
  /** 参照画像。画像保存先からの相対パス("inputs/<uuid>.png")。 */
  image: string;
  model: string;
  preprocessor: Preprocessor;
  /** 0〜2 */
  strength: number;
  /** 0〜1(終了より小さい) */
  startPercent: number;
  endPercent: number;
}

/** SD 1.5 / SDXL のみ。v0.2 は 1 ユニットまで。 */
export interface ControlnetParams {
  enabled: boolean;
  units: ControlnetUnit[];
}

/** Inpaint(docs/13 §3.2)。SD 1.5 / SDXL のみ。顔補正とは同時に使えない。 */
export interface InpaintParams {
  enabled: boolean;
  /** 参照画像("inputs/<uuid>.png")。 */
  image: string;
  /** マスク("inputs/<uuid>.png"。白 = 描き直す範囲)。 */
  mask: string;
  /** 0.05〜1 */
  denoise: number;
  /** 0〜64 px */
  growMask: number;
  /** 0〜31 px */
  feather: number;
  compositeBack: boolean;
}

/** 画像から生成(img2img。docs/19 §4)。SD 1.5 / SDXL のみ。Inpaint とは同時に使えない。 */
export interface Img2imgParams {
  enabled: boolean;
  /** 元画像("inputs/<uuid>.png")。 */
  image: string;
  /** 0.05〜1 */
  denoise: number;
}

/** 参照画像つき生成(Qwen-Image 2.1 の画像編集。docs/24 §4.1)。Qwen-Image 2.1 のみ。 */
export interface ReferenceParams {
  enabled: boolean;
  /** 参照画像("inputs/<uuid>.png")。1〜4 枚。1 枚目が編集される画像で、出力サイズも 1 枚目で決まる。 */
  images: string[];
  /** 参照画像の総画素の基準。768 / 1024 / 1536。 */
  resolution: number;
}

/** 顔の入れ替えで合成する範囲(docs/42 Q-332)。 */
export type FaceSwapRegion = "head" | "face";

/** 顔の入れ替え(docs/42 §4.1)。Qwen-Image 2.1 のみ。1 ジョブにつき対象 1 枚(複数の対象は 1 枚ずつ投入する)。 */
export interface FaceSwapParams {
  enabled: boolean;
  /** 対象画像("inputs/<uuid>.png")。顔以外はこの画像のまま保つ。 */
  target: string;
  /** 入れ替える顔の画像("inputs/<uuid>.png")。 */
  face: string;
  /** 顔以外を元のまま保つ(後段合成)。 */
  keepOutside: boolean;
  region: FaceSwapRegion;
  /** 参照画像の総画素の基準。768 / 1024 / 1536。 */
  resolution: number;
  /** 後段合成の顔検出モデル("bbox/…")。 */
  detector: string;
}

/** docs/03-data-model.md の params_json と同一。 */
export interface GenerationParams {
  schemaVersion: number;
  model: ModelSpec;
  prompt: string;
  negativePrompt: string;
  width: number;
  height: number;
  steps: number;
  cfg: number;
  guidance: number;
  sampler: string;
  scheduler: string;
  seed: number;
  seedRandomize: boolean;
  batchSize: number;
  /** 無ければ顔補正は無効。 */
  faceDetailer?: FaceDetailerParams;
  /** 無ければ ControlNet は無効。 */
  controlnet?: ControlnetParams;
  /** 無ければ Inpaint は無効。 */
  inpaint?: InpaintParams;
  /** 無ければ画像から生成は無効。 */
  img2img?: Img2imgParams;
  /** 無ければ参照画像つき生成は無効。 */
  reference?: ReferenceParams;
  /** 無ければ顔の入れ替えは無効。 */
  faceSwap?: FaceSwapParams;
}

export type JobStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface Job {
  id: string;
  status: JobStatus;
  position: number;
  params: GenerationParams;
  error?: { code: string; message: string };
  imageIds: string[];
  /** 出力先のプロジェクト(null = 未分類。docs/40)。 */
  projectId: string | null;
  createdAt: number;
  startedAt?: number;
  finishedAt?: number;
}

export interface ImageItem {
  id: string;
  jobId: string;
  width: number;
  height: number;
  seed: number;
  isFavorite: boolean;
  createdAt: number;
  /** 所属プロジェクト(null = 未分類。docs/40)。 */
  projectId: string | null;
  /** 絶対パス。表示には `convertFileSrc` で URL に変換する。 */
  filePath: string;
  thumbPath: string;
}

export interface JobProgress {
  jobId: string;
  step: number;
  totalSteps: number;
  node?: string;
  /** 顔補正を使うジョブで、どの段階の進捗か。 */
  phase?: "generate" | "face_detail";
}

export interface Page<T> {
  items: T[];
  nextCursor?: string;
}

export const MAX_ENQUEUE_COUNT = 20;

/** `projectId`: 生成画像の入れ先(null / 省略 = 未分類。docs/40 §4.2)。 */
export const jobEnqueue = (params: GenerationParams, count = 1, projectId: string | null = null) =>
  invoke<Job[]>("job_enqueue", { params, count, projectId });

export const jobList = (opts: { statuses?: JobStatus[]; cursor?: string; limit?: number } = {}) =>
  invoke<Page<Job>>("job_list", opts);

export const jobCancel = (id: string) => invoke<void>("job_cancel", { id });

export const jobReorder = (id: string, newPosition: number) =>
  invoke<void>("job_reorder", { id, newPosition });

export const jobRetry = (id: string) => invoke<Job>("job_retry", { id });

export const jobClearFinished = () => invoke<number>("job_clear_finished");

/** 終了済みジョブを履歴から外す(docs/20 §3.4)。 */
export const jobRemove = (ids: string[]) => invoke<number>("job_remove", { ids });

export const queuePause = () => invoke<void>("queue_pause");
export const queueResume = () => invoke<void>("queue_resume");
export const queueState = () => invoke<{ paused: boolean }>("queue_state");
