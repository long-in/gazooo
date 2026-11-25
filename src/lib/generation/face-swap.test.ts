import { describe, expect, it } from "vitest";

import { defaultParams } from "./defaults";
import {
  addTargets,
  defaultFaceSwap,
  FACE_SWAP_PROMPT,
  FACE_SWAP_STEPS,
  estimateJobSeconds,
  estimateMinutes,
  faceSwapJobs,
  faceSwapReady,
  hasFaceSwapPrompt,
  isSmallFaceImage,
  MAX_FACE_SWAP_TARGETS,
  qualityOf,
  supportsFaceSwap,
  targetsFromParams,
  withFaceSwap,
  withFaceSwapEnabled,
} from "./face-swap";
import { defaultReference, withReferenceEnabled } from "./reference";

const qwen = () => defaultParams("qwen_image");

describe("face swap accuracy helpers (docs/46)", () => {
  it("warns when the short side of the face image is under 512px", () => {
    expect(isSmallFaceImage({ width: 400, height: 2000 })).toBe(true);
    expect(isSmallFaceImage({ width: 511, height: 511 })).toBe(true);
    expect(isSmallFaceImage({ width: 512, height: 512 })).toBe(false);
    expect(isSmallFaceImage({ width: 1024, height: 768 })).toBe(false);
  });

  it("estimates time from the measured seconds per step", () => {
    expect(estimateJobSeconds(8)).toBeGreaterThanOrEqual(85);
    expect(estimateJobSeconds(8)).toBeLessThanOrEqual(95);
    expect(estimateJobSeconds(25)).toBeGreaterThanOrEqual(280);
    expect(estimateJobSeconds(25)).toBeLessThanOrEqual(295);
    // 3 枚 × 2 回 × 8 ステップ(約 92 秒)= 約 9.2 分 → 切り上げて 10
    expect(estimateMinutes(6, 8)).toBe(10);
    expect(estimateMinutes(1, 8)).toBe(2);
  });
});

