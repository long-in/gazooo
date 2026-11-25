import type { Language } from "@/lib/tauri/settings";
import type {
  BaseModel,
  CatalogView,
  DownloadState,
  DownloadStatus,
  Localized,
  ModelCategory,
  ModelTag,
  ModelFamily,
  ModelVariant,
} from "@/lib/tauri/models";

/** ベースモデルによる絞り込み。`all` は絞り込まない。 */
export type BaseFilter = "all" | BaseModel;

export const CATEGORIES: ModelCategory[] = ["checkpoint", "controlnet", "detector", "flux", "anima", "qwen_image", "krea2"];

/** 「まとめて取得」の 1 件(本体、またはそれに必要なテキストエンコーダ・VAE)。 */
export interface BundleItem {
  family: ModelFamily;
  variant: ModelVariant;
  installed: boolean;
}

/**
 * ファミリーの選択中の variant と、その `requires`(テキストエンコーダ・VAE)を、取得する順に並べる。
 * `requires` が無いファミリーは空(まとめて取得の対象ではない)。参照先がカタログに無いものは飛ばす。
 */
export function bundleFor(
  families: ModelFamily[],
  family: ModelFamily,
  selected: ModelVariant,
  isInstalled: (familyId: string, variantId: string) => boolean,
): BundleItem[] {
  if (!family.requires || family.requires.length === 0) return [];
  const items: BundleItem[] = [{ family, variant: selected, installed: isInstalled(family.id, selected.id) }];
  for (const r of family.requires) {
    const f = families.find((x) => x.id === r.familyId);
    const v = f?.variants.find((x) => x.id === r.variantId);
    if (f && v) items.push({ family: f, variant: v, installed: isInstalled(f.id, v.id) });
  }
  return items;
}

/** 未導入の分の合計サイズ(同じファイルが重複して数えられないよう、配置先とファイル名で 1 つにまとめる)。 */
export function bundleMissingBytes(items: BundleItem[]): number {
  const seen = new Set<string>();
  let total = 0;
  for (const it of items) {
    if (it.installed) continue;
    const key = `${it.variant.destDir}/${it.variant.fileName.toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    total += it.variant.sizeBytes;
  }
  return total;
}

export const variantKey = (familyId: string, variantId: string) => `${familyId}/${variantId}`;

export const localized = (l: Localized, lang: Language) => l[lang];

export function isCategory(v: string | null): v is ModelCategory {
  return v !== null && (CATEGORIES as string[]).includes(v);
}

export function isBase(v: string | null): v is BaseModel {
  return v === "sd15" || v === "sdxl";
}

/** 用途による絞り込み(docs/16 §3.3、Q-61)。`all` は絞り込まない。チェックポイントのタブだけで使う。 */
export type PurposeFilter = "all" | ModelTag;

export const PURPOSES: PurposeFilter[] = ["all", "anime"];

export function isPurpose(v: string | null): v is PurposeFilter {
  return v !== null && (PURPOSES as string[]).includes(v);
}

export function matchesPurpose(f: ModelFamily, purpose: PurposeFilter): boolean {
  return purpose === "all" || f.tags.includes(purpose);
}

/** ベースモデルを持たない variant(検出モデルなど)は、どの絞り込みでも表示する。 */
export function matchesBase(v: ModelVariant, base: BaseFilter): boolean {
  return base === "all" || v.baseModel === undefined || v.baseModel === base;
}

/**
 * カテゴリ・ベースモデル・用途で絞り込む。表示できる variant が無いファミリーは除く。
 * 用途フィルタはチェックポイントのタブだけで効く(他のタブでは常に全件。docs/16 Q-61)。
 */
export function visibleFamilies(
  families: ModelFamily[],
  category: ModelCategory,
  base: BaseFilter,
  purpose: PurposeFilter = "all",
): { family: ModelFamily; variants: ModelVariant[] }[] {
  return families
    .filter((f) => f.category === category)
    .filter((f) => category !== "checkpoint" || matchesPurpose(f, purpose))
    .map((family) => ({ family, variants: family.variants.filter((v) => matchesBase(v, base)) }))
    .filter((x) => x.variants.length > 0);
}

/**
 * 選択中の variant。ユーザーが選んだものが表示中ならそれ、なければ推奨(なければ先頭)。
 * `variants` は絞り込み後の一覧。
 */
export function pickVariant(variants: ModelVariant[], chosenId: string | undefined): ModelVariant {
  return (
    variants.find((v) => v.id === chosenId) ??
    variants.find((v) => v.recommended) ??
    variants[0]
  );
}

export function installedMap(view: CatalogView | null): Map<string, { installed: boolean; sizeMatches: boolean }> {
  const m = new Map<string, { installed: boolean; sizeMatches: boolean }>();
  for (const s of view?.installed ?? []) {
    m.set(variantKey(s.familyId, s.variantId), { installed: s.installed, sizeMatches: s.sizeMatches });
  }
  return m;
}

export const isActiveStatus = (s: DownloadStatus) =>
  s === "queued" || s === "downloading" || s === "verifying";

/** 同じモデルの、いちばん新しいダウンロード(取得中を優先)。 */
export function downloadFor(
  downloads: DownloadState[],
  familyId: string,
  variantId: string,
): DownloadState | undefined {
  const mine = downloads.filter((d) => d.familyId === familyId && d.variantId === variantId);
  return mine.find((d) => isActiveStatus(d.status)) ?? mine[mine.length - 1];
}

export function progressPercent(d: Pick<DownloadState, "receivedBytes" | "totalBytes">): number {
  if (d.totalBytes <= 0) return 0;
  return Math.min(100, Math.floor((d.receivedBytes / d.totalBytes) * 100));
}

/** バイト数を読みやすく(1000 進。ファイルサイズの表示に一般的な 1GB = 10^9 に合わせる)。 */
export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(n >= 1e10 ? 0 : 1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

export const formatSpeed = (bytesPerSec: number) => `${formatBytes(bytesPerSec)}/s`;

/** 残り時間(秒)。速度が分からなければ null。 */
export function etaSeconds(d: DownloadState): number | null {
  if (!d.bytesPerSec || d.bytesPerSec <= 0) return null;
  return Math.ceil((d.totalBytes - d.receivedBytes) / d.bytesPerSec);
}

export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}
