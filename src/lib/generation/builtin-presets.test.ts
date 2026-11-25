import { describe, expect, it } from "vitest";

import { applyBuiltinPreset, BUILTIN_PRESETS, isBuiltinPresetId } from "./builtin-presets";
import { defaultParams } from "./defaults";

describe("組み込みプリセット(docs/16 §5)", () => {
  it("アニメ向けの 2 件があり、ID が重複しない", () => {
    expect(BUILTIN_PRESETS.map((p) => p.id)).toEqual(["builtin-anime-sdxl-portrait", "builtin-anime-sdxl-square"]);
    expect(isBuiltinPresetId("builtin-anime-sdxl-square")).toBe(true);
    expect(isBuiltinPresetId("something")).toBe(false);
  });

  it("サイズは 64 の倍数で、縦長は 832×1216・正方形は 1024×1024", () => {
    const [portrait, square] = BUILTIN_PRESETS;
    expect([portrait.settings.width, portrait.settings.height]).toEqual([832, 1216]);
    expect([square.settings.width, square.settings.height]).toEqual([1024, 1024]);
    for (const p of BUILTIN_PRESETS) {
      expect(p.settings.width % 64).toBe(0);
      expect(p.settings.height % 64).toBe(0);
      expect(p.settings.negativePrompt).toContain("lowres");
    }
  });

  it("適用しても、プロンプト・シード・モデルの選択・拡張機能は変わらない(SDXL のとき)", () => {
    const current = {
      ...defaultParams("sdxl"),
      prompt: "1girl, solo",
      seed: 42,
      seedRandomize: false,
      model: { kind: "sdxl" as const, name: "NoobAI-XL-v1.1.safetensors", clip: [], vae: null },
      faceDetailer: { enabled: true, detector: "face.pt", denoise: 0.4, prompt: "", confidence: 0.5 },
    };
    const next = applyBuiltinPreset(current, BUILTIN_PRESETS[0]);
    expect(next.prompt).toBe("1girl, solo");
    expect(next.seed).toBe(42);
    expect(next.model.name).toBe("NoobAI-XL-v1.1.safetensors");
    expect(next.faceDetailer?.enabled).toBe(true);
    expect([next.width, next.height, next.steps, next.cfg]).toEqual([832, 1216, 28, 5]);
    expect([next.sampler, next.scheduler]).toEqual(["euler_ancestral", "normal"]);
  });

  it("SD1.5 や Flux を選んでいるときは SDXL に切り替え、モデル名は空にする", () => {
    for (const kind of ["sd15", "flux_gguf"] as const) {
      const current = { ...defaultParams(kind), prompt: "x" };
      current.model = { ...current.model, name: "something.safetensors", clip: ["a", "b"], vae: "ae.safetensors" };
      const next = applyBuiltinPreset(current, BUILTIN_PRESETS[1]);
      expect(next.model.kind).toBe("sdxl");
      expect(next.model.name).toBe("");
      expect(next.model.clip).toEqual([]);
      expect(next.model.vae).toBeNull();
      expect(next.prompt).toBe("x");
    }
  });
});
