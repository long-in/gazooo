import { describe, expect, it } from "vitest";

import { defaultParams, restoreParams, switchKind } from "./defaults";
import {
  defaultReference,
  estimateOutputSize,
  imageTag,
  insertImageTag,
  MAX_REFERENCE_IMAGES,
  referenceReady,
  sizeOf,
  supportsReference,
  withReferenceEnabled,
  withReferenceImageAdded,
  withReferenceImageMoved,
  withReferenceImageRemoved,
  withReferenceResolution,
} from "./reference";

const qwen = () => defaultParams("qwen_image");
const add = (p = qwen(), path = "inputs/a.png", w = 896, h = 1184) => withReferenceImageAdded(p, path, w, h);

describe("reference defaults and support", () => {
  it("starts disabled with the standard size and supports only Qwen-Image 2.1", () => {
    expect(defaultReference()).toEqual({ enabled: false, images: [], resolution: 1024 });
    expect(supportsReference("qwen_image")).toBe(true);
    for (const k of ["sd15", "sdxl", "flux", "flux_gguf", "anima", "krea2"] as const) {
      expect(supportsReference(k)).toBe(false);
    }
  });

  it("maps resolutions to the size buttons", () => {
    expect(sizeOf(768)).toBe("small");
    expect(sizeOf(1024)).toBe("standard");
    expect(sizeOf(1536)).toBe("large");
    expect(sizeOf(1000)).toBeNull();
  });

  it("is ready unless enabled with no images", () => {
    expect(referenceReady(undefined)).toBe(true);
    expect(referenceReady(defaultReference())).toBe(true);
    expect(referenceReady({ ...defaultReference(), enabled: true })).toBe(false);
    expect(referenceReady({ ...defaultReference(), enabled: true, images: ["inputs/a.png"] })).toBe(true);
  });
});

describe("reference images", () => {
  it("adds up to the limit, skips duplicates, and sizes the form from the first image only", () => {
    const one = add();
    expect(one.reference?.images).toEqual(["inputs/a.png"]);
    // 896×1184 → 64 の倍数に丸めた生成サイズ(Rust の検証が通る値)
    expect([one.width, one.height]).toEqual([896, 1216]);

    const two = add(one, "inputs/b.png", 2000, 500);
    expect(two.reference?.images).toEqual(["inputs/a.png", "inputs/b.png"]);
    expect([two.width, two.height]).toEqual([896, 1216]); // 2 枚目では変えない
    expect(add(two, "inputs/b.png")).toBe(two); // 重複

    let full = qwen();
    for (let i = 0; i < MAX_REFERENCE_IMAGES + 2; i++) full = add(full, `inputs/${i}.png`);
    expect(full.reference?.images).toHaveLength(MAX_REFERENCE_IMAGES);
  });

  it("removes and moves images, ignoring out-of-range requests", () => {
    let p = add(add(add(qwen(), "inputs/a.png"), "inputs/b.png"), "inputs/c.png");
    p = withReferenceImageMoved(p, 2, -1);
    expect(p.reference?.images).toEqual(["inputs/a.png", "inputs/c.png", "inputs/b.png"]);
    expect(withReferenceImageMoved(p, 0, -1)).toBe(p);
    expect(withReferenceImageMoved(p, 2, 1)).toBe(p);
    p = withReferenceImageRemoved(p, 0);
    expect(p.reference?.images).toEqual(["inputs/c.png", "inputs/b.png"]);
    expect(withReferenceImageRemoved(p, 5)).toBe(p);
  });

  it("toggles enabled and changes the resolution without losing images", () => {
    let p = add();
    p = withReferenceEnabled(p, true);
    p = withReferenceResolution(p, 1536);
    expect(p.reference).toEqual({ enabled: true, images: ["inputs/a.png"], resolution: 1536 });
  });
});

describe("estimateOutputSize", () => {
  it("keeps the aspect ratio at about resolution² pixels, in multiples of 32", () => {
    expect(estimateOutputSize(1024, 1024, 1024)).toEqual({ width: 1024, height: 1024 });
    expect(estimateOutputSize(3000, 4000, 1024)).toEqual({ width: 896, height: 1184 });
    expect(estimateOutputSize(1024, 1024, 768)).toEqual({ width: 768, height: 768 });
    const big = estimateOutputSize(1600, 900, 1536);
    expect(big.width % 32).toBe(0);
    expect(big.height % 32).toBe(0);
    expect(big.width).toBeGreaterThan(big.height);
  });
});

describe("insertImageTag", () => {
  it("inserts at the caret, replaces a selection, and appends without a selection", () => {
    expect(imageTag(2)).toBe("<image2>");
    expect(insertImageTag("ab", 1, { start: 1, end: 1 })).toEqual({ prompt: "a<image1>b", caret: 9 });
    expect(insertImageTag("abcd", 2, { start: 1, end: 3 })).toEqual({ prompt: "a<image2>d", caret: 9 });
    expect(insertImageTag("ab", 1)).toEqual({ prompt: "ab<image1>", caret: 10 });
    expect(insertImageTag("ab", 1, { start: 99, end: 99 }).prompt).toBe("ab<image1>");
  });
});

describe("kind switching and restore", () => {
  it("keeps the images but disables the feature when leaving Qwen-Image 2.1", () => {
    const p = withReferenceEnabled(add(), true);
    const away = switchKind(p, "sdxl");
    expect(away.reference).toEqual({ enabled: false, images: ["inputs/a.png"], resolution: 1024 });
    const back = switchKind(withReferenceEnabled(away, true), "qwen_image");
    expect(back.reference?.enabled).toBe(true);
  });

  it("restores a saved draft, filling missing fields", () => {
    const restored = restoreParams({ ...qwen(), reference: { enabled: true } as never });
    expect(restored.reference).toEqual({ enabled: true, images: [], resolution: 1024 });
    expect(restoreParams(qwen()).reference).toBeUndefined();
  });
});
