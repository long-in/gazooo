import type { EngineModels } from "@/lib/tauri/engine";
import type { ModelCatalog } from "@/lib/tauri/models";
import type {
  ControlnetParams,
  ControlnetUnit,
  FaceDetailerParams,
  GenerationParams,
  Img2imgParams,
  InpaintParams,
  ModelKind,
  Preprocessor,
  FaceSwapParams,
  ReferenceParams,
} from "@/lib/tauri/jobs";

import { type FileKinds, fileKinds, filterForKind } from "./compat";
import { defaultImg2img } from "./img2img-defaults";
import { defaultInpaint } from "./inpaint";
import { defaultFaceSwap, supportsFaceSwap } from "./face-swap";
import { defaultReference, supportsReference } from "./reference";

export const MODEL_KINDS: ModelKind[] = ["sd15", "sdxl", "flux", "flux_gguf", "anima", "qwen_image", "krea2"];

export const MAX_SEED = Number.MAX_SAFE_INTEGER; // 2^53 - 1(Rust 側の上限と同じ)

export const isFlux = (kind: ModelKind) => kind === "flux" || kind === "flux_gguf";

/** 拡散モデル・テキストエンコーダ(1 つ)・VAE を別々に読む種別(docs/17。Anima / Qwen-Image 2.1 / Krea 2)。 */
export const isSplit = (kind: ModelKind): kind is "anima" | "qwen_image" | "krea2" =>
  kind === "anima" || kind === "qwen_image" || kind === "krea2";

/** VAE を別に選ぶ種別(Flux 系と、Anima / Qwen-Image 2.1 / Krea 2)。SD 系はチェックポイント内蔵の VAE が既定。 */
export const needsExternalVae = (kind: ModelKind) => isFlux(kind) || isSplit(kind);

/** ネガティブプロンプトを使う種別。Flux は使わず、Krea 2 は使えない(docs/17 §3)。 */
export const usesNegative = (kind: ModelKind) => !isFlux(kind) && kind !== "krea2";

/** 顔補正・ControlNet・Inpaint に対応するのは SD 1.5 / SDXL のみ。 */
export const supportsExtras = (kind: ModelKind) => kind === "sd15" || kind === "sdxl";

/** モデル種別ごとの推奨値(docs/05-ui-design.md §3)。 */
export function defaultParams(kind: ModelKind): GenerationParams {
  const base = {
    schemaVersion: 1,
    model: { kind, name: "", clip: [] as string[], vae: null as string | null },
    prompt: "",
    negativePrompt: "",
    seed: 0,
    seedRandomize: true,
    batchSize: 1,
  };
  switch (kind) {
    case "sd15":
      return { ...base, width: 512, height: 512, steps: 20, cfg: 7, guidance: 3.5, sampler: "euler", scheduler: "normal" };
    case "sdxl":
      return { ...base, width: 1024, height: 1024, steps: 25, cfg: 6.5, guidance: 3.5, sampler: "dpmpp_2m", scheduler: "karras" };
    case "flux":
    case "flux_gguf":
      return { ...base, width: 1024, height: 1024, steps: 20, cfg: 1, guidance: 3.5, sampler: "euler", scheduler: "simple" };
    // 公式の ComfyUI テンプレートの既定(docs/17 §4)
    case "anima":
      return {
        ...base,
        width: 1024,
        height: 1024,
        steps: 30,
        cfg: 4,
        guidance: 3.5,
        sampler: "euler",
        scheduler: "simple",
        negativePrompt: "worst quality, low quality, score_1, score_2, score_3, artist name, blurry, jpeg artifacts, chromatic aberration",
      };
    case "qwen_image":
      return { ...base, width: 1024, height: 1024, steps: 25, cfg: 1, guidance: 3.5, sampler: "euler", scheduler: "simple" };
    case "krea2":
      return { ...base, width: 1024, height: 1024, steps: 8, cfg: 1, guidance: 3.5, sampler: "euler", scheduler: "simple" };
  }
}

/** 顔補正の既定値(docs/10 §4.2)。無効の状態で始める。 */
export function defaultFaceDetailer(): FaceDetailerParams {
  return { enabled: false, detector: "", denoise: 0.4, prompt: "", confidence: 0.5 };
}

/** 顔補正に対応するモデル種別(SD 1.5 / SDXL)。 */
export const supportsFaceDetailer = (kind: ModelKind) => supportsExtras(kind);

/** ControlNet の既定値(docs/10 §5.2)。参照画像とモデルは未選択。 */
export function defaultControlnetUnit(): ControlnetUnit {
  return { image: "", model: "", preprocessor: "canny", strength: 1, startPercent: 0, endPercent: 1 };
}

