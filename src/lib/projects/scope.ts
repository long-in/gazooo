/** ギャラリーのスコープ(見る範囲。docs/40 §5.1)の純関数。 */

import type { ImageScopeArg } from "@/lib/tauri/images";

/** スコープパネルの項目。`favorites` = 「すべて」+ お気に入りのみ(ショートカット)。 */
export type ScopeEntry = "all" | "favorites" | "unassigned" | { projectId: string };

export interface ScopeState {
  scope: ImageScopeArg;
  favoritesOnly: boolean;
}

/** パネルの項目を選んだときの状態。別の項目へ移るときは、お気に入りの絞り込みを解除する。 */
export function selectEntry(entry: ScopeEntry): ScopeState {
  if (entry === "favorites") return { scope: "all", favoritesOnly: true };
  return { scope: entry, favoritesOnly: false };
}

/** 現在の状態が、パネルのどの項目に当たるか(強調表示用)。プロジェクト内のお気に入り絞り込みは、そのプロジェクトのまま。 */
export function activeEntry(state: ScopeState): ScopeEntry {
  if (state.scope === "all") return state.favoritesOnly ? "favorites" : "all";
  return state.scope;
}

export function sameEntry(a: ScopeEntry, b: ScopeEntry): boolean {
  if (typeof a === "string" || typeof b === "string") return a === b;
  return a.projectId === b.projectId;
}

/** 保存用の文字列("all" / "favorites" / "unassigned" / "project:<id>")。 */
export function encodeEntry(entry: ScopeEntry): string {
  return typeof entry === "string" ? entry : `project:${entry.projectId}`;
}

/** 保存値の復元。壊れた値・未知の値は「すべて」(起動不能にしない)。 */
export function decodeEntry(raw: unknown): ScopeEntry {
  if (raw === "favorites" || raw === "unassigned" || raw === "all") return raw;
  if (typeof raw === "string" && raw.startsWith("project:") && raw.length > "project:".length) {
    return { projectId: raw.slice("project:".length) };
  }
  return "all";
}

/** 画像の所属がスコープに合うか。 */
export function scopeMatches(scope: ImageScopeArg, projectId: string | null): boolean {
  if (scope === "all") return true;
  if (scope === "unassigned") return projectId === null;
  return scope.projectId === projectId;
}

/** スコープの指す存在しないプロジェクトを「すべて」へ戻す(削除された・別ユーザーの保存値)。 */
export function validEntry(entry: ScopeEntry, projectIds: ReadonlySet<string>): ScopeEntry {
  return typeof entry === "object" && !projectIds.has(entry.projectId) ? "all" : entry;
}

/** 生成ページの帯のスコープ: 「このプロジェクト」= 現在のプロジェクト(なし = 未分類)、「すべて」= 全画像。 */
export function stripScope(allProjects: boolean, currentProjectId: string | null): ImageScopeArg {
  if (allProjects) return "all";
  return currentProjectId ? { projectId: currentProjectId } : "unassigned";
}
