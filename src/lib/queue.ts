import type { GenerationParams, Job, JobProgress, JobStatus } from "@/lib/tauri/jobs";

/** キュー画面のフィルタ(docs/20 §3.1)。 */
export type QueueTab = "all" | "active" | "completed" | "failed";

export const QUEUE_TABS: QueueTab[] = ["all", "active", "completed", "failed"];

const TAB_STATUSES: Record<QueueTab, JobStatus[] | null> = {
  all: null,
  active: ["running", "queued"],
  completed: ["completed"],
  failed: ["failed", "cancelled"],
};

export const isActiveJob = (j: Job) => j.status === "running" || j.status === "queued";
export const isFinishedJob = (j: Job) => !isActiveJob(j);

/** タブごとの件数。 */
export function countByTab(jobs: Job[]): Record<QueueTab, number> {
  const n: Record<QueueTab, number> = { all: jobs.length, active: 0, completed: 0, failed: 0 };
  for (const j of jobs) {
    if (isActiveJob(j)) n.active += 1;
    else if (j.status === "completed") n.completed += 1;
    else n.failed += 1;
  }
  return n;
}

/** プロンプト・ネガティブ・モデル名に、空白区切りのすべての語が含まれる(大文字小文字は区別しない)。 */
export function matchesQuery(params: GenerationParams, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const text = `${params.prompt}\n${params.negativePrompt}\n${params.model.name}`.toLowerCase();
  return words.every((w) => text.includes(w));
}

/** タブと検索語で絞り込む(並びは保つ)。 */
export function filterJobs(jobs: Job[], tab: QueueTab, query: string): Job[] {
  const statuses = TAB_STATUSES[tab];
  return jobs.filter((j) => (statuses === null || statuses.includes(j.status)) && matchesQuery(j.params, query));
}

/** 所要時間(ミリ秒)。開始・終了が揃っていなければ `null`。 */
export function durationMs(job: Job): number | null {
  if (job.startedAt === undefined || job.finishedAt === undefined) return null;
  const d = job.finishedAt - job.startedAt;
  return d >= 0 ? d : null;
}

/** 「1:05」「1:02:03」の形式。 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** 実行中ジョブの経過時間と、ステップの進み具合からの残り時間の目安。進んでいなければ残りは `null`。 */
export function runningTiming(
  job: Job,
  progress: JobProgress | undefined,
  now: number,
): { elapsedMs: number; remainingMs: number | null } {
  const elapsedMs = job.startedAt !== undefined ? Math.max(0, now - job.startedAt) : 0;
  if (!progress || progress.totalSteps <= 0 || progress.step <= 0 || progress.step >= progress.totalSteps) {
    return { elapsedMs, remainingMs: null };
  }
  const perStep = elapsedMs / progress.step;
  return { elapsedMs, remainingMs: perStep * (progress.totalSteps - progress.step) };
}

const median = (xs: number[]) => {
  const s = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 === 1 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
};

/**
 * 同じ種別で、画面の面積が近い(0.7〜1.4 倍)完了ジョブの所要時間の中央値(ミリ秒)から、
 * 1 枚あたりの目安を出す(docs/20 §4.2)。実績が無ければ `null`。
 */
export function estimateMsPerImage(jobs: Job[], params: GenerationParams): number | null {
  const area = params.width * params.height;
  const samples = jobs
    .filter((j) => j.status === "completed" && j.params.model.kind === params.model.kind)
    .filter((j) => {
      const a = j.params.width * j.params.height;
      return a >= area * 0.7 && a <= area * 1.4;
    })
    .map((j) => ({ d: durationMs(j), n: Math.max(1, j.params.batchSize) }))
    .filter((x): x is { d: number; n: number } => x.d !== null && x.d > 0)
    .map((x) => x.d / x.n);
  return samples.length === 0 ? null : median(samples);
}