export function defaultControlnet(): ControlnetParams {
  return { enabled: false, units: [] };
}

/** ControlNet に対応するモデル種別(SD 1.5 / SDXL)。 */
export const supportsControlnet = (kind: ModelKind) => supportsExtras(kind);

/** 画像から生成(img2img)に対応するモデル種別(SD 1.5 / SDXL)。 */
export const supportsImg2img = (kind: ModelKind) => supportsExtras(kind);

/** Inpaint に対応するモデル種別(SD 1.5 / SDXL)。 */
export const supportsInpaint = (kind: ModelKind) => supportsExtras(kind);

const MODEL_HINT: Record<Preprocessor, RegExp | undefined> = {
  canny: /canny/i,
  depth: /depth/i,
  openpose: /pose/i,
  none: undefined,
};

/**
 * 前処理に合う ControlNet モデルを選ぶ。現在のモデルが合うならそのまま、合わなければ名前に前処理名を含む
 * 最初のもの、それも無ければ現在のモデル(一覧にあれば)、最後に一覧の先頭。一覧が空なら現在値のまま。
 */
export function pickControlnetModel(controlnets: string[], preprocessor: Preprocessor, current: string): string {
  if (controlnets.length === 0) return current;
  const hint = MODEL_HINT[preprocessor];
  if (hint?.test(current) && controlnets.includes(current)) return current;
  const match = hint && controlnets.find((n) => hint.test(n));
  if (match) return match;
  return controlnets.includes(current) ? current : controlnets[0];
}

/** 種別を切り替える。プロンプトとシード設定は引き継ぎ、サイズ・ステップ・サンプラー等は種別の既定に戻す。 */
export function switchKind(current: GenerationParams, kind: ModelKind): GenerationParams {
  if (current.model.kind === kind) return current;
  return {
    ...defaultParams(kind),
    prompt: current.prompt,
    negativePrompt: current.negativePrompt,
    seed: current.seed,
    seedRandomize: current.seedRandomize,
    batchSize: current.batchSize,
    // 顔補正の設定は引き継ぐ。非対応の種別(Flux)へ切り替えるときは無効にする(値は残す)
    ...(current.faceDetailer && {
      faceDetailer: { ...current.faceDetailer, enabled: current.faceDetailer.enabled && supportsFaceDetailer(kind) },
    }),
    // ControlNet も同様(参照画像などの設定は残し、非対応の種別では無効にする)
    ...(current.controlnet && {
      controlnet: { ...current.controlnet, enabled: current.controlnet.enabled && supportsControlnet(kind) },
    }),
    // 画像から生成も同様(元画像は残し、非対応の種別では無効にする)
    ...(current.img2img && {
      img2img: { ...current.img2img, enabled: current.img2img.enabled && supportsImg2img(kind) },
    }),
    // Inpaint も同様(参照画像・マスクは残し、非対応の種別では無効にする)
    ...(current.inpaint && {
      inpaint: { ...current.inpaint, enabled: current.inpaint.enabled && supportsInpaint(kind) },
    }),
    // 参照画像つき生成(Qwen-Image 2.1)も同様(画像は残し、非対応の種別では無効にする)
    ...(current.reference && {
      reference: { ...current.reference, enabled: current.reference.enabled && supportsReference(kind) },
    }),
    // 顔の入れ替えも同様(画像は残し、非対応の種別では無効にする)
    ...(current.faceSwap && {
      faceSwap: { ...current.faceSwap, enabled: current.faceSwap.enabled && supportsFaceSwap(kind) },
    }),
  };
}

/** ネガティブプロンプトの既定値(種別ごと。docs/21 §2.1)。既定が無い種別では空。 */
export const defaultNegative = (kind: ModelKind): string => defaultParams(kind).negativePrompt;

/**
 * 「すべて初期化」(docs/21 §2.2): プロンプト・サイズ・ステップ等を種別の推奨値に戻す。
 * モデル種別・モデル・CLIP・VAE の選択は保持し、拡張機能は設定ごと外す(参照画像も捨てる)。
 */
export function resetParams(current: GenerationParams): GenerationParams {
  return { ...defaultParams(current.model.kind), model: current.model };
}

/** 種別に対応するモデル一覧。 */
export function modelsFor(kind: ModelKind, models: EngineModels | null): string[] {
  if (!models) return [];
  switch (kind) {
    case "sd15":
    case "sdxl":
      return models.checkpoints;
    case "flux":
    case "anima":
    case "qwen_image":
    case "krea2":
      return models.unets;
    case "flux_gguf":
      return models.ggufUnets;
  }
}

