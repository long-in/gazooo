import { describe, expect, it } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";
import type { EngineModels } from "@/lib/tauri/engine";
import type { ModelCatalog, ModelFamily, ModelProfile, ModelVariant } from "@/lib/tauri/models";

import {
  candidateBadges,
  familyByFileName,
  familyKinds,
  featuresFor,
  installedFamilyIds,
  modelContext,
  profileFor,
  recommendedPatch,
  switchTarget,
} from "./profile";

const L = { ja: "x", en: "x" };
const profile = (over: Partial<ModelProfile> = {}): ModelProfile => ({
  summary: L,
  style: ["general"],
  bestFor: { ja: ["a"], en: ["a"] },
  weakAt: { ja: ["b"], en: ["b"] },
  promptStyle: { kind: "both", hint: L },
  recommended: { steps: 25, cfg: 6.5, sampler: "dpmpp_2m", scheduler: "karras", size: { width: 1024, height: 1024 } },
  resources: { vramGb: 8, speed: "medium" },
  moderation: { level: "unknown", note: L },
  sourceNote: L,
  ...over,
});
const variant = (id: string, fileName: string, over: Partial<ModelVariant> = {}): ModelVariant => ({
  id,
  label: L,
  recommended: false,
  source: { url: "https://huggingface.co/a/b/resolve/c/m.safetensors", repo: "a/b" },
  sizeBytes: 1,
  sha256: "0".repeat(64),
  destDir: "checkpoints",
  fileName,
  license: { name: "l", url: "https://x", commercialUse: true },
  requiresConsent: false,
  ...over,
});
const family = (id: string, over: Partial<ModelFamily>): ModelFamily => ({
  id,
  category: "checkpoint",
  name: L,
  description: L,
  usedBy: [],
  tags: [],
  variants: [],
  ...over,
});

const sd15 = family("checkpoint-sd15", {
  profile: profile(),
  variants: [variant("a", "v1-5-pruned-emaonly.safetensors", { baseModel: "sd15", recommended: true })],
});
const sdxl = family("checkpoint-sdxl", {
  profile: profile(),
  variants: [variant("a", "sd_xl_base_1.0.safetensors", { baseModel: "sdxl", recommended: true })],
});
const noob = family("checkpoint-noobai-xl", {
  tags: ["anime"],
  profile: profile({ style: ["anime", "illustration"], moderation: { level: "loose", note: L } }),
  variants: [variant("v1.1", "NoobAI-XL-v1.1.safetensors", { baseModel: "sdxl", recommended: true })],
});
const schnell = family("flux-schnell-gguf", {
  category: "flux",
  kind: "flux_gguf",
  profile: profile({ resources: { vramGb: 8, speed: "fast" } }),
  variants: [
    variant("q4", "flux1-schnell-Q4_K_S.gguf", { destDir: "unet", recommended: true }),
    variant("q8", "flux1-schnell-Q8_0.gguf", { destDir: "unet" }),
  ],
});
const vae = family("flux-vae", { category: "flux", variants: [variant("ae", "ae.safetensors", { destDir: "vae" })] });
const canny = family("controlnet-canny", {
  category: "controlnet",
  variants: [variant("x", "controlnet-canny-x.safetensors", { destDir: "controlnet", baseModel: "sdxl" })],
});
const anima = family("anima-base", {
  category: "anima",
  kind: "anima",
  tags: ["anime"],
  profile: profile({ style: ["anime", "illustration"], moderation: { level: "loose", note: L }, resources: { vramGb: 6, speed: "fast" } }),
  variants: [variant("anima-base-v1.0", "anima-base-v1.0.safetensors", { destDir: "unet", recommended: true })],
});
const animaTe = family("anima-text-encoder", {
  category: "anima",
  variants: [variant("te", "qwen_3_06b_base.safetensors", { destDir: "clip", recommended: true })],
});
const catalog: ModelCatalog = { schemaVersion: 2, families: [sd15, sdxl, noob, schnell, vae, canny, anima, animaTe] };

