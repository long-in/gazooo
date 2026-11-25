import type { GenerationParams, Img2imgParams } from "@/lib/tauri/jobs";

import {
  defaultControlnet,
  defaultControlnetUnit,
  pickControlnetModel,
} from "./defaults";
import { defaultImg2img } from "./img2img-defaults";
import { sizeForImage } from "./inpaint";

export { defaultImg2img };

/** 強さのボタン(弱・中・強。docs/19 §3)。 */
export const DENOISE_LEVELS = { weak: 0.45, medium: 0.6, strong: 0.75 } as const;
export type DenoiseLevel = keyof typeof DENOISE_LEVELS;

/** 数値に対応するボタン(一致しなければ `null`)。 */
export function levelOf(denoise: number): DenoiseLevel | null {
  const hit = (Object.keys(DENOISE_LEVELS) as DenoiseLevel[]).find((k) => Math.abs(DENOISE_LEVELS[k] - denoise) < 1e-6);
  return hit ?? null;
}

/** 画像から生成を実行できる状態か(元画像がある)。 */
export function img2imgReady(i2i: Img2imgParams | undefined): boolean {
  return i2i?.enabled !== true || i2i.image !== "";
}

/** 構図の固定(ControlNet の画像が元画像と同じで、有効)になっているか。 */
export function isCompositionLocked(params: GenerationParams): boolean {
  const image = params.img2img?.image ?? "";
  const cn = params.controlnet;
  return image !== "" && cn?.enabled === true && cn.units[0]?.image === image;
}

/** 構図の固定の既定(docs/19 §4): 輪郭(Canny)・強さ 0.8・終了 0.8。 */
const LOCK_UNIT = { preprocessor: "canny", strength: 0.8, startPercent: 0, endPercent: 0.8 } as const;

/** 構図の固定を入れる/切る。入れるときは元画像を ControlNet の参照画像にし、モデルは候補から選ぶ。 */
export function withCompositionLock(params: GenerationParams, on: boolean, controlnets: string[]): GenerationParams {
  const cn = params.controlnet ?? defaultControlnet();
  if (!on) return { ...params, controlnet: { ...cn, enabled: false } };
  const base = { ...(cn.units[0] ?? defaultControlnetUnit()), ...LOCK_UNIT, image: params.img2img?.image ?? "" };
  const unit = { ...base, model: pickControlnetModel(controlnets, base.preprocessor, base.model) };
  return { ...params, controlnet: { enabled: true, units: [unit] } };
}

/** 元画像を設定する。生成サイズを画像に合わせ、構図の固定中なら ControlNet の画像も揃える。 */
export function withImg2imgImage(params: GenerationParams, path: string, width: number, height: number): GenerationParams {
  const size = sizeForImage(width, height);
  const locked = isCompositionLocked(params);
  const next: GenerationParams = {
    ...params,
    width: size.width,
    height: size.height,
    img2img: { ...(params.img2img ?? defaultImg2img()), image: path },
  };
  if (!locked || !next.controlnet) return next;
  return { ...next, controlnet: { ...next.controlnet, units: [{ ...next.controlnet.units[0], image: path }] } };
}

/** 画像から生成を入れる/切る。入れるときは、同時に使えない Inpaint を切る。 */
export function withImg2imgEnabled(params: GenerationParams, enabled: boolean): GenerationParams {
  return {
    ...params,
    img2img: { ...(params.img2img ?? defaultImg2img()), enabled },
    ...(enabled && params.inpaint && { inpaint: { ...params.inpaint, enabled: false } }),
  };
}
