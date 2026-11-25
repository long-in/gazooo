import { describe, expect, it } from "vitest";

import type { DownloadState, ModelFamily, ModelVariant } from "@/lib/tauri/models";

import {
  bundleFor,
  bundleMissingBytes,
  downloadFor,
  etaSeconds,
  formatBytes,
  formatDuration,
  isBase,
  isCategory,
  isPurpose,
  matchesBase,
  matchesPurpose,
  pickVariant,
  progressPercent,
  visibleFamilies,
} from "./catalog";

const L = { ja: "x", en: "x" };
const variant = (id: string, over: Partial<ModelVariant> = {}): ModelVariant => ({
  id,
  label: L,
  recommended: false,
  source: { url: "https://huggingface.co/a/b/resolve/c/m.safetensors", repo: "a/b" },
  sizeBytes: 1,
  sha256: "0".repeat(64),
  destDir: "controlnet",
  fileName: `${id}.safetensors`,
  license: { name: "l", url: "https://x", commercialUse: true },
  requiresConsent: false,
  ...over,
});
const family = (id: string, category: ModelFamily["category"], variants: ModelVariant[]): ModelFamily => ({
  id,
  category,
  name: L,
  description: L,
  usedBy: [],
  tags: [],
  variants,
});

const canny = family("controlnet-canny", "controlnet", [
  variant("sdxl-xinsir", { baseModel: "sdxl", recommended: true }),
  variant("sdxl-diffusers", { baseModel: "sdxl" }),
  variant("sd15", { baseModel: "sd15", recommended: true }),
]);
const face = family("detector-face", "detector", [variant("yolov9c", { recommended: true }), variant("yolov8m")]);
const sd15ckpt = family("checkpoint-sd15", "checkpoint", [variant("a", { baseModel: "sd15" })]);

const noob: ModelFamily = {
  ...family("checkpoint-noobai-xl", "checkpoint", [variant("v1.1", { baseModel: "sdxl", recommended: true })]),
  tags: ["anime"],
};
const animagine: ModelFamily = {
  ...family("checkpoint-animagine-xl-4", "checkpoint", [variant("opt", { baseModel: "sdxl", recommended: true })]),
  tags: ["anime"],
};
const sdxlckpt = family("checkpoint-sdxl", "checkpoint", [variant("base", { baseModel: "sdxl", recommended: true })]);

describe("用途フィルタ(docs/16 §3.3)", () => {
  const all = [sd15ckpt, sdxlckpt, noob, animagine];

  it("チェックポイントのタブで、タグを持つファミリーだけに絞る", () => {
    expect(visibleFamilies(all, "checkpoint", "all", "all").map((x) => x.family.id)).toHaveLength(4);
    expect(visibleFamilies(all, "checkpoint", "all", "anime").map((x) => x.family.id)).toEqual([
      "checkpoint-noobai-xl",
      "checkpoint-animagine-xl-4",
    ]);
  });

  it("ベースモデルのフィルタと併用できる", () => {
    expect(visibleFamilies(all, "checkpoint", "sd15", "anime")).toEqual([]);
    expect(visibleFamilies(all, "checkpoint", "sdxl", "anime")).toHaveLength(2);
  });

  it("用途フィルタはチェックポイント以外のタブでは効かない", () => {
    expect(visibleFamilies([canny, face], "controlnet", "all", "anime")).toHaveLength(1);
    expect(visibleFamilies([canny, face], "detector", "all", "anime")).toHaveLength(1);
  });

  it("既定では絞り込まない", () => {
    expect(visibleFamilies(all, "checkpoint", "all")).toHaveLength(4);
    expect(matchesPurpose(sd15ckpt, "all")).toBe(true);
    expect(matchesPurpose(sd15ckpt, "anime")).toBe(false);
  });

  it("URL パラメータの値を検証する", () => {
    expect(isPurpose("anime")).toBe(true);
    expect(isPurpose("nsfw")).toBe(false);
    expect(isPurpose("all")).toBe(true);
    expect(isPurpose("evil")).toBe(false);
    expect(isPurpose(null)).toBe(false);
  });
});

describe("絞り込み", () => {
  it("カテゴリで絞る", () => {
    const r = visibleFamilies([canny, face, sd15ckpt], "controlnet", "all");
    expect(r.map((x) => x.family.id)).toEqual(["controlnet-canny"]);
    expect(r[0].variants).toHaveLength(3);
  });

  it("ベースモデルで variant を絞る。表示できる variant が無いファミリーは除く", () => {
    const r = visibleFamilies([canny, face, sd15ckpt], "controlnet", "sd15");
    expect(r[0].variants.map((v) => v.id)).toEqual(["sd15"]);
    expect(visibleFamilies([sd15ckpt], "checkpoint", "sdxl")).toEqual([]);
  });

  it("ベースモデルを持たない variant(検出モデル)はどの絞り込みでも表示する", () => {
    expect(visibleFamilies([face], "detector", "sdxl")[0].variants).toHaveLength(2);
    expect(matchesBase(face.variants[0], "sd15")).toBe(true);
  });

  it("URL パラメータの値を検証する", () => {
    expect(isCategory("controlnet")).toBe(true);
    expect(isCategory("evil")).toBe(false);
    expect(isCategory(null)).toBe(false);
    expect(isBase("sdxl")).toBe(true);
    expect(isBase("flux")).toBe(false);
  });
});

