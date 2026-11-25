import { describe, expect, it } from "vitest";

import type { ConvertEntry } from "@/lib/tauri/convert";

import {
  backgroundApplies,
  dimensionsLabel,
  formatLabel,
  mergeEntries,
  qualityApplies,
  runBlockReason,
} from "./options";

const entry = (id: string, error: string | null = null): ConvertEntry => ({
  id,
  name: `${id}.png`,
  detectedFormat: "png",
  width: 10,
  height: 20,
  bytes: 100,
  error,
});

describe("convert options", () => {
  it("品質は PNG とロスレス WebP では効かない", () => {
    expect(qualityApplies("png", false)).toBe(false);
    expect(qualityApplies("webp", true)).toBe(false);
    expect(qualityApplies("webp", false)).toBe(true);
    expect(qualityApplies("jpeg", false)).toBe(true);
  });

  it("背景色は JPEG のときだけ", () => {
    expect(backgroundApplies("jpeg")).toBe(true);
    expect(backgroundApplies("png")).toBe(false);
    expect(backgroundApplies("webp")).toBe(false);
  });

  it("変換ボタンを押せない理由を返す", () => {
    const s = { outputDir: "C:\\out" };
    expect(runBlockReason([entry("a")], s, false)).toBeNull();
    expect(runBlockReason([entry("a")], s, true)).toBe("running");
    expect(runBlockReason([], s, false)).toBe("noEntries");
    // エラー行だけなら変換できる画像がない
    expect(runBlockReason([entry("a", "x")], s, false)).toBe("noEntries");
    expect(runBlockReason([entry("a")], { outputDir: " " }, false)).toBe("noOutputDir");
    expect(runBlockReason([entry("a")], null, false)).toBe("noOutputDir");
  });

  it("行の追加は id の重複を無視する", () => {
    const merged = mergeEntries([entry("a")], [entry("a"), entry("b")]);
    expect(merged.map((e) => e.id)).toEqual(["a", "b"]);
  });

  it("表示用のラベル", () => {
    expect(dimensionsLabel({ width: 1920, height: 1080 })).toBe("1920×1080");
    expect(dimensionsLabel({ width: 0, height: 0 })).toBe("");
    expect(formatLabel("webp")).toBe("WebP");
    expect(formatLabel(null)).toBe("");
  });
});
