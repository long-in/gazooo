import type { SuggestedChanges } from "@/lib/tauri/assistant";
import type { EngineModels } from "@/lib/tauri/engine";
import type { GenerationParams } from "@/lib/tauri/jobs";

import { defaultFaceDetailer, isFlux, supportsExtras, usesNegative } from "./defaults";

/** AI の提案をフォームへ適用する・差分を表示する・元に戻すためのロジック(docs/11 §6)。 */

type ScalarKey =
  | "prompt"
  | "negativePrompt"
  | "width"
  | "height"
  | "steps"
  | "cfg"
  | "guidance"
  | "sampler"
  | "scheduler"
  | "batchSize";

const SCALAR_KEYS: ScalarKey[] = [
  "prompt",
  "negativePrompt",
  "width",
  "height",
  "steps",
  "cfg",
  "guidance",
  "sampler",
  "scheduler",
  "batchSize",
];

/**
 * フォーム側の事情に合わせて、適用できる項目だけに絞る。
 * - sampler / scheduler は、取得済みの一覧にあるものだけ(一覧が未取得なら素通し)
 * - 現在のモデル種別に合わない項目(Flux の cfg・ネガティブ、SD 系の guidance・顔補正)は除く
 */
export function usableChanges(
  params: GenerationParams,
  changes: SuggestedChanges,
  models: EngineModels | null,
): SuggestedChanges {
  const flux = isFlux(params.model.kind);
  const out: SuggestedChanges = {};
  for (const key of SCALAR_KEYS) {
    const v = changes[key];
    if (v === undefined) continue;
    if (flux && key === "cfg") continue;
    if (key === "negativePrompt" && !usesNegative(params.model.kind)) continue;
    if (!flux && key === "guidance") continue;
    if (key === "sampler" && models && models.samplers.length > 0 && !models.samplers.includes(v as string)) continue;
    if (key === "scheduler" && models && models.schedulers.length > 0 && !models.schedulers.includes(v as string)) continue;
    (out as Record<string, unknown>)[key] = v;
  }
  if (changes.faceDetailer && supportsExtras(params.model.kind)) {
    const f: NonNullable<SuggestedChanges["faceDetailer"]> = {};
    if (changes.faceDetailer.enabled !== undefined) f.enabled = changes.faceDetailer.enabled;
    if (changes.faceDetailer.denoise !== undefined) f.denoise = changes.faceDetailer.denoise;
    if (Object.keys(f).length > 0) out.faceDetailer = f;
  }
  return out;
}

export interface DiffRow {
  /** 項目名の i18n 用キー(`generate.*` などに対応させる)。 */
  field: ScalarKey | "faceEnabled" | "faceDenoise";
  from: string;
  to: string;
}

const show = (v: unknown) => (typeof v === "boolean" ? (v ? "ON" : "OFF") : String(v ?? ""));

/** 適用前後の差分(変わる項目だけ)。 */
export function diffRows(params: GenerationParams, changes: SuggestedChanges): DiffRow[] {
  const rows: DiffRow[] = [];
  for (const key of SCALAR_KEYS) {
    const to = changes[key];
    if (to === undefined || to === params[key]) continue;
    rows.push({ field: key, from: show(params[key]), to: show(to) });
  }
  const face = changes.faceDetailer;
  if (face) {
    const cur = params.faceDetailer ?? defaultFaceDetailer();
    const curEnabled = params.faceDetailer?.enabled ?? false;
    if (face.enabled !== undefined && face.enabled !== curEnabled) {
      rows.push({ field: "faceEnabled", from: show(curEnabled), to: show(face.enabled) });
    }
    if (face.denoise !== undefined && face.denoise !== cur.denoise) {
      rows.push({ field: "faceDenoise", from: show(cur.denoise), to: show(face.denoise) });
    }
  }
  return rows;
}

/** 提案された値の一覧(現在のフォームとの比較なし。適用済みの提案の表示用)。 */
export function listChanges(changes: SuggestedChanges): { field: DiffRow["field"]; to: string }[] {
  const rows: { field: DiffRow["field"]; to: string }[] = [];
  for (const key of SCALAR_KEYS) {
    if (changes[key] !== undefined) rows.push({ field: key, to: show(changes[key]) });
  }
  if (changes.faceDetailer?.enabled !== undefined) rows.push({ field: "faceEnabled", to: show(changes.faceDetailer.enabled) });
  if (changes.faceDetailer?.denoise !== undefined) rows.push({ field: "faceDenoise", to: show(changes.faceDetailer.denoise) });
  return rows;
}

/**
 * 変更を適用した結果と、元に戻すための変更(適用した項目の元の値だけ)を返す。
 * 元に戻すのは、その項目だけなので、適用後に手で編集した他の項目には影響しない。
 */
export function applyChanges(
  params: GenerationParams,
  changes: SuggestedChanges,
): { next: GenerationParams; inverse: SuggestedChanges } {
  const next: GenerationParams = { ...params };
  const inverse: SuggestedChanges = {};
  for (const key of SCALAR_KEYS) {
    const v = changes[key];
    if (v === undefined) continue;
    (next as unknown as Record<string, unknown>)[key] = v;
    (inverse as Record<string, unknown>)[key] = params[key];
  }
  if (changes.faceDetailer) {
    const base = params.faceDetailer ?? { ...defaultFaceDetailer(), enabled: false };
    next.faceDetailer = { ...base, ...changes.faceDetailer };
    const inv: NonNullable<SuggestedChanges["faceDetailer"]> = {};
    if (changes.faceDetailer.enabled !== undefined) inv.enabled = base.enabled;
    if (changes.faceDetailer.denoise !== undefined) inv.denoise = base.denoise;
    inverse.faceDetailer = inv;
  }
  return { next, inverse };
}
