import { beforeEach, describe, expect, it, vi } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";
import type { Job } from "@/lib/tauri/jobs";

import { sortJobs, useJobStore } from "./job-store";

const job = (id: string, over: Partial<Job> = {}): Job => ({
  id,
  status: "queued",
  position: 0,
  projectId: null,
  params: defaultParams("sdxl"),
  imageIds: [],
  createdAt: 0,
  ...over,
});

beforeEach(() => {
  useJobStore.setState({ jobs: [], progress: {}, loaded: false });
});

describe("sortJobs", () => {
  it("実行中 → 待機(position 昇順)→ 終了(新しい順)", () => {
    const sorted = sortJobs([
      job("old", { status: "completed", createdAt: 1 }),
      job("q2", { position: 5 }),
      job("new", { status: "failed", createdAt: 9 }),
      job("run", { status: "running" }),
      job("q1", { position: 2 }),
    ]);
    expect(sorted.map((j) => j.id)).toEqual(["run", "q1", "q2", "new", "old"]);
  });
});

describe("job-store", () => {
  it("更新イベントを id で畳み込み、並び順を保つ", () => {
    const { applyUpdated } = useJobStore.getState();
    applyUpdated(job("a", { position: 1 }));
    applyUpdated(job("b", { position: 0 }));
    applyUpdated(job("a", { position: 1, status: "running" }));
    expect(useJobStore.getState().jobs.map((j) => [j.id, j.status])).toEqual([
      ["a", "running"],
      ["b", "queued"],
    ]);
  });

  it("実行中でなくなったジョブの進捗は消える", () => {
    const s = useJobStore.getState();
    s.applyUpdated(job("a", { status: "running" }));
    s.applyProgress({ jobId: "a", step: 3, totalSteps: 20 });
    expect(useJobStore.getState().progress.a.step).toBe(3);
    s.applyUpdated(job("a", { status: "completed" }));
    expect(useJobStore.getState().progress.a).toBeUndefined();
  });
});

const api = vi.hoisted(() => ({
  jobList: vi.fn(),
  jobRemove: vi.fn(),
  jobRetry: vi.fn(),
  queuePause: vi.fn(),
  queueResume: vi.fn(),
  queueState: vi.fn(),
}));
vi.mock("@/lib/tauri/jobs", async (orig) => ({ ...(await orig<typeof import("@/lib/tauri/jobs")>()), ...api }));

describe("job-store(キュー管理。docs/20 §3)", () => {
  beforeEach(() => {
    Object.values(api).forEach((m) => m.mockReset());
    useJobStore.setState({ jobs: [], progress: {}, loaded: false, paused: false, nextCursor: null, loadingMore: false });
  });

  it("履歴は cursor で続きを読み込み、重複は畳み込む", async () => {
    api.jobList
      .mockResolvedValueOnce({ items: [job("a", { status: "completed", createdAt: 9 })], nextCursor: "c1" })
      .mockResolvedValueOnce({
        items: [job("a", { status: "completed", createdAt: 9 }), job("b", { status: "completed", createdAt: 1 })],
      });
    await useJobStore.getState().load();
    expect(useJobStore.getState().nextCursor).toBe("c1");
    await useJobStore.getState().loadMore();
    expect(useJobStore.getState().jobs.map((j) => j.id)).toEqual(["a", "b"]);
    expect(useJobStore.getState().nextCursor).toBeNull();
    expect(api.jobList).toHaveBeenLastCalledWith({ limit: 200, cursor: "c1" });
    // これ以上は読み込まない
    await useJobStore.getState().loadMore();
    expect(api.jobList).toHaveBeenCalledTimes(2);
  });

  it("一時停止・再開は IPC を呼び、状態を保つ。イベントでも更新される", async () => {
    await useJobStore.getState().setPaused(true);
    expect(api.queuePause).toHaveBeenCalled();
    expect(useJobStore.getState().paused).toBe(true);
    await useJobStore.getState().setPaused(false);
    expect(api.queueResume).toHaveBeenCalled();
    expect(useJobStore.getState().paused).toBe(false);
    useJobStore.getState().applyQueueState(true);
    expect(useJobStore.getState().paused).toBe(true);
    api.queueState.mockResolvedValueOnce({ paused: false });
    await useJobStore.getState().loadQueueState();
    expect(useJobStore.getState().paused).toBe(false);
  });

  it("履歴から外すと、一覧からも消える。失敗したときは一覧を変えない", async () => {
    useJobStore.setState({ jobs: [job("a", { status: "completed" }), job("b", { status: "failed" })] });
    api.jobRemove.mockResolvedValueOnce(1);
    await useJobStore.getState().remove(["a"]);
    expect(api.jobRemove).toHaveBeenCalledWith(["a"]);
    expect(useJobStore.getState().jobs.map((j) => j.id)).toEqual(["b"]);
    api.jobRemove.mockRejectedValueOnce({ code: "VALIDATION", message: "x" });
    await expect(useJobStore.getState().remove(["b"])).rejects.toBeDefined();
    expect(useJobStore.getState().jobs.map((j) => j.id)).toEqual(["b"]);
  });

  it("再試行は新しいジョブを投入し、元の失敗カードを外す。投入に失敗したら外さない(docs/28 §8.2)", async () => {
    useJobStore.setState({ jobs: [job("f", { status: "failed" })] });
    api.jobRetry.mockResolvedValueOnce(job("n", { position: 1 }));
    api.jobRemove.mockResolvedValueOnce(1);
    await useJobStore.getState().retry("f");
    expect(api.jobRemove).toHaveBeenCalledWith(["f"]);
    expect(useJobStore.getState().jobs.map((j) => j.id)).toEqual(["n"]);

    useJobStore.setState({ jobs: [job("g", { status: "failed" })] });
    api.jobRetry.mockRejectedValueOnce({ code: "X", message: "x" });
    await expect(useJobStore.getState().retry("g")).rejects.toBeDefined();
    expect(api.jobRemove).toHaveBeenCalledTimes(1);
    expect(useJobStore.getState().jobs.map((j) => j.id)).toEqual(["g"]);
  });
});
