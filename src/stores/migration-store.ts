import { create } from "zustand";

import { type AppError, toAppError } from "@/lib/tauri/errors";
import {
  type MigrationPlan,
  type MigrationProgress,
  type MigrationReport,
  type MigrationTarget,
  storageMigrationCancel,
  storageMigrationPlan,
  storageMigrationStart,
} from "@/lib/tauri/storage";

/** 移行先を選ぶ → 内容を確認 → 実行中 → 結果(docs/51 §3.2)。 */
export type MigrationStep = "closed" | "pick" | "planning" | "confirm" | "running" | "done";

interface MigrationState {
  step: MigrationStep;
  target: MigrationTarget;
  /** null = 個別なら全体設定に戻す、全体なら標準値に戻す。 */
  newDir: string | null;
  plan: MigrationPlan | null;
  progress: MigrationProgress | null;
  report: MigrationReport | null;
  error: AppError | null;
  cancelling: boolean;
  /** 実行中にアプリを閉じようとして、確認を出している。 */
  quitAsked: boolean;

  /** 移行先の選択から始める。 */
  open: (target: MigrationTarget) => void;
  /** 移行先を決めて、内容を確認する。 */
  choose: (newDir: string | null) => Promise<void>;
  /** 成功したら結果を返す。 */
  start: () => Promise<MigrationReport | null>;
  cancel: () => Promise<void>;
  setProgress: (progress: MigrationProgress) => void;
  askQuit: (asked: boolean) => void;
  /** 実行中は閉じない。 */
  close: () => void;
}

const initial = {
  newDir: null,
  plan: null,
  progress: null,
  report: null,
  error: null,
  cancelling: false,
  quitAsked: false,
};

export const useMigrationStore = create<MigrationState>((set, get) => ({
  step: "closed",
  target: "imagesUser",
  ...initial,

  open: (target) => {
    if (get().step === "running") return;
    set({ ...initial, step: "pick", target });
  },

  choose: async (newDir) => {
    const { target, step } = get();
    if (step === "running") return;
    set({ step: "planning", newDir, plan: null, error: null });
    // 確認中に閉じた・選び直した場合は反映しない
    const current = () => get().step === "planning" && get().target === target && get().newDir === newDir;
    try {
      const plan = await storageMigrationPlan(target, newDir);
      if (current()) set({ step: "confirm", plan });
    } catch (e) {
      if (current()) set({ step: "pick", error: toAppError(e) });
    }
  },

  start: async () => {
    const { target, newDir, step, plan } = get();
    if (step !== "confirm" || !plan || plan.blockers.length > 0) return null;
    set({ step: "running", progress: null, error: null, cancelling: false });
    try {
      const report = await storageMigrationStart(target, newDir);
      set({ step: "done", report, quitAsked: false });
      return report;
    } catch (e) {
      const error = toAppError(e);
      // 状況が変わっている可能性があるので、内容を確認し直してから理由を示す
      set({ step: "confirm", cancelling: false, quitAsked: false });
      await get().choose(newDir);
      if (get().step !== "closed") set({ error });
      return null;
    }
  },

  cancel: async () => {
    if (get().step !== "running") return;
    set({ cancelling: true });
    await storageMigrationCancel();
  },

  setProgress: (progress) => {
    if (get().step === "running") set({ progress });
  },

  askQuit: (quitAsked) => set({ quitAsked }),

  close: () => {
    if (get().step !== "running") set({ ...initial, step: "closed" });
  },
}));
