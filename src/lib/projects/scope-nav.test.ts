import { describe, expect, it } from "vitest";

import { filterProjects, formatCount, moveIndex } from "./scope-nav";

describe("moveIndex(docs/40 §11.2)", () => {
  it("↑↓ は端で止まり、Home / End は両端へ", () => {
    expect(moveIndex(0, 5, "ArrowDown")).toBe(1);
    expect(moveIndex(4, 5, "ArrowDown")).toBe(4);
    expect(moveIndex(0, 5, "ArrowUp")).toBe(0);
    expect(moveIndex(3, 5, "Home")).toBe(0);
    expect(moveIndex(1, 5, "End")).toBe(4);
  });
  it("対象外のキー・項目なしは null", () => {
    expect(moveIndex(0, 5, "a")).toBeNull();
    expect(moveIndex(0, 0, "ArrowDown")).toBeNull();
  });
});

describe("filterProjects", () => {
  const ps = [
    { id: "1", name: "キャラA" },
    { id: "2", name: "Background" },
    { id: "3", name: "背景テスト" },
  ];
  it("空の検索語はすべて", () => expect(filterProjects(ps, "  ", null)).toHaveLength(3));
  it("部分一致(大文字小文字を区別しない)", () => {
    expect(filterProjects(ps, "back", null).map((p) => p.id)).toEqual(["2"]);
  });
  it("選択中は一致しなくても残す", () => {
    expect(filterProjects(ps, "back", "1").map((p) => p.id)).toEqual(["1", "2"]);
  });
});

describe("formatCount", () => {
  it("1 万未満は桁区切り", () => expect(formatCount(1240, "ja")).toBe("1,240"));
  it("1 万以上は短縮", () => {
    expect(formatCount(12_345, "ja")).toBe("1.2万");
    expect(formatCount(12_345, "en")).toBe("12k");
  });
});
