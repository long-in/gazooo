import { describe, expect, it } from "vitest";

import type { EngineModels } from "@/lib/tauri/engine";
import type { ModelCatalog, ModelFamily, ModelProfile, ModelVariant } from "@/lib/tauri/models";

import { fileKinds, fileKey, isCatalogued, usableFor } from "./compat";
import { autofill, clipsForKind, defaultParams, installedKinds, modelsForKind, vaesForKind } from "./defaults";

const L = { ja: "x", en: "x" };
const profile = {} as ModelProfile;
const variant = (fileName: string, over: Partial<ModelVariant> = {}): ModelVariant =>
  ({
    id: fileName,
    label: L,
    recommended: true,
    source: { url: "https://huggingface.co/a/b/resolve/c/m", repo: "a/b" },
    sizeBytes: 1,
    sha256: "0".repeat(64),
    destDir: "checkpoints",
    fileName,
    license: { name: "l", url: "https://x", commercialUse: true },
    requiresConsent: false,
    ...over,
  }) as ModelVariant;
const fam = (id: string, over: Partial<ModelFamily>): ModelFamily =>
  ({ id, category: "checkpoint", name: L, description: L, usedBy: [], tags: [], variants: [], ...over }) as ModelFamily;

const catalog: ModelCatalog = {
  schemaVersion: 2,
  families: [
    fam("checkpoint-sd15", { profile, variants: [variant("v1-5.safetensors", { baseModel: "sd15" })] }),
    fam("checkpoint-sdxl", { profile, variants: [variant("sdxl-base.safetensors", { baseModel: "sdxl" })] }),
    fam("anima-base", { category: "anima", kind: "anima", profile, variants: [variant("anima.safetensors", { destDir: "unet" })] }),
    fam("krea2-turbo", { category: "krea2", kind: "krea2", profile, variants: [variant("krea2.safetensors", { destDir: "unet" })] }),
    fam("anima-vae", { category: "anima", variants: [variant("qwen_image_vae.safetensors", { destDir: "vae" })] }),
    fam("krea2-vae", { category: "krea2", variants: [variant("qwen_image_vae.safetensors", { destDir: "vae" })] }),
    fam("anima-te", { category: "anima", variants: [variant("qwen_3_06b.safetensors", { destDir: "clip" })] }),
    fam("krea2-te", { category: "krea2", variants: [variant("qwen3vl_4b.safetensors", { destDir: "clip" })] }),
    fam("flux-vae", { category: "flux", variants: [variant("ae.safetensors", { destDir: "vae" })] }),
    fam("controlnet-canny", { category: "controlnet", variants: [variant("canny.safetensors", { destDir: "controlnet" })] }),
  ],
};

const models = (over: Partial<EngineModels> = {}): EngineModels => ({
  checkpoints: ["v1-5.safetensors", "sdxl-base.safetensors", "my-merge.safetensors"],
  unets: ["anima.safetensors", "krea2.safetensors"],
  ggufUnets: [],
  clips: ["qwen_3_06b.safetensors", "qwen3vl_4b.safetensors"],
  vaes: ["qwen_image_vae.safetensors", "ae.safetensors"],
  loras: [],
  faceDetectors: [],
  controlnets: [],
  samplers: ["euler"],
  schedulers: ["normal"],
  taggers: [],
  ...over,
});

