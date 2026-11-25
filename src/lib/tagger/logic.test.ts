import { describe, expect, it } from "vitest";

import {
  DEFAULT_TAGGER_MODEL,
  DEFAULT_THRESHOLD,
  applyTags,
  clampThreshold,
  pickTaggerModel,
  prefersNaturalLanguage,
} from "./logic";

describe("applyTags", () => {
  it("置き換えは、現在のプロンプトを捨てて結果だけにする", () => {
    expect(applyTags("old, prompt", " 1girl, solo ", "replace")).toBe("1girl, solo");
  });

  it("末尾に追加は、カンマで区切って足す(重複した区切りを作らない)", () => {
    expect(applyTags("masterpiece, ", "1girl, solo", "append")).toBe("masterpiece, 1girl, solo");
    expect(applyTags("masterpiece,", "1girl", "append")).toBe("masterpiece, 1girl");
  });

  it("現在が空なら、追加でもそのまま入る。結果が空なら現在を保つ", () => {
    expect(applyTags("", "1girl", "append")).toBe("1girl");
    expect(applyTags("masterpiece", "", "append")).toBe("masterpiece");
  });
});

describe("prefersNaturalLanguage", () => {
  it("文章向きのモデルだけ true", () => {
    for (const k of ["flux", "flux_gguf", "qwen_image", "krea2"] as const) {
      expect(prefersNaturalLanguage(k)).toBe(true);
    }
    for (const k of ["sd15", "sdxl", "anima"] as const) {
      expect(prefersNaturalLanguage(k)).toBe(false);
    }
  });
});

describe("pickTaggerModel", () => {
  it("現在の選択が候補にあれば維持し、なければ既定 → 先頭の順", () => {
    const list = ["wd-vit-tagger-v3", DEFAULT_TAGGER_MODEL];
    expect(pickTaggerModel(list, "wd-vit-tagger-v3")).toBe("wd-vit-tagger-v3");
    expect(pickTaggerModel(list, "gone")).toBe(DEFAULT_TAGGER_MODEL);
    expect(pickTaggerModel(["a", "b"])).toBe("a");
    expect(pickTaggerModel([])).toBeNull();
  });
});

describe("clampThreshold", () => {
  it("範囲内に丸め、数値でなければ既定値にする", () => {
    expect(clampThreshold(0, DEFAULT_THRESHOLD)).toBe(0.05);
    expect(clampThreshold(2, DEFAULT_THRESHOLD)).toBe(1);
    expect(clampThreshold(0.5, DEFAULT_THRESHOLD)).toBe(0.5);
    expect(clampThreshold(Number.NaN, DEFAULT_THRESHOLD)).toBe(DEFAULT_THRESHOLD);
  });
});
