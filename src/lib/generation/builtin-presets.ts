import type { GenerationParams } from "@/lib/tauri/jobs";
import type { Localized } from "@/lib/tauri/models";

import { defaultParams } from "./defaults";

/**
 * アプリ組み込みのプリセット(docs/16 §5)。ユーザーのプリセット(DB)とは別に、コードの定数として持つ。
 * 編集・削除はできない。適用すると、生成の設定(サイズ・ステップ・CFG・サンプラー・ネガティブ)だけを変え、
 * 今のプロンプト・モデルの選択・シード・顔補正などの拡張機能は保つ。
 *
 * 値は、アニメ系 SDXL(Animagine XL 4.0 / NoobAI-XL)の配布元の推奨に共通する無難なもの。
 * モデル固有のタグ(品質タグなど)は、プリセットでは吸収せず、説明と AI サポートで案内する(docs/16 §5)。
 */
export interface BuiltinPreset {
  id: string;
  name: Localized;
  description: Localized;
  /** 適用する設定。 */
  settings: Pick<
    GenerationParams,
    "width" | "height" | "steps" | "cfg" | "sampler" | "scheduler" | "negativePrompt"
  >;
}

const ANIME_NEGATIVE = "lowres, bad anatomy, worst quality";

const ANIME_COMMON = {
  steps: 28,
  cfg: 5,
  sampler: "euler_ancestral",
  scheduler: "normal",
  negativePrompt: ANIME_NEGATIVE,
} as const;

export const BUILTIN_PRESETS: BuiltinPreset[] = [
  {
    id: "builtin-anime-sdxl-portrait",
    name: { ja: "アニメ(SDXL・縦長)", en: "Anime (SDXL, portrait)" },
    description: {
      ja: "アニメ系の SDXL モデル向け。832×1216、28 ステップ、CFG 5、Euler a。Danbooru タグで書くと効きやすい",
      en: "For anime SDXL models. 832x1216, 28 steps, CFG 5, Euler a. Works best with Danbooru-style tags",
    },
    settings: { width: 832, height: 1216, ...ANIME_COMMON },
  },
  {
    id: "builtin-anime-sdxl-square",
    name: { ja: "アニメ(SDXL・正方形)", en: "Anime (SDXL, square)" },
    description: {
      ja: "アニメ系の SDXL モデル向け。1024×1024、28 ステップ、CFG 5、Euler a。Danbooru タグで書くと効きやすい",
      en: "For anime SDXL models. 1024x1024, 28 steps, CFG 5, Euler a. Works best with Danbooru-style tags",
    },
    settings: { width: 1024, height: 1024, ...ANIME_COMMON },
  },
];

export const isBuiltinPresetId = (id: string) => BUILTIN_PRESETS.some((p) => p.id === id);

/**
 * 組み込みプリセットを適用した結果。SDXL 向けなので、他の種別(SD1.5 / Flux)を選んでいるときは、
 * 種別を SDXL に切り替える(モデルは空にして、あとでエンジンの一覧から選び直す)。
 */
export function applyBuiltinPreset(current: GenerationParams, preset: BuiltinPreset): GenerationParams {
  const model = current.model.kind === "sdxl" ? current.model : defaultParams("sdxl").model;
  return { ...current, ...preset.settings, model };
}
