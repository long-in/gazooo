import type { GenerationParams } from "./jobs";
import { invoke } from "./invoke";

/** プロジェクトの色(ユーザーカラーと同じ 8 色。docs/40 §5.4)。 */
export const PROJECT_COLORS = ["red", "orange", "amber", "green", "teal", "blue", "violet", "pink"] as const;
export type ProjectColor = (typeof PROJECT_COLORS)[number];

export interface Project {
  id: string;
  name: string;
  color: ProjectColor | null;
  sortOrder: number;
  imageCount: number;
  /** 最新の画像(表紙。無ければ null)。 */
  coverImageId: string | null;
  /** 前回の生成設定があるか(本体は `projectGet`)。 */
  hasLastParams: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface ProjectList {
  projects: Project[];
  totalImages: number;
  favoriteImages: number;
  unassignedImages: number;
}

export interface ProjectDetail {
  project: Project;
  /** 最後にこのプロジェクトで投入した生成設定(無ければ null)。 */
  lastParams: GenerationParams | null;
}

export const projectList = () => invoke<ProjectList>("project_list");
export const projectGet = (id: string) => invoke<ProjectDetail>("project_get", { id });
export const projectCreate = (name: string, color: ProjectColor | null = null) =>
  invoke<Project>("project_create", { name, color });
/** 名前と色を置き換える(`color` null = なし)。 */
export const projectUpdate = (id: string, name: string, color: ProjectColor | null) =>
  invoke<Project>("project_update", { id, name, color });
export const projectReorder = (ids: string[]) => invoke<void>("project_reorder", { ids });
/** 画像・ジョブは消えず未分類へ戻る。戻った画像数を返す。 */
export const projectDelete = (id: string) => invoke<number>("project_delete", { id });
