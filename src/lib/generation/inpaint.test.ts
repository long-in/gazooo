import { describe, expect, it } from "vitest";

import { defaultInpaint, hasPaint, inpaintReady, invertPaint, sizeForImage, toGrayMask } from "./inpaint";

describe("sizeForImage", () => {
  it("keeps 64-multiple images as they are", () => {
    expect(sizeForImage(512, 768)).toEqual({ width: 512, height: 768, exact: true });
  });
  it("rounds to 64 and flags the change", () => {
    expect(sizeForImage(1000, 600)).toEqual({ width: 1024, height: 576, exact: false });
  });
  it("shrinks long sides over 2048 keeping the ratio", () => {
    expect(sizeForImage(4096, 2048)).toEqual({ width: 2048, height: 1024, exact: false });
  });
  it("clamps tiny images to the minimum", () => {
    expect(sizeForImage(100, 100)).toEqual({ width: 256, height: 256, exact: false });
  });
});

describe("mask data helpers", () => {
  const px = (r: number, g: number, b: number, a: number) => [r, g, b, a];
  it("invertPaint swaps painted and unpainted pixels", () => {
    const d = new Uint8ClampedArray([...px(255, 0, 0, 255), ...px(0, 0, 0, 0)]);
    invertPaint(d);
    expect(Array.from(d)).toEqual([...px(255, 0, 0, 0), ...px(255, 0, 0, 255)]);
  });
  it("hasPaint detects any painted pixel", () => {
    expect(hasPaint(new Uint8ClampedArray([...px(255, 0, 0, 0), ...px(255, 0, 0, 0)]))).toBe(false);
    expect(hasPaint(new Uint8ClampedArray([...px(255, 0, 0, 0), ...px(255, 0, 0, 200)]))).toBe(true);
  });
  it("toGrayMask uses the red channel as the value", () => {
    const d = new Uint8ClampedArray(px(200, 0, 0, 255));
    toGrayMask(d);
    expect(Array.from(d)).toEqual(px(200, 200, 200, 255));
  });
});

describe("inpaintReady", () => {
  it("is ready when disabled or undefined", () => {
    expect(inpaintReady(undefined)).toBe(true);
    expect(inpaintReady(defaultInpaint())).toBe(true);
  });
  it("needs both image and mask when enabled", () => {
    const on = { ...defaultInpaint(), enabled: true };
    expect(inpaintReady(on)).toBe(false);
    expect(inpaintReady({ ...on, image: "inputs/a.png" })).toBe(false);
    expect(inpaintReady({ ...on, image: "inputs/a.png", mask: "inputs/m.png" })).toBe(true);
  });
});
