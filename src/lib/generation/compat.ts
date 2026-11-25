import type { ModelKind } from "@/lib/tauri/jobs";
import type { ModelCatalog, ModelFamily } from "@/lib/tauri/models";

/**
 * モデルファイルと生成種別の整合(docs/20 §2)。カタログでファイルの用途が分かるものだけを扱い、
 * カタログにないファイル(手動配置)は判断せず、すべての種別に出す。Rust 側 `services/models/compat.rs` と同じ規則。
 */

/** ファイル名(小文字・パス区切りを除く)→ 使える種別。 */
export type FileKinds = Map<string, Set<ModelKind>>;

/** パスの区切りを問わず、ファイル名だけを小文字で取り出す。 */
export const fileKey = (name: string) => (name.split(/[\\/]/).pop() ?? name).toLowerCase();

function kindsOfFamily(f: ModelFamily, baseModel: string | undefined): ModelKind[] {
  if (f.category === "controlnet" || f.category === "detector") return [];
  if (f.category === "checkpoint") return baseModel === "sd15" || baseModel === "sdxl" ? [baseModel] : [];
  if (f.kind) return [f.kind];
  // 本体以外(テキストエンコーダ・VAE)は、カテゴリの種別で使う。Flux は Flux と Flux GGUF の両方
  if (f.category === "flux") return ["flux", "flux_gguf"];
  return [f.category];
}

/** カタログから、ファイルごとの使える種別を作る。同じファイルが複数のファミリーにあれば、和集合。 */
export function fileKinds(catalog: ModelCatalog | null | undefined): FileKinds {
  const index: FileKinds = new Map();
  for (const f of catalog?.families ?? []) {
    for (const v of f.variants) {
      const kinds = kindsOfFamily(f, v.baseModel);
      if (kinds.length === 0) continue;
      const key = fileKey(v.fileName);
      const set = index.get(key) ?? new Set<ModelKind>();
      for (const k of kinds) set.add(k);
      index.set(key, set);
    }
  }
  return index;
}

/** そのファイルを `kind` で使ってよいか。カタログにないファイルは、判断できないので使ってよい。 */
export function usableFor(index: FileKinds, name: string, kind: ModelKind): boolean {
  const kinds = index.get(fileKey(name));
  return kinds === undefined || kinds.has(kind);
}

/** カタログにあるファイルか(「カタログ外」の判定用)。 */
export const isCatalogued = (index: FileKinds, name: string) => index.has(fileKey(name));

/** `names` から、`kind` で使えるものだけを残す(順序は保つ)。 */
export function filterForKind(names: string[], kind: ModelKind, index: FileKinds): string[] {
  return names.filter((n) => usableFor(index, n, kind));
}
