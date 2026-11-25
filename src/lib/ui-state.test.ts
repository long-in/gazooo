import { describe, expect, it } from "vitest";

import { clampWidth, defaultUiState, parseUiState } from "./ui-state";

describe("parseUiState", () => {
  it("オブジェクト以外は既定に戻す", () => {
    expect(parseUiState(null)).toEqual(defaultUiState);
    expect(parseUiState("x")).toEqual(defaultUiState);
    expect(parseUiState([])).toEqual(defaultUiState);
  });

  it("正しい項目だけ取り込み、壊れた項目は捨てる", () => {
    const s = parseUiState({
      density: "comfortable",
      panels: { a: false, b: "yes", c: true },
      widths: { dock: 320, bad: -1, nan: "x" },
    });
    expect(s.density).toBe("comfortable");
    expect(s.panels).toEqual({ a: false, c: true });
    expect(s.widths).toEqual({ dock: 320 });
  });

  it("未知の密度は標準(compact)にする", () => {
    expect(parseUiState({ density: "huge" }).density).toBe("compact");
  });
});

describe("clampWidth", () => {
  it("最小・最大に収め、整数に丸める", () => {
    expect(clampWidth(100, 200, 400)).toBe(200);
    expect(clampWidth(999, 200, 400)).toBe(400);
    expect(clampWidth(250.6, 200, 400)).toBe(251);
  });
});

describe("generateMode", () => {
  it("既定はかんたん、advanced だけを受け入れる", () => {
    expect(defaultUiState.generateMode).toBe("simple");
    expect(parseUiState({ generateMode: "advanced" }).generateMode).toBe("advanced");
    expect(parseUiState({ generateMode: "x" }).generateMode).toBe("simple");
  });
});

describe("プロジェクトの画面状態(docs/40)", () => {
  it("既定は未分類・ギャラリーは「すべて」", () => {
    expect(defaultUiState.currentProjectId).toBeNull();
    expect(defaultUiState.galleryScope).toBe("all");
  });

  it("保存値を取り込み、壊れた値は既定に戻す", () => {
    const s = parseUiState({ currentProjectId: "p1", galleryScope: "project:p1" });
    expect(s.currentProjectId).toBe("p1");
    expect(s.galleryScope).toBe("project:p1");
    const bad = parseUiState({ currentProjectId: 5, galleryScope: {} });
    expect(bad.currentProjectId).toBeNull();
    expect(bad.galleryScope).toBe("all");
    expect(parseUiState({ currentProjectId: "" }).currentProjectId).toBeNull();
  });
});
