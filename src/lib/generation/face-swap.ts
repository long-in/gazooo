import type { FaceSwapParams, GenerationParams } from "@/lib/tauri/jobs";

import { REFERENCE_RESOLUTIONS } from "./reference";

/** 顔の入れ替えに対応するモデル種別(Qwen-Image 2.1 のみ。docs/42 Q-329)。 */
export const supportsFaceSwap = (kind: GenerationParams["model"]["kind"]) => kind === "qwen_image";

/** 対象画像を一度に取り込む枚数の上限の目安(Rust の `MAX_PICK_MANY` と同じ。docs/42 Q-348)。 */
export const MAX_FACE_SWAP_TARGETS = 50;

/** 後段合成の顔検出モデルの既定(Rust の `FACE_SWAP_DETECTOR` と同じ)。 */
export const FACE_SWAP_DETECTOR = "bbox/face_yolov9c.pt";

/**
 * 既定のプロンプト(docs/42 §4.2)。BFS の作者が配布する文面で、実機で動作を確認した。
 * `<image1>` = 対象、`<image2>` = 顔。
 */
export const FACE_SWAP_PROMPT =
  "head_swap: start with <image1> as the base image, keeping its lighting, environment, and background. " +
  "remove the head from <image1> completely and replace it with the head from <image2>, strictly preserving " +
  "the hair, eye color, nose structure from <image2>. copy the direction of the eye, head rotation, micro " +
  "expressions from <image1>, high quality, sharp details, 4k";

/** ステップ数の選択肢(docs/42 Q-343。実機: 8 ステップ 約 90 秒、25 ステップ 約 290 秒)。 */
export const FACE_SWAP_STEPS = { fast: 8, quality: 25 } as const;
export type FaceSwapQuality = keyof typeof FACE_SWAP_STEPS;

/** 顔画像の短辺がこれ未満だと、似方が弱くなるので警告する(docs/46 §6 D・Q-396)。生成は止めない。 */
export const MIN_FACE_SHORT_SIDE = 512;

/** 顔画像が小さすぎるか。 */
export const isSmallFaceImage = (img: { width: number; height: number }) =>
  Math.min(img.width, img.height) < MIN_FACE_SHORT_SIDE;

/** 1 ジョブの所要秒数の目安(RTX 2060 12GB の実測: 8 ステップ約 90 秒、25 ステップ約 290 秒。docs/42 §9.1)。 */
export const estimateJobSeconds = (steps: number) => Math.round(steps * 11.5);

/** 合計の所要時間の目安(分。切り上げ)。`jobs` はジョブの総数(対象の枚数 × 投入回数)。 */
export const estimateMinutes = (jobs: number, steps: number) => Math.ceil((jobs * estimateJobSeconds(steps)) / 60);

/** 顔の入れ替えの既定値(docs/42 §4.1)。無効の状態で始める。 */
export function defaultFaceSwap(): FaceSwapParams {
  return {
    enabled: false,
    target: "",
    face: "",
    keepOutside: true,
    region: "head",
    resolution: REFERENCE_RESOLUTIONS.standard,
    detector: FACE_SWAP_DETECTOR,
  };
}

/** 顔の入れ替えの設定を部分更新する(未設定なら既定値から作る)。 */
export function withFaceSwap(params: GenerationParams, patch: Partial<FaceSwapParams>): GenerationParams {
  return { ...params, faceSwap: { ...(params.faceSwap ?? defaultFaceSwap()), ...patch } };
}

/** プロンプトが、顔の入れ替えの雛形で始まっているか(有効化のたびに上書きしないための目印)。 */
export const hasFaceSwapPrompt = (prompt: string) => prompt.trimStart().startsWith("head_swap:");

/**
 * 顔の入れ替えを入れる/切る。入れるときは、参照画像つき生成(排他)を切り、
 * プロンプトが雛形でなければ雛形を入れ、ステップ・CFG・枚数を既定にする(docs/42 §3・Q-337・Q-343)。
 * 切るときは設定だけを無効にする(プロンプトは戻さない。UI が「元に戻す」を出す)。
 */
export function withFaceSwapEnabled(params: GenerationParams, enabled: boolean): GenerationParams {
  const next = withFaceSwap(params, { enabled });
  if (!enabled) return next;
  return {
    ...next,
    ...(next.reference && { reference: { ...next.reference, enabled: false } }),
    prompt: hasFaceSwapPrompt(next.prompt) ? next.prompt : FACE_SWAP_PROMPT,
    steps: hasFaceSwapPrompt(params.prompt) ? params.steps : FACE_SWAP_STEPS.fast,
    cfg: 1,
    batchSize: 1,
  };
}

/** ステップ数に対応する品質のボタン(一致しなければ `null`)。 */
export function qualityOf(steps: number): FaceSwapQuality | null {
  return (Object.keys(FACE_SWAP_STEPS) as FaceSwapQuality[]).find((k) => FACE_SWAP_STEPS[k] === steps) ?? null;
}

/** 顔の入れ替えを実行できる状態か(有効なら、顔画像と対象画像が 1 枚以上ある)。 */
export function faceSwapReady(fs: FaceSwapParams | undefined, targets: string[]): boolean {
  return fs?.enabled !== true || (fs.face !== "" && targets.length > 0);
}

/** 対象画像を末尾に足す(重複・上限超過は無視する)。 */
export function addTargets(targets: string[], paths: string[]): string[] {
  const out = [...targets];
  for (const p of paths) {
    if (out.length >= MAX_FACE_SWAP_TARGETS) break;
    if (!out.includes(p)) out.push(p);
  }
  return out;
}

/**
 * 対象画像ごとのジョブのパラメータ(docs/42 §4.3)。各ジョブは対象 1 枚・1 ジョブ 1 枚(`batchSize` 1)。
 * 無効のとき、または対象が無いときは空。`compositeAvailable` が false なら、合成は切って投入する。
 */
export function faceSwapJobs(
  params: GenerationParams,
  targets: string[],
  compositeAvailable = true,
): GenerationParams[] {
  const fs = params.faceSwap;
  if (!fs?.enabled) return [];
  // 後段合成のノードが無い ComfyUI では、合成なしで投入する(あると失敗するため。docs/42 Q-331)
  const keepOutside = fs.keepOutside && compositeAvailable;
  // プロンプトが空なら雛形を詰める(docs/42 §4.2・Q-389)
  const prompt = params.prompt.trim() === "" ? FACE_SWAP_PROMPT : params.prompt;
  return targets.map((target) => ({ ...params, prompt, batchSize: 1, faceSwap: { ...fs, target, keepOutside } }));
}

/**
 * 復元した設定(再現・プリセット・プロジェクト)から、フォームの対象画像の一覧を作る。
 * 保存されるのは 1 ジョブぶん(対象 1 枚)なので、その 1 枚だけになる。
 */
export function targetsFromParams(params: Partial<GenerationParams>): string[] {
  const t = params.faceSwap?.target;
  return t ? [t] : [];
}