describe("face swap", () => {
  it("supports only Qwen-Image 2.1", () => {
    expect(supportsFaceSwap("qwen_image")).toBe(true);
    for (const k of ["sd15", "sdxl", "flux", "flux_gguf", "anima", "krea2"] as const) {
      expect(supportsFaceSwap(k)).toBe(false);
    }
  });

  it("starts disabled with the head region and the composite on", () => {
    expect(defaultFaceSwap()).toMatchObject({ enabled: false, keepOutside: true, region: "head", resolution: 1024 });
  });

  it("enabling puts the template, 8 steps, CFG 1 and one image per job", () => {
    const p = withFaceSwapEnabled({ ...qwen(), prompt: "a cat", steps: 25, cfg: 4, batchSize: 3 }, true);
    expect(p.faceSwap?.enabled).toBe(true);
    expect(p.prompt).toBe(FACE_SWAP_PROMPT);
    expect(hasFaceSwapPrompt(p.prompt)).toBe(true);
    expect([p.steps, p.cfg, p.batchSize]).toEqual([FACE_SWAP_STEPS.fast, 1, 1]);
  });

  it("keeps an already-edited template and the chosen steps when enabled again", () => {
    const edited = `${FACE_SWAP_PROMPT} the person on the left`;
    const once = { ...withFaceSwapEnabled(qwen(), true), prompt: edited, steps: 25 };
    const again = withFaceSwapEnabled(once, true);
    expect(again.prompt).toBe(edited);
    expect(again.steps).toBe(25);
  });

  it("enabling turns the reference feature off (exclusive), and the reverse", () => {
    const withRef = withReferenceEnabled(qwen(), true);
    expect(withRef.reference?.enabled).toBe(true);
    const swap = withFaceSwapEnabled(withRef, true);
    expect(swap.reference?.enabled).toBe(false);
    const back = withReferenceEnabled(swap, true);
    expect(back.faceSwap?.enabled).toBe(false);
    expect(back.reference?.enabled).toBe(true);
    // 参照画像の設定そのものは残る
    expect(defaultReference().images).toEqual([]);
  });

  it("disabling only flips the switch", () => {
    const on = withFaceSwapEnabled(qwen(), true);
    const off = withFaceSwapEnabled(on, false);
    expect(off.faceSwap?.enabled).toBe(false);
    expect(off.prompt).toBe(on.prompt);
  });

  it("maps steps to a quality button", () => {
    expect(qualityOf(8)).toBe("fast");
    expect(qualityOf(25)).toBe("quality");
    expect(qualityOf(12)).toBeNull();
  });

  it("is ready only with a face and at least one target when enabled", () => {
    const on = { ...defaultFaceSwap(), enabled: true };
    expect(faceSwapReady(undefined, [])).toBe(true);
    expect(faceSwapReady(defaultFaceSwap(), [])).toBe(true); // 無効なら見ない
    expect(faceSwapReady(on, ["inputs/t.png"])).toBe(false); // 顔なし
    expect(faceSwapReady({ ...on, face: "inputs/f.png" }, [])).toBe(false); // 対象なし
    expect(faceSwapReady({ ...on, face: "inputs/f.png" }, ["inputs/t.png"])).toBe(true);
  });

  it("adds targets without duplicates and up to the limit", () => {
    expect(addTargets(["a"], ["a", "b", "c"])).toEqual(["a", "b", "c"]);
    const many = Array.from({ length: MAX_FACE_SWAP_TARGETS + 10 }, (_, i) => `inputs/${i}.png`);
    expect(addTargets([], many)).toHaveLength(MAX_FACE_SWAP_TARGETS);
  });

  it("builds one job per target with batch size 1", () => {
    const base = withFaceSwap({ ...withFaceSwapEnabled(qwen(), true), batchSize: 4 }, { face: "inputs/f.png" });
    const jobs = faceSwapJobs(base, ["inputs/a.png", "inputs/b.png", "inputs/c.png"]);
    expect(jobs.map((j) => j.faceSwap?.target)).toEqual(["inputs/a.png", "inputs/b.png", "inputs/c.png"]);
    expect(jobs.every((j) => j.batchSize === 1 && j.faceSwap?.face === "inputs/f.png")).toBe(true);
    // 元の設定は書き換えない
    expect(base.faceSwap?.target).toBe("");
  });

  it("drops the composite when the engine lacks the nodes", () => {
    const base = withFaceSwap(withFaceSwapEnabled(qwen(), true), { face: "inputs/f.png" });
    expect(faceSwapJobs(base, ["inputs/a.png"], true)[0].faceSwap?.keepOutside).toBe(true);
    expect(faceSwapJobs(base, ["inputs/a.png"], false)[0].faceSwap?.keepOutside).toBe(false);
  });

  it("fills the template when the prompt is empty (Q-389)", () => {
    const base = withFaceSwap(withFaceSwapEnabled(qwen(), true), { face: "inputs/f.png" });
    expect(faceSwapJobs({ ...base, prompt: "  " }, ["inputs/a.png"])[0].prompt).toBe(FACE_SWAP_PROMPT);
    expect(faceSwapJobs({ ...base, prompt: "head_swap: x" }, ["inputs/a.png"])[0].prompt).toBe("head_swap: x");
  });

  it("builds no jobs when disabled or without targets", () => {
    expect(faceSwapJobs(qwen(), ["inputs/a.png"])).toEqual([]);
    expect(faceSwapJobs(withFaceSwapEnabled(qwen(), true), [])).toEqual([]);
  });

  it("restores the single target of a saved job", () => {
    expect(targetsFromParams({ faceSwap: { ...defaultFaceSwap(), target: "inputs/t.png" } })).toEqual(["inputs/t.png"]);
    expect(targetsFromParams({ faceSwap: defaultFaceSwap() })).toEqual([]);
    expect(targetsFromParams({})).toEqual([]);
  });
});