const engine = (over: Partial<EngineModels>): EngineModels => ({
  checkpoints: [],
  unets: [],
  ggufUnets: [],
  clips: [],
  vaes: [],
  loras: [],
  faceDetectors: [],
  controlnets: [],
  samplers: [],
  schedulers: [],
  taggers: [],
  ...over,
});

describe("モデルの説明(docs/18)", () => {
  it("ファイル名(大文字小文字は区別しない)でファミリーに結び付ける。選べないものは対象外", () => {
    expect(familyByFileName(catalog, "noobai-xl-v1.1.SAFETENSORS")?.id).toBe("checkpoint-noobai-xl");
    expect(familyByFileName(catalog, "flux1-schnell-Q8_0.gguf")?.id).toBe("flux-schnell-gguf");
    expect(familyByFileName(catalog, "controlnet-canny-x.safetensors")).toBeUndefined();
    expect(familyByFileName(catalog, "ae.safetensors")).toBeUndefined();
    expect(familyByFileName(catalog, "")).toBeUndefined();
    expect(familyByFileName(null, "x")).toBeUndefined();
  });

  it("種別はベースモデル・配置先から決まる", () => {
    expect(familyKinds(noob)).toEqual(["sdxl"]);
    expect(familyKinds(sd15)).toEqual(["sd15"]);
    expect(familyKinds(schnell)).toEqual(["flux_gguf"]);
    expect(familyKinds(anima)).toEqual(["anima"]);
  });

  it("Anima / Qwen-Image 2.1 / Krea 2 は拡散モデル(unets)から導入済みを判定し、切り替え先の種別も決まる(docs/17)", () => {
    const m = engine({ unets: ["anima-base-v1.0.safetensors"] });
    expect(installedFamilyIds(catalog, m)).toEqual(["anima-base"]);
    expect(switchTarget(anima, m)).toEqual({ kind: "anima", name: "anima-base-v1.0.safetensors" });
    // テキストエンコーダ・VAE は、生成画面で選べるモデルではない
    expect(familyByFileName(catalog, "qwen_3_06b_base.safetensors")).toBeUndefined();
    expect(profileFor(catalog, "anima", "anima-base-v1.0.safetensors")).toMatchObject({ exact: true });
    expect(profileFor(catalog, "anima", "other.safetensors")).toMatchObject({ exact: false });
    expect(profileFor(catalog, "anima", "other.safetensors")?.family.id).toBe("anima-base");
    // 対応機能: Anima はネガティブを使えるが、拡張機能はない。Krea 2 はネガティブも使わない
    expect(featuresFor("anima")).toEqual({ negative: true, faceDetailer: false, controlnet: false, inpaint: false, img2img: false });
    expect(featuresFor("qwen_image").negative).toBe(true);
    expect(featuresFor("krea2")).toEqual({ negative: false, faceDetailer: false, controlnet: false, inpaint: false, img2img: false });
  });

  it("一致すればそのモデルの説明、カタログ外・種別違いなら種別の一般説明(exact=false)", () => {
    const hit = profileFor(catalog, "sdxl", "NoobAI-XL-v1.1.safetensors");
    expect(hit).toMatchObject({ exact: true });
    expect(hit?.family.id).toBe("checkpoint-noobai-xl");

    const manual = profileFor(catalog, "sdxl", "my-private-model.safetensors");
    expect(manual).toMatchObject({ exact: false });
    expect(manual?.family.id).toBe("checkpoint-sdxl");
    expect(profileFor(catalog, "sd15", "x.safetensors")?.family.id).toBe("checkpoint-sd15");
    expect(profileFor(catalog, "flux_gguf", "x.gguf")?.family.id).toBe("flux-schnell-gguf");

    // SD1.5 の種別で SDXL のモデル名が選ばれていたら、一致扱いにしない
    expect(profileFor(catalog, "sd15", "NoobAI-XL-v1.1.safetensors")?.exact).toBe(false);
    expect(profileFor(null, "sdxl", "x")).toBeNull();
  });

  it("対応機能は種別から計算する(Flux 系は何も使えない)", () => {
    expect(featuresFor("sdxl")).toEqual({ negative: true, faceDetailer: true, controlnet: true, inpaint: true, img2img: true });
    expect(featuresFor("flux_gguf")).toEqual({ negative: false, faceDetailer: false, controlnet: false, inpaint: false, img2img: false });
  });

  it("導入済みのファミリー ID は、ComfyUI の一覧にあるファイルから決める(ファイル名は含めない)", () => {
    const ids = installedFamilyIds(
      catalog,
      engine({ checkpoints: ["NoobAI-XL-v1.1.safetensors", "other.safetensors"], ggufUnets: ["flux1-schnell-Q4_K_S.gguf"] }),
    );
    expect(ids).toEqual(["checkpoint-noobai-xl", "flux-schnell-gguf"]);
    expect(installedFamilyIds(catalog, null)).toEqual([]);
    expect(installedFamilyIds(null, engine({}))).toEqual([]);
  });

  it("AI へ渡す情報は、ファミリー ID だけ(カタログ外なら familyId は null)", () => {
    const p = {
      ...defaultParams("sdxl"),
      model: { kind: "sdxl" as const, name: "my-private.safetensors", clip: [], vae: null },
    };
    const ctx = modelContext(catalog, p, engine({ checkpoints: ["my-private.safetensors", "sd_xl_base_1.0.safetensors"] }));
    expect(ctx).toEqual({ familyId: null, installedFamilies: ["checkpoint-sdxl"] });
    expect(JSON.stringify(ctx)).not.toContain("my-private");

    const known = { ...p, model: { ...p.model, name: "NoobAI-XL-v1.1.safetensors" } };
    expect(modelContext(catalog, known, null)?.familyId).toBe("checkpoint-noobai-xl");
    expect(modelContext(null, known, null)).toBeUndefined();
  });

  it("推奨設定は、サイズ・ステップ・CFG・サンプラーだけ(プロンプトは含めない)", () => {
    expect(recommendedPatch(profile())).toEqual({
      width: 1024,
      height: 1024,
      steps: 25,
      cfg: 6.5,
      sampler: "dpmpp_2m",
      scheduler: "karras",
    });
    const noSampler = profile({ recommended: { steps: 4, cfg: 1, size: { width: 512, height: 768 } } });
    expect(recommendedPatch(noSampler)).toEqual({ width: 512, height: 768, steps: 4, cfg: 1 });
  });

  it("切り替え先は導入済みの variant(推奨を優先)。種別も決まる。未導入なら null", () => {
    expect(switchTarget(noob, engine({ checkpoints: ["NoobAI-XL-v1.1.safetensors"] }))).toEqual({
      kind: "sdxl",
      name: "NoobAI-XL-v1.1.safetensors",
    });
    expect(switchTarget(schnell, engine({ ggufUnets: ["flux1-schnell-Q8_0.gguf"] }))).toEqual({
      kind: "flux_gguf",
      name: "flux1-schnell-Q8_0.gguf",
    });
    expect(
      switchTarget(schnell, engine({ ggufUnets: ["flux1-schnell-Q8_0.gguf", "flux1-schnell-Q4_K_S.gguf"] }))?.name,
    ).toBe("flux1-schnell-Q4_K_S.gguf");
    expect(switchTarget(noob, engine({ checkpoints: ["other.safetensors"] }))).toBeNull();
    expect(switchTarget(noob, null)).toBeNull();
  });

  it("候補のバッジ: アニメ・規制の強さ・重い・カタログ外", () => {
    expect(candidateBadges(catalog, "NoobAI-XL-v1.1.safetensors")).toEqual(["anime", "moderation-loose"]);
    expect(candidateBadges(catalog, "sd_xl_base_1.0.safetensors")).toEqual([]);
    expect(candidateBadges(catalog, "unknown.safetensors")).toEqual(["uncatalogued"]);
    expect(candidateBadges(null, "unknown.safetensors")).toEqual([]);
  });
});
