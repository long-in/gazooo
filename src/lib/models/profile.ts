import {
  modelsFor,
  supportsControlnet,
  supportsFaceDetailer,
  supportsImg2img,
  supportsInpaint,
  usesNegative,
} from "@/lib/generation/defaults";
import type { EngineModels } from "@/lib/tauri/engine";
import type { GenerationParams, ModelKind } from "@/lib/tauri/jobs";
import type { ModelCatalog, ModelFamily, ModelProfile } from "@/lib/tauri/models";

/**
 * モデルの説明(docs/18)。カタログのファミリーの `profile` を、生成画面で選ばれているモデルと結び付ける。
 * 対応機能(顔補正・ControlNet・Inpaint・ネガティブ)は、データに持たず種別から計算する(docs/18 §3)。
 */

/** カタログ外のモデル(手動で置いたファイル)に出す、種別の一般的な説明(docs/18 §4)。 */
const GENERIC_FAMILY: Record<ModelKind, string> = {
  sd15: "checkpoint-sd15",
  sdxl: "checkpoint-sdxl",
  flux: "flux-schnell-gguf",
  flux_gguf: "flux-schnell-gguf",
  anima: "anima-base",
  qwen_image: "qwen-image-2-1",
  krea2: "krea2-turbo",
};

/** 生成画面のモデル選択に出るファミリー(チェックポイント、Flux の拡散モデル本体)。 */
export const isSelectableFamily = (f: ModelFamily) => f.category === "checkpoint" || f.kind !== undefined;

/** そのファミリーを使うときの生成画面の種別(複数のベースモデルがあれば複数)。 */
export function familyKinds(f: ModelFamily): ModelKind[] {
  if (f.kind) return [f.kind];
  const kinds = new Set<ModelKind>();
  for (const v of f.variants) if (v.baseModel) kinds.add(v.baseModel);
  return [...kinds];
}

const lower = (s: string) => s.toLowerCase();

/** ファイル名がカタログの `fileName` と一致するファミリー(大文字小文字は区別しない)。 */
export function familyByFileName(catalog: ModelCatalog | null | undefined, fileName: string): ModelFamily | undefined {
  if (!catalog || fileName === "") return undefined;
  const key = lower(fileName);
  return catalog.families.find((f) => isSelectableFamily(f) && f.variants.some((v) => lower(v.fileName) === key));
}

export interface ProfileMatch {
  family: ModelFamily;
  profile: ModelProfile;
  /** true = ファイル名がカタログと一致。false = カタログ外で、種別の一般説明を出している。 */
  exact: boolean;
}

/** 選択中のモデルの説明。カタログが無い・プロフィールが無いときは `null`。 */
export function profileFor(
  catalog: ModelCatalog | null | undefined,
  kind: ModelKind,
  fileName: string,
): ProfileMatch | null {
  if (!catalog) return null;
  const hit = familyByFileName(catalog, fileName);
  if (hit?.profile && familyKinds(hit).includes(kind)) return { family: hit, profile: hit.profile, exact: true };
  const generic = catalog.families.find((f) => f.id === GENERIC_FAMILY[kind]);
  return generic?.profile ? { family: generic, profile: generic.profile, exact: false } : null;
}

/** 対応機能(○ / ×)。種別から決まる。 */
export function featuresFor(kind: ModelKind) {
  return {
    negative: usesNegative(kind),
    faceDetailer: supportsFaceDetailer(kind),
    controlnet: supportsControlnet(kind),
    inpaint: supportsInpaint(kind),
    img2img: supportsImg2img(kind),
  };
}

/** 導入済み(ComfyUI から選べる)ファミリーの ID。AI サポートへ送るのは、このファミリー ID だけ(docs/18 §6.2)。 */
export function installedFamilyIds(catalog: ModelCatalog | null | undefined, models: EngineModels | null): string[] {
  if (!catalog || !models) return [];
  const have = new Set([...models.checkpoints, ...models.ggufUnets, ...models.unets].map(lower));
  return catalog.families
    .filter((f) => isSelectableFamily(f) && f.variants.some((v) => have.has(lower(v.fileName))))
    .map((f) => f.id);
}

/** AI サポートに添える、選択中・導入済みのモデル(ファミリー ID のみ。ファイル名・パスは含めない)。 */
export function modelContext(
  catalog: ModelCatalog | null | undefined,
  params: GenerationParams,
  models: EngineModels | null,
): { familyId: string | null; installedFamilies: string[] } | undefined {
  if (!catalog) return undefined;
  const match = profileFor(catalog, params.model.kind, params.model.name);
  return {
    familyId: match?.exact ? match.family.id : null,
    installedFamilies: installedFamilyIds(catalog, models),
  };
}

/** 「推奨設定を適用」で変える項目(プロンプトは変えない)。 */
export type RecommendedPatch = Pick<GenerationParams, "width" | "height" | "steps" | "cfg"> &
  Partial<Pick<GenerationParams, "sampler" | "scheduler">>;

export function recommendedPatch(profile: ModelProfile): RecommendedPatch {
  const r = profile.recommended;
  return {
    width: r.size.width,
    height: r.size.height,
    steps: r.steps,
    cfg: r.cfg,
    ...(r.sampler && { sampler: r.sampler }),
    ...(r.scheduler && { scheduler: r.scheduler }),
  };
}

/**
 * ファミリーへ切り替えるときの種別とファイル名。導入済みの variant(推奨を優先)が無ければ `null`
 * (未導入なら、モデル取得ページへ案内する)。
 */
export function switchTarget(
  family: ModelFamily,
  models: EngineModels | null,
): { kind: ModelKind; name: string } | null {
  if (!models) return null;
  const variants = [...family.variants].sort((a, b) => Number(b.recommended) - Number(a.recommended));
  for (const kind of familyKinds(family)) {
    const list = modelsFor(kind, models);
    for (const v of variants) {
      if (v.baseModel && family.category === "checkpoint" && v.baseModel !== kind) continue;
      const found = list.find((n) => lower(n) === lower(v.fileName));
      if (found) return { kind, name: found };
    }
  }
  return null;
}

/** 候補(ファイル名)に付ける短い補足のバッジ(docs/18 §5)。カタログ外は `uncatalogued`。 */
export type CandidateBadge = "anime" | "realistic" | "moderation-strict" | "moderation-moderate" | "moderation-loose" | "heavy" | "uncatalogued";

export function candidateBadges(catalog: ModelCatalog | null | undefined, fileName: string): CandidateBadge[] {
  const f = familyByFileName(catalog, fileName);
  if (!f?.profile) return catalog ? ["uncatalogued"] : [];
  const p = f.profile;
  const out: CandidateBadge[] = [];
  if (p.style.includes("anime") || p.style.includes("illustration")) out.push("anime");
  if (p.style.includes("realistic")) out.push("realistic");
  if (p.moderation.level !== "unknown") out.push(`moderation-${p.moderation.level}`);
  if (p.resources.speed === "slow" || p.resources.vramGb >= 12) out.push("heavy");
  return out;
}