describe("variant の選択", () => {
  const sdxl = canny.variants.filter((v) => v.baseModel === "sdxl");

  it("ユーザーの選択が表示中ならそれを使う", () => {
    expect(pickVariant(sdxl, "sdxl-diffusers").id).toBe("sdxl-diffusers");
  });

  it("選択が絞り込みで消えていたら、推奨にする", () => {
    expect(pickVariant(sdxl, "sd15").id).toBe("sdxl-xinsir");
    expect(pickVariant(sdxl, undefined).id).toBe("sdxl-xinsir");
  });

  it("推奨が無ければ先頭", () => {
    expect(pickVariant([variant("p"), variant("q")], undefined).id).toBe("p");
  });
});

describe("ダウンロード状態", () => {
  const d = (over: Partial<DownloadState>): DownloadState => ({
    downloadId: "1",
    familyId: "f",
    variantId: "v",
    status: "downloading",
    receivedBytes: 50,
    totalBytes: 200,
    ...over,
  });

  it("同じモデルの取得中を優先して返す", () => {
    const list = [d({ downloadId: "old", status: "failed" }), d({ downloadId: "new", status: "queued" })];
    expect(downloadFor(list, "f", "v")?.downloadId).toBe("new");
    expect(downloadFor(list, "f", "other")).toBeUndefined();
  });

  it("進捗率は 0〜100 に収める", () => {
    expect(progressPercent(d({}))).toBe(25);
    expect(progressPercent(d({ receivedBytes: 999 }))).toBe(100);
    expect(progressPercent(d({ totalBytes: 0 }))).toBe(0);
  });

  it("残り時間は速度が分かるときだけ", () => {
    expect(etaSeconds(d({ bytesPerSec: 50 }))).toBe(3);
    expect(etaSeconds(d({}))).toBeNull();
  });
});

describe("書式", () => {
  it("バイト数", () => {
    expect(formatBytes(6_938_078_334)).toBe("6.9 GB");
    expect(formatBytes(52_026_019)).toBe("52 MB");
    expect(formatBytes(2_502_139_136)).toBe("2.5 GB");
    expect(formatBytes(512)).toBe("512 B");
  });

  it("時間", () => {
    expect(formatDuration(5)).toBe("5s");
    expect(formatDuration(125)).toBe("2m 5s");
    expect(formatDuration(3700)).toBe("1h 1m");
  });
});

describe("まとめて取得(docs/17 §5、Q-70)", () => {
  const sized = (id: string, file: string, size: number, dest: ModelVariant["destDir"]) =>
    variant(id, { fileName: file, sizeBytes: size, destDir: dest, recommended: true });
  const model: ModelFamily = {
    ...family("anima-base", "anima", [sized("m", "anima-base-v1.0.safetensors", 4_000, "unet")]),
    kind: "anima",
    requires: [
      { familyId: "anima-te", variantId: "te" },
      { familyId: "anima-vae", variantId: "vae" },
      { familyId: "missing", variantId: "x" },
    ],
  };
  const te = family("anima-te", "anima", [sized("te", "te.safetensors", 1_000, "clip")]);
  const vae = family("anima-vae", "anima", [sized("vae", "qwen_image_vae.safetensors", 250, "vae")]);
  const all = [model, te, vae];

  it("本体のあとに、必要なファイルが並ぶ(参照先が無いものは飛ばす)", () => {
    const items = bundleFor(all, model, model.variants[0], () => false);
    expect(items.map((i) => i.variant.id)).toEqual(["m", "te", "vae"]);
    expect(items.every((i) => !i.installed)).toBe(true);
  });

  it("requires が無いファミリーは対象外", () => {
    expect(bundleFor(all, te, te.variants[0], () => false)).toEqual([]);
  });

  it("導入済みの印と、未導入の合計サイズ", () => {
    const installed = new Set(["anima-te/te"]);
    const items = bundleFor(all, model, model.variants[0], (f, v) => installed.has(`${f}/${v}`));
    expect(items.map((i) => i.installed)).toEqual([false, true, false]);
    expect(bundleMissingBytes(items)).toBe(4_250);
    expect(bundleMissingBytes(items.map((i) => ({ ...i, installed: true })))).toBe(0);
  });

  it("同じファイル(配置先とファイル名が同じ)は、サイズを重複して数えない", () => {
    const vae2 = family("krea2-vae", "krea2", [sized("vae", "qwen_image_vae.safetensors", 250, "vae")]);
    const items = [
      { family: vae, variant: vae.variants[0], installed: false },
      { family: vae2, variant: vae2.variants[0], installed: false },
    ];
    expect(bundleMissingBytes(items)).toBe(250);
  });
});
