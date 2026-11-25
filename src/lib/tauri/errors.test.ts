import { describe, expect, it } from "vitest";

import { isAppError, isOutOfMemory, toAppError } from "./errors";

describe("AppError", () => {
  it("Rust 側の形式を判定できる", () => {
    expect(isAppError({ code: "DB_ERROR", message: "x", details: null })).toBe(true);
    expect(isAppError("oops")).toBe(false);
    expect(isAppError(null)).toBe(false);
  });

  it("未知のエラーは INTERNAL に正規化される", () => {
    expect(toAppError(new Error("boom"))).toEqual({ code: "INTERNAL", message: "boom" });
    expect(toAppError("str").code).toBe("INTERNAL");
  });

  it("AppError はそのまま返す", () => {
    const e = { code: "VALIDATION", message: "m", details: { field: "comfyuiUrl" } };
    expect(toAppError(e)).toBe(e);
  });
});

describe("isOutOfMemory", () => {
  it("メモリ不足を示す典型的なメッセージを判定する", () => {
    expect(isOutOfMemory("KSampler: MPS backend out of memory (MPS allocated: 9.1 GB)")).toBe(true);
    expect(isOutOfMemory("CUDA out of memory. Tried to allocate 2.00 GiB")).toBe(true);
    expect(isOutOfMemory("OOM when allocating tensor")).toBe(true);
    expect(isOutOfMemory("Failed to allocate memory")).toBe(true);
    expect(isOutOfMemory("KSampler: invalid sampler")).toBe(false);
    expect(isOutOfMemory("room")).toBe(false);
  });
});
