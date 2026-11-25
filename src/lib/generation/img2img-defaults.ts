import type { Img2imgParams } from "@/lib/tauri/jobs";

/** 画像から生成(img2img。docs/19 §4)の既定値。無効の状態で始める。 */
export function defaultImg2img(): Img2imgParams {
  return { enabled: false, image: "", denoise: 0.6 };
}