describe("ファイルと種別の索引", () => {
  const index = fileKinds(catalog);

  it("チェックポイントはベースモデル、本体は kind、エンコーダ・VAE はカテゴリの種別", () => {
    expect([...index.get("v1-5.safetensors")!]).toEqual(["sd15"]);
    expect([...index.get("sdxl-base.safetensors")!]).toEqual(["sdxl"]);
    expect([...index.get("krea2.safetensors")!]).toEqual(["krea2"]);
    expect([...index.get("qwen3vl_4b.safetensors")!]).toEqual(["krea2"]);
    expect([...index.get("ae.safetensors")!].sort()).toEqual(["flux", "flux_gguf"]);
  });

  it("同じファイルが複数のファミリーにあれば和集合。ControlNet は対象外", () => {
    expect([...index.get("qwen_image_vae.safetensors")!].sort()).toEqual(["anima", "krea2"]);
    expect(index.has("canny.safetensors")).toBe(false);
  });

  it("パス区切り・大文字小文字を問わず、カタログ外は判断しない(使ってよい)", () => {
    expect(fileKey("Sub\\Dir/SDXL-Base.safetensors")).toBe("sdxl-base.safetensors");
    expect(usableFor(index, "sub/SDXL-Base.safetensors", "sdxl")).toBe(true);
    expect(usableFor(index, "sdxl-base.safetensors", "sd15")).toBe(false);
    expect(usableFor(index, "my-merge.safetensors", "sd15")).toBe(true);
    expect(isCatalogued(index, "my-merge.safetensors")).toBe(false);
  });
});

describe("種別ごとの選択肢", () => {
  it("本体は種別に合うものとカタログ外だけ。カタログが無ければ絞り込まない", () => {
    expect(modelsForKind("sd15", models(), catalog)).toEqual(["v1-5.safetensors", "my-merge.safetensors"]);
    expect(modelsForKind("sdxl", models(), catalog)).toEqual(["sdxl-base.safetensors", "my-merge.safetensors"]);
    expect(modelsForKind("anima", models(), catalog)).toEqual(["anima.safetensors"]);
    expect(modelsForKind("krea2", models(), catalog)).toEqual(["krea2.safetensors"]);
    expect(modelsForKind("sd15", models(), null)).toHaveLength(3);
  });

  it("エンコーダ・VAE も絞る(共有の VAE は両方の種別に出る)", () => {
    expect(clipsForKind("anima", models(), catalog)).toEqual(["qwen_3_06b.safetensors"]);
    expect(clipsForKind("krea2", models(), catalog)).toEqual(["qwen3vl_4b.safetensors"]);
    expect(vaesForKind("anima", models(), catalog)).toEqual(["qwen_image_vae.safetensors"]);
    expect(vaesForKind("krea2", models(), catalog)).toEqual(["qwen_image_vae.safetensors"]);
    expect(vaesForKind("flux", models(), catalog)).toEqual(["ae.safetensors"]);
  });

  it("導入済みの種別だけが選べる", () => {
    const only = models({ checkpoints: ["sdxl-base.safetensors"], unets: ["krea2.safetensors"] });
    expect([...installedKinds(only, catalog)].sort()).toEqual(["krea2", "sdxl"]);
    // カタログ外のチェックポイントは、SD 1.5 / SDXL の両方として導入済みに数える
    const own = models({ checkpoints: ["my-merge.safetensors"], unets: [] });
    expect([...installedKinds(own, catalog)].sort()).toEqual(["sd15", "sdxl"]);
  });
});

describe("autofill は種別に合うものを選ぶ", () => {
  it("合わないモデルが選ばれていたら、種別に合う先頭へ差し替える", () => {
    const p = { ...defaultParams("sd15"), model: { ...defaultParams("sd15").model, name: "sdxl-base.safetensors" } };
    expect(autofill(p, models(), catalog).model.name).toBe("v1-5.safetensors");
    // カタログが無いときは、従来どおり(一覧にあれば保つ)
    expect(autofill(p, models(), null).model.name).toBe("sdxl-base.safetensors");
  });

  it("分割モデルのエンコーダ・VAE は、種別に合うものを選ぶ", () => {
    const base = defaultParams("krea2");
    const r = autofill({ ...base, model: { ...base.model, clip: ["qwen_3_06b.safetensors"] } }, models(), catalog);
    expect(r.model.name).toBe("krea2.safetensors");
    expect(r.model.clip).toEqual(["qwen3vl_4b.safetensors"]);
    expect(r.model.vae).toBe("qwen_image_vae.safetensors");
  });
});
