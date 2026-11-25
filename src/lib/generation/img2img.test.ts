import { describe, expect, it } from "vitest";

import { defaultParams, supportsImg2img, switchKind } from "./defaults";
import {
  DENOISE_LEVELS,
  defaultImg2img,
  img2imgReady,
  isCompositionLocked,
  levelOf,
  withCompositionLock,
  withImg2imgEnabled,
  withImg2imgImage,
} from "./img2img";

const IMG = { path: "inputs/src.png", width: 1000, height: 600 };

describe("img2img の基本", () => {
  it("既定は無効・強さ 0.6 で、SD 系だけが対応する", () => {
    expect(defaultImg2img()).toEqual({ enabled: false, image: "", denoise: 0.6 });
    expect(supportsImg2img("sd15") && supportsImg2img("sdxl")).toBe(true);
    for (const k of ["flux", "flux_gguf", "anima", "qwen_image", "krea2"] as const) expect(supportsImg2img(k)).toBe(false);
  });

  it("強さのボタンと数値が対応し、元画像が無ければ実行できない", () => {
    expect(DENOISE_LEVELS).toEqual({ weak: 0.45, medium: 0.6, strong: 0.75 });
    expect(levelOf(0.6)).toBe("medium");
    expect(levelOf(0.5)).toBeNull();
    expect(img2imgReady(undefined)).toBe(true);
    expect(img2imgReady({ enabled: true, image: "", denoise: 0.6 })).toBe(false);
    expect(img2imgReady({ enabled: true, image: "inputs/a.png", denoise: 0.6 })).toBe(true);
    expect(img2imgReady({ enabled: false, image: "", denoise: 0.6 })).toBe(true);
  });

  it("元画像でサイズが決まり(64 の倍数に丸め)、有効にすると Inpaint を切る", () => {
    const base = {
      ...defaultParams("sdxl"),
      inpaint: { enabled: true, image: "inputs/i.png", mask: "inputs/m.png", denoise: 0.7, growMask: 8, feather: 8, compositeBack: true },
    };
    const set = withImg2imgImage(base, IMG.path, IMG.width, IMG.height);
    expect([set.width, set.height]).toEqual([1024, 576]);
    expect(set.img2img?.image).toBe(IMG.path);
    const on = withImg2imgEnabled(set, true);
    expect(on.img2img?.enabled).toBe(true);
    expect(on.inpaint?.enabled).toBe(false);
  });

  it("種別を Flux などへ切り替えると無効になる(画像は残る)", () => {
    const on = withImg2imgEnabled(withImg2imgImage(defaultParams("sdxl"), IMG.path, 512, 512), true);
    const flux = switchKind(on, "flux");
    expect(flux.img2img).toEqual({ enabled: false, image: IMG.path, denoise: 0.6 });
  });
});

describe("構図の固定", () => {
  const withImage = withImg2imgImage(defaultParams("sdxl"), IMG.path, 1024, 1024);

  it("入れると ControlNet を元画像・Canny・強さ 0.8・終了 0.8 で設定する", () => {
    const p = withCompositionLock(withImage, true, ["a-canny.safetensors", "b-depth.safetensors"]);
    expect(p.controlnet?.enabled).toBe(true);
    expect(p.controlnet?.units[0]).toMatchObject({
      image: IMG.path,
      preprocessor: "canny",
      strength: 0.8,
      startPercent: 0,
      endPercent: 0.8,
      model: "a-canny.safetensors",
    });
    expect(isCompositionLocked(p)).toBe(true);
  });

  it("切ると ControlNet を無効にする。ControlNet の画像が別なら、固定とはみなさない", () => {
    const locked = withCompositionLock(withImage, true, ["c.safetensors"]);
    expect(isCompositionLocked(withCompositionLock(locked, false, []))).toBe(false);
    const other = { ...locked, controlnet: { enabled: true, units: [{ ...locked.controlnet!.units[0], image: "inputs/other.png" }] } };
    expect(isCompositionLocked(other)).toBe(false);
  });

  it("固定中に元画像を変えると、ControlNet の画像も揃う", () => {
    const locked = withCompositionLock(withImage, true, ["c.safetensors"]);
    const changed = withImg2imgImage(locked, "inputs/new.png", 512, 512);
    expect(changed.controlnet?.units[0].image).toBe("inputs/new.png");
    expect(isCompositionLocked(changed)).toBe(true);
  });
});
