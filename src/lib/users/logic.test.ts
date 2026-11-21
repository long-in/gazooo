import { describe, expect, it } from "vitest";

import {
  applyPinKey,
  colorClass,
  formatRemaining,
  initialOf,
  isWeakPin,
  lockedUntilFromError,
  lockRemainingMs,
  pinKeyFromEvent,
} from "./logic";

describe("PIN pad keys", () => {
  it("maps digits, numpad digits, Backspace, Esc and Enter", () => {
    expect(pinKeyFromEvent("7")).toEqual({ kind: "digit", digit: "7" });
    // テンキーも `key` は数字になる
    expect(pinKeyFromEvent("0")).toEqual({ kind: "digit", digit: "0" });
    expect(pinKeyFromEvent("Backspace")).toEqual({ kind: "backspace" });
    expect(pinKeyFromEvent("Escape")).toEqual({ kind: "clear" });
    expect(pinKeyFromEvent("Enter")).toEqual({ kind: "enter" });
    expect(pinKeyFromEvent("a")).toBeNull();
    expect(pinKeyFromEvent("10")).toBeNull();
  });

  it("appends digits up to the length, deletes and clears", () => {
    let v = "";
    for (const d of "12345") v = applyPinKey(v, { kind: "digit", digit: d }, 4);
    expect(v).toBe("1234");
    expect(applyPinKey(v, { kind: "backspace" }, 4)).toBe("123");
    expect(applyPinKey(v, { kind: "clear" }, 4)).toBe("");
    expect(applyPinKey("", { kind: "backspace" }, 4)).toBe("");
    expect(applyPinKey(v, { kind: "enter" }, 4)).toBe("1234");
  });
});

describe("lockout display", () => {
  it("computes the remaining time and formats it", () => {
    expect(lockRemainingMs(null, 1000)).toBe(0);
    expect(lockRemainingMs(500, 1000)).toBe(0);
    expect(lockRemainingMs(61_000, 1000)).toBe(60_000);
    expect(formatRemaining(65_000)).toBe("1:05");
    expect(formatRemaining(29_100)).toBe("0:30");
  });

  it("reads the wait from an AUTH_LOCKED_OUT error only", () => {
    expect(
      lockedUntilFromError(
        { code: "AUTH_LOCKED_OUT", message: "", details: { retryAfterMs: 30_000 } as never },
        1000,
      ),
    ).toBe(31_000);
    expect(
      lockedUntilFromError({ code: "AUTH_FAILED", message: "" }, 1000),
    ).toBeNull();
    expect(
      lockedUntilFromError({ code: "AUTH_LOCKED_OUT", message: "" }, 1000),
    ).toBeNull();
  });
});

describe("avatar and PIN strength", () => {
  it("uses the first character of the name", () => {
    expect(initialOf("taro")).toBe("T");
    expect(initialOf("  はなこ")).toBe("は");
    expect(initialOf("😀 smile")).toBe("😀");
    expect(initialOf("")).toBe("?");
  });

  it("falls back to a known colour class", () => {
    expect(colorClass("red")).toContain("bg-red");
    expect(colorClass("nope")).toContain("bg-blue");
  });

  it("flags repeated and sequential PINs as weak", () => {
    expect(isWeakPin("0000")).toBe(true);
    expect(isWeakPin("1234")).toBe(true);
    expect(isWeakPin("4321")).toBe(true);
    expect(isWeakPin("1357")).toBe(false);
    expect(isWeakPin("")).toBe(false);
  });
});
