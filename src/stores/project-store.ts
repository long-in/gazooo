import { create } from "zustand";

import { toAppError } from "@/lib/tauri/errors";
import {
  type Project,
  type ProjectColor,
  projectCreate,
  projectDelete,
  projectGet,
  projectList,
  projectReorder,
  projectUpdate,
} from "@/lib/tauri/projects";

import { useFormStore } from "./form-store";
import { useUiStore } from "./ui-store";

export interface ProjectTotals {
  total: number;
  favorites: number;
  unassigned: number;
}

export interface SwitchResult {
  /** 前回の設定をフォームへ適用したか。 */
  restored: boolean;
  /** 適用を元に戻す(適用したときのみ)。 */
  undo: (() => void) | null;
}

interface ProjectState {
  projects: Project[];
  totals: ProjectTotals;
  loaded: boolean;
  /** 生成ページで選んでいるプロジェクト(null = 未分類。docs/40 §2)。 */
  currentId: string | null;

  /** 一覧と件数を取得する。現在のプロジェクトが無くなっていたら未分類へ戻す。 */
  load: () => Promise<void>;
  /** 保存済みの現在のプロジェクトを取り込む(画面状態の取り込み後に 1 回)。 */
  restoreCurrent: () => void;
  /** 失敗時は `AppError` を throw する(名前の重複は VALIDATION(name))。 */
  create: (name: string, color: ProjectColor | null) => Promise<Project>;
  update: (id: string, name: string, color: ProjectColor | null) => Promise<Project>;
  /** 画像・ジョブは未分類へ戻る。 */
  remove: (id: string) => Promise<number>;
  /** `ids` は全プロジェクトの新しい並び。失敗時は一覧を取り直す。 */
  reorder: (ids: string[]) => Promise<void>;
  /**
   * 現在のプロジェクトを切り替える。前回の設定があればフォームへ適用する(なければ今のフォームを維持。docs/40 §4.1)。
   * 取得に失敗したら、切り替えは取り消して `AppError` を throw する。
   */
  switchTo: (id: string | null) => Promise<SwitchResult>;
}

const emptyTotals: ProjectTotals = { total: 0, favorites: 0, unassigned: 0 };

// 続けて切り替えたとき、遅れて届いた古い応答でフォームを上書きしないための世代番号
let switchGeneration = 0;

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
  totals: emptyTotals,
  loaded: false,
  currentId: null,

  load: async () => {
    try {
      const list = await projectList();
      const { currentId } = get();
      const gone = currentId !== null && !list.projects.some((p) => p.id === currentId);
      set({
        projects: list.projects,
        totals: { total: list.totalImages, favorites: list.favoriteImages, unassigned: list.unassignedImages },
        loaded: true,
        ...(gone ? { currentId: null } : {}),
      });
      if (gone) useUiStore.getState().setCurrentProjectId(null);
    } catch {
      set({ loaded: true });
    }
  },

  restoreCurrent: () => {
    const saved = useUiStore.getState().currentProjectId;
    const { projects } = get();
    // 一覧の取得前は保存値をそのまま採り、取得後の `load` で存在確認する
    set({ currentId: saved !== null && (projects.length === 0 || projects.some((p) => p.id === saved)) ? saved : null });
  },

  create: async (name, color) => {
    const project = await projectCreate(name, color);
    await get().load();
    return project;
  },

  update: async (id, name, color) => {
    const project = await projectUpdate(id, name, color);
    set((s) => ({ projects: s.projects.map((p) => (p.id === id ? project : p)) }));
    return project;
  },

  remove: async (id) => {
    const n = await projectDelete(id);
    if (get().currentId === id) {
      set({ currentId: null });
      useUiStore.getState().setCurrentProjectId(null);
    }
    await get().load();
    return n;
  },

  reorder: async (ids) => {
    const before = get().projects;
    const byId = new Map(before.map((p) => [p.id, p]));
    set({ projects: ids.flatMap((id) => byId.get(id) ?? []) });
    try {
      await projectReorder(ids);
    } catch (e) {
      set({ projects: before });
      throw toAppError(e);
    }
  },

  switchTo: async (id) => {
    const gen = ++switchGeneration;
    const previous = get().currentId;
    const previousParams = useFormStore.getState().params;
    set({ currentId: id });
    useUiStore.getState().setCurrentProjectId(id);
    if (id === null) return { restored: false, undo: null };

    let lastParams;
    try {
      lastParams = (await projectGet(id)).lastParams;
    } catch (e) {
      if (gen === switchGeneration) {
        set({ currentId: previous });
        useUiStore.getState().setCurrentProjectId(previous);
      }
      throw toAppError(e);
    }
    if (gen !== switchGeneration || !lastParams) return { restored: false, undo: null };
    useFormStore.getState().applyPreset(lastParams);
    return { restored: true, undo: () => useFormStore.getState().applyPreset(previousParams) };
  },
}));