/** カタログから作るファイル → 種別の索引(カタログが無ければ空 = 絞り込まない)。 */
const indexOf = (catalog: ModelCatalog | null | undefined): FileKinds => fileKinds(catalog);

/** 種別に合う本体の一覧(カタログで他の種別のものと分かるファイルは除く。docs/20 §2)。 */
export function modelsForKind(kind: ModelKind, models: EngineModels | null, catalog?: ModelCatalog | null): string[] {
  return filterForKind(modelsFor(kind, models), kind, indexOf(catalog));
}

/** 種別に合うテキストエンコーダの一覧。 */
export function clipsForKind(kind: ModelKind, models: EngineModels | null, catalog?: ModelCatalog | null): string[] {
  return filterForKind(models?.clips ?? [], kind, indexOf(catalog));
}

/** 種別に合う VAE の一覧。 */
export function vaesForKind(kind: ModelKind, models: EngineModels | null, catalog?: ModelCatalog | null): string[] {
  return filterForKind(models?.vaes ?? [], kind, indexOf(catalog));
}

/** 導入済みのモデルがある種別(生成画面の種別の選択肢に使う。docs/20 §2.3)。 */
export function installedKinds(models: EngineModels | null, catalog?: ModelCatalog | null): Set<ModelKind> {
  return new Set(MODEL_KINDS.filter((k) => modelsForKind(k, models, catalog).length > 0));
}

/** Anima / Qwen-Image 2.1 / Krea 2 の、テキストエンコーダと VAE を一覧から選ぶときの目印(docs/17 §2)。 */
const SPLIT_FILES: Record<"anima" | "qwen_image" | "krea2", { clip: RegExp; vae: RegExp }> = {
  anima: { clip: /qwen_3_06b|qwen3_0\.6b/i, vae: /^qwen_image_vae\b/i },
  qwen_image: { clip: /qwen3vl_8b/i, vae: /qwen_image_2\.1_vae/i },
  krea2: { clip: /qwen3vl_4b/i, vae: /^qwen_image_vae\b/i },
};

const first = (list: string[], pattern?: RegExp) =>
  (pattern ? list.find((n) => pattern.test(n)) : undefined) ?? list[0] ?? "";

/**
 * エンジンのモデル一覧に合わせて、未選択・存在しないモデルを埋める。
 * 既に有効な選択はそのまま保つ。一覧が未取得(null)なら何も変えない。
 */
export function autofill(
  params: GenerationParams,
  models: EngineModels | null,
  catalog?: ModelCatalog | null,
): GenerationParams {
  if (!models) return params;
  const { kind } = params.model;
  // 種別に合うものだけを候補にする(カタログが無ければ全件)
  const list = modelsForKind(kind, models, catalog);
  const clips = clipsForKind(kind, models, catalog);
  const vaes = vaesForKind(kind, models, catalog);
  const name = list.includes(params.model.name) ? params.model.name : (list[0] ?? "");

  let clip = params.model.clip;
  let vae = params.model.vae;
  if (isFlux(kind)) {
    const t5 = clips.includes(clip[0]) ? clip[0] : first(clips, /t5/i);
    const clipL = clips.includes(clip[1]) ? clip[1] : first(clips, /clip_l|clip-l/i);
    clip = [t5, clipL];
    vae = vae && vaes.includes(vae) ? vae : first(vaes, /^ae\b|ae\.|flux/i) || null;
  } else if (kind === "anima" || kind === "qwen_image" || kind === "krea2") {
    // テキストエンコーダは 1 つ。種別に合うものを優先して選ぶ(有効な選択は保つ)
    const hint = SPLIT_FILES[kind];
    const te = clips.includes(clip[0]) ? clip[0] : first(clips, hint.clip);
    clip = [te];
    vae = vae && vaes.includes(vae) ? vae : first(vaes, hint.vae) || null;
  } else {
    clip = [];
    // SD 系の VAE は任意(null = チェックポイント内蔵)。存在しない指定だけ外す。
    vae = vae && vaes.includes(vae) ? vae : null;
  }

  const sampler = models.samplers.includes(params.sampler) ? params.sampler : (models.samplers[0] ?? params.sampler);
  const scheduler = models.schedulers.includes(params.scheduler)
    ? params.scheduler
    : (models.schedulers[0] ?? params.scheduler);

  // 顔補正の検出モデル: 未選択・存在しないものは、顔用(face)を優先して一覧から選ぶ。
  // 一覧が空(ノード未導入)なら触らない(設定は残し、利用可否は画面側で判断する)
  let faceDetailer = params.faceDetailer;
  if (faceDetailer && models.faceDetectors.length > 0 && !models.faceDetectors.includes(faceDetailer.detector)) {
    faceDetailer = { ...faceDetailer, detector: first(models.faceDetectors, /face/i) };
  }

  // ControlNet のモデル: 前処理に合うものを選ぶ。有効な選択は保つ(一覧が空なら触らない)
  let controlnet = params.controlnet;
  const unit = controlnet?.units[0];
  if (controlnet && unit) {
    const model = pickControlnetModel(models.controlnets, unit.preprocessor, unit.model);
    if (model !== unit.model) controlnet = { ...controlnet, units: [{ ...unit, model }] };
  }

  const next = {
    ...params,
    model: { ...params.model, name, clip, vae },
    sampler,
    scheduler,
    ...(faceDetailer && { faceDetailer }),
    ...(controlnet && { controlnet }),
  };
  return JSON.stringify(next) === JSON.stringify(params) ? params : next;
}

