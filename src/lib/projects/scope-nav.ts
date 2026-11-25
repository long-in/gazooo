/** スコープパネル(docs/40 §11)のキー移動・絞り込み・件数表記の純関数。 */

import type { Language } from "@/lib/tauri/settings";

/** プロジェクトの絞り込み欄を出す閾値(件数がこれを超えたとき。§11.2)。 */
export const PROJECT_FILTER_THRESHOLD = 8;

/** 狭いウィンドウとみなす幅(px。§11.4)。 */
export const NARROW_WIDTH = 900;

/** ↑ / ↓ / Home / End による項目間の移動先(端では止まる。対象外のキーは null)。 */
export function moveIndex(index: number, count: number, key: string): number | null {
  if (count <= 0) return null;
  const cur = Math.min(Math.max(index, 0), count - 1);
  switch (key) {
    case "ArrowDown":
      return Math.min(cur + 1, count - 1);
    case "ArrowUp":
      return Math.max(cur - 1, 0);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** 名前の部分一致(大文字小文字を区別しない)で絞り込む。選択中のプロジェクトは一致しなくても残す。 */
export function filterProjects<T extends { id: string; name: string }>(projects: readonly T[], query: string, activeId: string | null): T[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [...projects];
  return projects.filter((p) => p.id === activeId || p.name.toLowerCase().includes(q));
}

/** 件数の表記。1 万未満は桁区切り、1 万以上は短縮(ja = 1.2万 / en = 12k)。 */
export function formatCount(n: number, lang: Language): string {
  if (n < 10_000) return n.toLocaleString("en-US");
  if (lang === "ja") {
    const v = Math.floor(n / 1000) / 10;
    return `${v}万`;
  }
  return `${Math.floor(n / 1000)}k`;
}
