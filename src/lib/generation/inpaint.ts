import type { InpaintParams } from "@/lib/tauri/jobs";

/** Inpaint の既定値(docs/13 §3.2)。無効の状態で始める。 */
export function defaultInpaint(): InpaintParams {
  return { enabled: false, image: "", mask: "", denoise: 0.7, growMask: 8, feather: 8, compositeBack: true };
}

const MIN_SIDE = 256;
const MAX_SIDE = 2048;

/**
 * 参照画像の大きさから生成サイズを決める。比率を保ったまま、長辺が 2048 を超えるときだけ縮小し、
 * 64 の倍数(256〜2048)に丸める。丸めで元の大きさと変わる場合は `exact: false`。
 */
export function sizeForImage(width: number, height: number): { width: number; height: number; exact: boolean } {
  const scale = Math.min(1, MAX_SIDE / Math.max(width, height));
  const snap = (v: number) => Math.min(MAX_SIDE, Math.max(MIN_SIDE, Math.round((v * scale) / 64) * 64));
  const w = snap(width);
  const h = snap(height);
  return { width: w, height: h, exact: w === width && h === height };
}

/** RGBA のマスク表示用データ(塗った所 = 不透明)の、塗った/塗っていないを入れ替える。赤で塗り直す。 */
export function invertPaint(data: Uint8ClampedArray): void {
  for (let i = 0; i < data.length; i += 4) {
    const painted = data[i + 3] > 127;
    data[i] = 255;
    data[i + 1] = 0;
    data[i + 2] = 0;
    data[i + 3] = painted ? 0 : 255;
  }
}

/** 1 画素でも塗られているか。 */
export function hasPaint(data: Uint8ClampedArray): boolean {
  for (let i = 3; i < data.length; i += 4) if (data[i] > 127) return true;
  return false;
}

/** 赤で塗った表示データ(黒背景に合成済み)を、R チャンネルを値とするグレースケール RGBA にそろえる。 */
export function toGrayMask(data: Uint8ClampedArray): void {
  for (let i = 0; i < data.length; i += 4) {
    const v = data[i];
    data[i] = v;
    data[i + 1] = v;
    data[i + 2] = v;
    data[i + 3] = 255;
  }
}

/** Inpaint を実行できる状態か(画像とマスクがそろっている)。 */
export function inpaintReady(ip: InpaintParams | undefined): boolean {
  return ip?.enabled !== true || (ip.image !== "" && ip.mask !== "");
}
