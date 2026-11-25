import { create } from "zustand";

import {
  type GenerationParams,
  type Job,
  type JobProgress,
  jobCancel,
  jobClearFinished,
  jobEnqueue,
  jobList,
  jobRemove,
  jobReorder,
  jobRetry,
  queuePause,
  queueResume,
  queueState,
} from "@/lib/tauri/jobs";

import { useProjectStore } from "./project-store";

/** 履歴を一度に読み込む件数(docs/20 Q-95)。 */
const PAGE_SIZE = 200;

const rank = (j: Job) => (j.status === "running" ? 0 : j.status === "queued" ? 1 : 2);

/** バックエンドの `job_list` と同じ並び: 実行中 → 待機(並び順)→ 終了(新しい順)。 */
export function sortJobs(jobs: Job[]): Job[] {
  return [...jobs].sort((a, b) => {
    const r = rank(a) - rank(b);
    if (r !== 0) return r;
    if (a.status === "queued" && b.status === "queued") return a.position - b.position;
    return b.createdAt - a.createdAt;
  });
}

interface JobState {
  /** 常に `sortJobs` 済み(セレクタが毎回新しい配列を返さないよう、保持する)。 */
  jobs: Job[];
  progress: Record<string, JobProgress>;
  loaded: boolean;
  /** 一時停止中(次のジョブを始めない。docs/20 §3.3)。 */
  paused: boolean;
  /** 履歴の続きを取得するためのカーソル(無ければ最後まで読み込み済み)。 */
  nextCursor: string | null;
  loadingMore: boolean;

  load: () => Promise<void>;
  /** 履歴の続きを読み込む。 */
  loadMore: () => Promise<void>;
  applyQueueState: (paused: boolean) => void;
  loadQueueState: () => Promise<void>;
  /** 一時停止・再開(失敗時は `AppError` を throw)。 */
  setPaused: (paused: boolean) => Promise<void>;
  /** 終了済みジョブを履歴から外す(失敗時は `AppError` を throw)。 */
  remove: (ids: string[]) => Promise<number>;
  applyUpdated: (job: Job) => void;
  applyProgress: (p: JobProgress) => void;

  /** 以下は失敗時に `AppError` を throw する(呼び出し側で表示)。 */
  enqueue: (params: GenerationParams, count: number) => Promise<Job[]>;
  cancel: (id: string) => Promise<void>;
  reorder: (id: string, newPosition: number) => Promise<void>;
  retry: (id: string) => Promise<Job>;
  clearFinished: () => Promise<number>;
}

export const useJobStore = create<JobState>((set, get) => ({
  jobs: [],
  progress: {},
  loaded: false,
  paused: false,
  nextCursor: null,
  loadingMore: false,

  load: async () => {
    try {
      const page = await jobList({ limit: PAGE_SIZE });
      set({ jobs: sortJobs(page.items), loaded: true, nextCursor: page.nextCursor ?? null });
    } catch {
      set({ loaded: true });
    }
  },

  loadMore: async () => {
    const cursor = get().nextCursor;
    if (!cursor || get().loadingMore) return;
    set({ loadingMore: true });
    try {
      const page = await jobList({ limit: PAGE_SIZE, cursor });
      set((s) => {
        const have = new Set(s.jobs.map((j) => j.id));
        return {
          jobs: sortJobs([...s.jobs, ...page.items.filter((j) => !have.has(j.id))]),
          nextCursor: page.nextCursor ?? null,
          loadingMore: false,
        };
      });
    } catch {
      set({ loadingMore: false });
    }
  },

  applyQueueState: (paused) => set({ paused }),

  loadQueueState: async () => {
    try {
      set({ paused: (await queueState()).paused });
    } catch {
      /* 取得できなくても画面は動く */
    }
  },

  setPaused: async (paused) => {
    await (paused ? queuePause() : queueResume());
    set({ paused });
  },

  remove: async (ids) => {
    const n = await jobRemove(ids);
    const gone = new Set(ids);
    set((s) => ({ jobs: s.jobs.filter((j) => !gone.has(j.id)) }));
    return n;
  },

  applyUpdated: (job) =>
    set((s) => {
      const others = s.jobs.filter((j) => j.id !== job.id);
      const progress = { ...s.progress };
      if (job.status !== "running") delete progress[job.id];
      return { jobs: sortJobs([...others, job]), progress };
    }),

  applyProgress: (p) => set((s) => ({ progress: { ...s.progress, [p.jobId]: p } })),

  // 投入結果は `job://updated` でも届くが、重複しても `applyUpdated` が id で畳み込む
  enqueue: async (params, count) => {
    // 生成画像は、選んでいるプロジェクトへ入る(未選択 = 未分類。docs/40 §4.2)
    const jobs = await jobEnqueue(params, count, useProjectStore.getState().currentId);
    jobs.forEach(get().applyUpdated);
    return jobs;
  },

  cancel: (id) => jobCancel(id),

  reorder: (id, newPosition) => jobReorder(id, newPosition),

  retry: async (id) => {
    const job = await jobRetry(id);
    get().applyUpdated(job);
    // 再試行は元の失敗カードを残さない(docs/28 §8.2)。外せなくても再試行自体は成功扱い
    try {
      await get().remove([id]);
    } catch {
      /* 元のカードが残るだけ */
    }
    return job;
  },

  clearFinished: async () => {
    const n = await jobClearFinished();
    await get().load();
    return n;
  },
}));
