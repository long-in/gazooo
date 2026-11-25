import { describe, expect, it } from "vitest";

import {
  ACCENTS,
  applyAccent,
  applyPinKey,
  USER_COLORS,
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

/** oklch 文字列 → 相対輝度(WCAG)。 */
function luminance(css: string): number {
  const m = /oklch\(([\d.]+) ([\d.]+) ([\d.]+)\)/.exec(css);
  if (!m) throw new Error(css);
  const [L, C, h] = [Number(m[1]), Number(m[2]), (Number(m[3]) * Math.PI) / 180];
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const mm = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const r = 4.0767416621 * l - 3.3077115913 * mm + 0.2309699292 * s;
  const g = -1.2684380046 * l + 2.6097574011 * mm - 0.3413193965 * s;
  const bl = -0.0041960863 * l - 0.7034186147 * mm + 1.707614701 * s;
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return 0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(bl);
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

describe("accent", () => {
  it("すべての色にアクセントがあり、前景色との対比が 4.5:1 以上", () => {
    for (const c of USER_COLORS) {
      for (const tone of [ACCENTS[c].light, ACCENTS[c].dark]) {
        expect(contrast(tone.bg, tone.fg), c).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("applyAccent は CSS 変数を上書きし、null で外す", () => {
    const props = new Map<string, string>();
    const el = {
      style: {
        setProperty: (k: string, v: string) => props.set(k, v),
        removeProperty: (k: string) => props.delete(k),
        getPropertyValue: (k: string) => props.get(k) ?? "",
      },
    } as unknown as HTMLElement;
    applyAccent(el, "pink", false);
    expect(el.style.getPropertyValue("--primary")).toBe(ACCENTS.pink.light.bg);
    expect(el.style.getPropertyValue("--sidebar-primary-foreground")).toBe(ACCENTS.pink.light.fg);
    applyAccent(el, "pink", true);
    expect(el.style.getPropertyValue("--ring")).toBe(ACCENTS.pink.dark.bg);
    applyAccent(el, null, true);
    expect(el.style.getPropertyValue("--primary")).toBe("");
  });
});
