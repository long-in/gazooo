import { describe, expect, it } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";
import type { Job } from "@/lib/tauri/jobs";

import {
  countByTab,
  durationMs,
  estimateMsPerImage,
  filterJobs,
  formatDuration,
  runningTiming,
} from "./queue";

const job = (id: string, over: Partial<Job> = {}, p: Partial<Job["params"]> = {}): Job => ({
  id,
  status: "queued",
  position: 0,
  projectId: null,
  params: { ...defaultParams("sdxl"), prompt: id, ...p },
  imageIds: [],
  createdAt: 0,
  ...over,
});

describe("フィルタと件数", () => {
  const jobs = [
    job("run", { status: "running" }),
    job("q", { status: "queued" }),
    job("done", { status: "completed" }),
    job("bad", { status: "failed" }),
    job("cx", { status: "cancelled" }),
  ];

  it("タブごとの件数", () => {
    expect(countByTab(jobs)).toEqual({ all: 5, active: 2, completed: 1, failed: 2 });
  });

  it("タブで絞り込み、並びは保つ", () => {
    expect(filterJobs(jobs, "active", "").map((j) => j.id)).toEqual(["run", "q"]);
    expect(filterJobs(jobs, "failed", "").map((j) => j.id)).toEqual(["bad", "cx"]);
    expect(filterJobs(jobs, "all", "")).toHaveLength(5);
  });

  it("検索はプロンプト・ネガティブ・モデル名に、全部の語が含まれるもの(大文字小文字は区別しない)", () => {
    const list = [
      job("a", {}, { prompt: "A Red Apple", negativePrompt: "blurry" }),
      job("b", {}, { prompt: "blue sky", model: { ...defaultParams("sdxl").model, name: "Animagine.safetensors" } }),
    ];
    expect(filterJobs(list, "all", "red apple").map((j) => j.id)).toEqual(["a"]);
    expect(filterJobs(list, "all", "BLURRY").map((j) => j.id)).toEqual(["a"]);
    expect(filterJobs(list, "all", "animagine sky").map((j) => j.id)).toEqual(["b"]);
    expect(filterJobs(list, "all", "red sky")).toEqual([]);
  });
});

describe("時間", () => {
  it("所要時間と表示", () => {
    expect(durationMs(job("a", { startedAt: 1000, finishedAt: 66_000 }))).toBe(65_000);
    expect(durationMs(job("a", { startedAt: 1000 }))).toBeNull();
    expect(formatDuration(65_000)).toBe("1:05");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(formatDuration(-5)).toBe("0:00");
  });

  it("実行中の経過と、ステップ進捗からの残り時間", () => {
    const j = job("r", { status: "running", startedAt: 0 });
    const t = runningTiming(j, { jobId: "r", step: 10, totalSteps: 40 }, 20_000);
    expect(t).toEqual({ elapsedMs: 20_000, remainingMs: 60_000 });
    // 進んでいない・完了間際は残りを出さない
    expect(runningTiming(j, undefined, 5000).remainingMs).toBeNull();
    expect(runningTiming(j, { jobId: "r", step: 0, totalSteps: 40 }, 5000).remainingMs).toBeNull();
    expect(runningTiming(j, { jobId: "r", step: 40, totalSteps: 40 }, 5000).remainingMs).toBeNull();
  });
});

describe("目安時間(過去の実績)", () => {
  const done = (id: string, ms: number, over: Partial<Job["params"]> = {}) =>
    job(id, { status: "completed", startedAt: 0, finishedAt: ms }, over);

  it("同じ種別・近いサイズの完了ジョブの中央値(1 ジョブの枚数で割る)", () => {
    const p = { ...defaultParams("sdxl"), width: 1024, height: 1024 };
    const jobs = [
      done("a", 40_000, { width: 1024, height: 1024 }),
      done("b", 60_000, { width: 1024, height: 1024 }),
      done("c", 100_000, { width: 1024, height: 1024 }),
      done("d", 90_000, { width: 1024, height: 1024, batchSize: 3 }), // 30 秒/枚
      done("e", 1000, { width: 256, height: 256 }), // サイズが違う
      job("f", { status: "failed", startedAt: 0, finishedAt: 5 }), // 完了ではない
      done("g", 1000, { model: { ...defaultParams("sd15").model } }), // 種別が違う
    ];
    // 40, 60, 100, 30 秒 → 中央値 50 秒
    expect(estimateMsPerImage(jobs, p)).toBe(50_000);
    expect(estimateMsPerImage([], p)).toBeNull();
  });
});