/** 保存済みの下書き(古い形式・欠損を含みうる)を、種別の既定値の上にマージして復元する。 */
export function restoreParams(saved: Partial<GenerationParams> | null | undefined): GenerationParams {
  const kind = saved?.model?.kind;
  const base = defaultParams(kind && MODEL_KINDS.includes(kind) ? kind : "sdxl");
  if (!saved) return base;
  return {
    ...base,
    ...saved,
    model: { ...base.model, ...saved.model, clip: saved.model?.clip ?? [], vae: saved.model?.vae ?? null },
    // 古い下書き・プリセット(顔補正なし)は無効のまま。ある場合は欠けた項目を既定で埋める
    ...(saved.faceDetailer && { faceDetailer: { ...defaultFaceDetailer(), ...saved.faceDetailer } }),
    ...(saved.controlnet && {
      controlnet: {
        enabled: saved.controlnet.enabled ?? false,
        units: (saved.controlnet.units ?? []).slice(0, 1).map((u) => ({ ...defaultControlnetUnit(), ...u })),
      },
    }),
    ...(saved.inpaint && { inpaint: { ...defaultInpaint(), ...saved.inpaint } satisfies InpaintParams }),
    ...(saved.img2img && { img2img: { ...defaultImg2img(), ...saved.img2img } satisfies Img2imgParams }),
    ...(saved.reference && {
      reference: { ...defaultReference(), ...saved.reference, images: saved.reference.images ?? [] } satisfies ReferenceParams,
    }),
    ...(saved.faceSwap && { faceSwap: { ...defaultFaceSwap(), ...saved.faceSwap } satisfies FaceSwapParams }),
  };
}

export function randomSeed(): number {
  // 乱数は 2^53 未満の整数(Rust 側と同じ範囲)
  const buf = new Uint32Array(2);
  crypto.getRandomValues(buf);
  return (buf[0] % 0x200000) * 0x100000000 + buf[1];
}

/** サイズの定番プリセット(64 の倍数)。 */
export function sizePresets(kind: ModelKind): { w: number; h: number }[] {
  return kind === "sd15"
    ? [{ w: 512, h: 512 }, { w: 512, h: 768 }, { w: 768, h: 512 }, { w: 768, h: 768 }]
    : [{ w: 1024, h: 1024 }, { w: 832, h: 1216 }, { w: 1216, h: 832 }, { w: 768, h: 768 }];
}

export type SizeOrientation = "portrait" | "square" | "landscape";
export type SizeScale = "small" | "medium" | "large";

const round64 = (n: number) => Math.max(64, Math.round(n / 64) * 64);

/** かんたんモードの画像サイズ(縦長 / 正方形 / 横長 × 小・中・大。docs/38 §2.2)。64 の倍数。 */
export function simpleSize(kind: ModelKind, orientation: SizeOrientation, scale: SizeScale): { w: number; h: number } {
  const base = (kind === "sd15" ? { small: 512, medium: 640, large: 768 } : { small: 768, medium: 1024, large: 1280 })[scale];
  if (orientation === "square") return { w: base, h: base };
  const short = round64(base * 0.8);
  const long = round64(base * 1.2);
  return orientation === "portrait" ? { w: short, h: long } : { w: long, h: short };
}

/** 現在のサイズに合うかんたんモードの選択(合わなければ null)。 */
export function matchSimpleSize(
  kind: ModelKind,
  width: number,
  height: number,
): { orientation: SizeOrientation; scale: SizeScale } | null {
  for (const orientation of ["portrait", "square", "landscape"] as const) {
    for (const scale of ["small", "medium", "large"] as const) {
      const { w, h } = simpleSize(kind, orientation, scale);
      if (w === width && h === height) return { orientation, scale };
    }
  }
  return null;
}
