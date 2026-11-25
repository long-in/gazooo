import type { GenerationParams, ReferenceParams } from "@/lib/tauri/jobs";

import { sizeForImage } from "./inpaint";

/** 参照画像の枚数の上限(docs/24 §4.1。Rust の `MAX_REFERENCE_IMAGES` と同じ)。 */
export const MAX_REFERENCE_IMAGES = 4;

/** 参照画像の大きさ(総画素の基準。docs/24 §3・Q-120。Rust の `REFERENCE_RESOLUTIONS` と同じ)。 */
export const REFERENCE_RESOLUTIONS = { small: 768, standard: 1024, large: 1536 } as const;
export type ReferenceSize = keyof typeof REFERENCE_RESOLUTIONS;

/** 参照画像つき生成の既定値(docs/24 §4.1)。無効の状態で始める。 */
export function defaultReference(): ReferenceParams {
  return { enabled: false, images: [], resolution: REFERENCE_RESOLUTIONS.standard };
}

/** 参照画像つき生成に対応するモデル種別(Qwen-Image 2.1 のみ。docs/24 Q-116)。 */
export const supportsReference = (kind: GenerationParams["model"]["kind"]) => kind === "qwen_image";

/** 数値に対応する大きさのボタン(一致しなければ `null`)。 */
export function sizeOf(resolution: number): ReferenceSize | null {
  const hit = (Object.keys(REFERENCE_RESOLUTIONS) as ReferenceSize[]).find((k) => REFERENCE_RESOLUTIONS[k] === resolution);
  return hit ?? null;
}

/** 参照画像つき生成を実行できる状態か(有効なら 1 枚以上ある)。 */
export function referenceReady(r: ReferenceParams | undefined): boolean {
  return r?.enabled !== true || r.images.length > 0;
}

/**
 * 出力サイズの目安(docs/24 §3)。`TextEncodeQwenImage21` は、1 枚目の参照画像を総画素が
 * およそ resolution × resolution になるよう縦横比を保って縮小し、32 の倍数に丸める。
 */
export function estimateOutputSize(width: number, height: number, resolution: number): { width: number; height: number } {
  const scale = Math.sqrt((resolution * resolution) / (width * height));
  const snap = (v: number) => Math.max(32, Math.round((v * scale) / 32) * 32);
  return { width: snap(width), height: snap(height) };
}

/** 参照画像つき生成を入れる/切る。入れるときは、顔の入れ替え(排他。docs/42 Q-335)を切る。 */
export function withReferenceEnabled(params: GenerationParams, enabled: boolean): GenerationParams {
  return {
    ...params,
    reference: { ...(params.reference ?? defaultReference()), enabled },
    ...(enabled && params.faceSwap && { faceSwap: { ...params.faceSwap, enabled: false } }),
  };
}

/**
 * 参照画像を末尾に足す(上限を超えるときは何もしない)。1 枚目を足したときは、フォームの生成サイズを
 * その画像に合わせる(Rust の検証が通る値にするため。実際の出力サイズは 1 枚目から決まる)。
 */
export function withReferenceImageAdded(
  params: GenerationParams,
  path: string,
  width: number,
  height: number,
): GenerationParams {
  const current = params.reference ?? defaultReference();
  if (current.images.length >= MAX_REFERENCE_IMAGES || current.images.includes(path)) return params;
  const size = current.images.length === 0 ? sizeForImage(width, height) : null;
  return {
    ...params,
    ...(size && { width: size.width, height: size.height }),
    reference: { ...current, images: [...current.images, path] },
  };
}

/** 参照画像を外す。 */
export function withReferenceImageRemoved(params: GenerationParams, index: number): GenerationParams {
  const current = params.reference ?? defaultReference();
  if (index < 0 || index >= current.images.length) return params;
  return { ...params, reference: { ...current, images: current.images.filter((_, i) => i !== index) } };
}

/** 参照画像を 1 つ前(-1)/後ろ(+1)へ動かす。範囲外へは動かさない。 */
export function withReferenceImageMoved(params: GenerationParams, index: number, delta: -1 | 1): GenerationParams {
  const current = params.reference ?? defaultReference();
  const to = index + delta;
  if (index < 0 || index >= current.images.length || to < 0 || to >= current.images.length) return params;
  const images = [...current.images];
  [images[index], images[to]] = [images[to], images[index]];
  return { ...params, reference: { ...current, images } };
}

/** 参照画像の大きさを変える。 */
export function withReferenceResolution(params: GenerationParams, resolution: number): GenerationParams {
  return { ...params, reference: { ...(params.reference ?? defaultReference()), resolution } };
}

/** `<imageN>` の目印(docs/24 §2)。 */
export const imageTag = (n: number) => `<image${n}>`;

/**
 * プロンプトのカーソル位置(選択範囲は置き換える)へ `<imageN>` を入れる。
 * 新しいプロンプトと、入れたあとのカーソル位置を返す。位置が無ければ末尾に足す。
 */
export function insertImageTag(
  prompt: string,
  n: number,
  selection?: { start: number; end: number } | null,
): { prompt: string; caret: number } {
  const tag = imageTag(n);
  const start = selection ? Math.min(Math.max(selection.start, 0), prompt.length) : prompt.length;
  const end = selection ? Math.min(Math.max(selection.end, start), prompt.length) : prompt.length;
  return { prompt: prompt.slice(0, start) + tag + prompt.slice(end), caret: start + tag.length };
}
