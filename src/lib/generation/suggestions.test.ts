import { describe, expect, it } from "vitest";

import type { EngineModels } from "@/lib/tauri/engine";

import { defaultParams } from "./defaults";
import { applyChanges, diffRows, usableChanges } from "./suggestions";

const models = { samplers: ["euler", "dpmpp_2m"], schedulers: ["normal", "karras"] } as EngineModels;

describe("usableChanges", () => {
  it("drops samplers and schedulers the engine does not have", () => {
    const p = defaultParams("sdxl");
    const r = usableChanges(p, { sampler: "unknown", scheduler: "karras", steps: 30 }, models);
    expect(r).toEqual({ scheduler: "karras", steps: 30 });
  });

  it("passes samplers through when the list is not loaded", () => {
    const r = usableChanges(defaultParams("sdxl"), { sampler: "anything" }, null);
    expect(r.sampler).toBe("anything");
  });

  it("drops fields that do not fit the model kind", () => {
    const flux = usableChanges(
      defaultParams("flux"),
      { cfg: 7, negativePrompt: "x", guidance: 4, faceDetailer: { enabled: true } },
      null,
    );
    expect(flux).toEqual({ guidance: 4 });
    const sd = usableChanges(defaultParams("sd15"), { guidance: 4, cfg: 8 }, null);
    expect(sd).toEqual({ cfg: 8 });
  });
});

describe("diffRows", () => {
  it("lists only values that change", () => {
    const p = { ...defaultParams("sd15"), steps: 20 };
    const rows = diffRows(p, { steps: 20, width: 768, faceDetailer: { enabled: true, denoise: 0.4 } });
    expect(rows).toEqual([
      { field: "width", from: "512", to: "768" },
      { field: "faceEnabled", from: "OFF", to: "ON" },
    ]);
  });
});

describe("applyChanges", () => {
  it("applies changes and returns the inverse for only those fields", () => {
    const p = { ...defaultParams("sd15"), steps: 20, width: 512 };
    const { next, inverse } = applyChanges(p, { steps: 35, faceDetailer: { enabled: true } });
    expect(next.steps).toBe(35);
    expect(next.faceDetailer?.enabled).toBe(true);
    expect(next.faceDetailer?.denoise).toBe(0.4);
    expect(inverse).toEqual({ steps: 20, faceDetailer: { enabled: false } });
    // 逆変更で元に戻る(他の項目は触らない)
    const edited = { ...next, width: 1024 };
    const back = applyChanges(edited, inverse).next;
    expect(back.steps).toBe(20);
    expect(back.faceDetailer?.enabled).toBe(false);
    expect(back.width).toBe(1024);
  });
});
