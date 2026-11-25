import { describe, expect, it } from "vitest";

import { matchSimpleSize, simpleSize } from "./defaults";

describe("simpleSize", () => {
  it("64 の倍数で、縦長は縦が長く横長は横が長い", () => {
    for (const kind of ["sd15", "sdxl"] as const) {
      for (const scale of ["small", "medium", "large"] as const) {
        const p = simpleSize(kind, "portrait", scale);
        const l = simpleSize(kind, "landscape", scale);
        expect(p.h).toBeGreaterThan(p.w);
        expect(l.w).toBeGreaterThan(l.h);
        for (const n of [p.w, p.h, l.w, l.h]) expect(n % 64).toBe(0);
      }
    }
  });

  it("SDXL の中サイズは定番の 1024 / 832×1216", () => {
    expect(simpleSize("sdxl", "square", "medium")).toEqual({ w: 1024, h: 1024 });
    expect(simpleSize("sdxl", "portrait", "medium")).toEqual({ w: 832, h: 1216 });
  });

  it("現在のサイズから選択を逆引きできる(合わなければ null)", () => {
    expect(matchSimpleSize("sdxl", 1216, 832)).toEqual({ orientation: "landscape", scale: "medium" });
    expect(matchSimpleSize("sdxl", 1000, 1000)).toBeNull();
  });
});
