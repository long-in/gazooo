import type { ModelKind } from "@/lib/tauri/jobs";
import { type ApplyMode, applyTranslation } from "@/lib/translate/apply";

/** 既定の解析モデル(docs/39 §4.2)。候補にあればこれを選ぶ。 */
export const DEFAULT_TAGGER_MODEL = "wd-v1-4-moat-tagger-v2";
export const DEFAULT_THRESHOLD = 0.35;
export const DEFAULT_CHARACTER_THRESHOLD = 0.85;
export const MIN_THRESHOLD = 0.05;
export const MAX_THRESHOLD = 1;

/** 解析結果を、現在のプロンプトへ反映した新しいプロンプトを返す(置き換え / 末尾に追加)。 */
export function applyTags(current: string, tags: string, mode: ApplyMode): string {
  return applyTranslation(current, tags, mode);
}

/**
 * タグ形式(Danbooru 系)が向かない = 文章で書くモデルか(docs/39 §6.2・Q-299)。
 * 該当するときは注記を出す(動作は妨げない)。
 */
export function prefersNaturalLanguage(kind: ModelKind): boolean {
  return kind === "flux" || kind === "flux_gguf" || kind === "qwen_image" || kind === "krea2";
}

/** 解析モデルの初期選択。候補に既定があればそれ、無ければ先頭。候補が空なら null。 */
export function pickTaggerModel(taggers: string[], current?: string): string | null {
  if (current && taggers.includes(current)) return current;
  if (taggers.includes(DEFAULT_TAGGER_MODEL)) return DEFAULT_TAGGER_MODEL;
  return taggers[0] ?? null;
}

/** しきい値の入力を範囲内に丸める(数値でなければ既定値)。 */
export function clampThreshold(v: number, fallback: number): number {
  if (!Number.isFinite(v)) return fallback;
  return Math.min(MAX_THRESHOLD, Math.max(MIN_THRESHOLD, v));
}
