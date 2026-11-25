import { describe, expect, it } from "vitest";

import { groupByProject } from "./use-move-images";

describe("groupByProject(元に戻す用)", () => {
  it("直前の所属ごとに画像 ID をまとめる(未分類は null)", () => {
    const g = groupByProject([
      { id: "a", projectId: "p1" },
      { id: "b", projectId: null },
      { id: "c", projectId: "p1" },
    ]);
    expect(g.get("p1")).toEqual(["a", "c"]);
    expect(g.get(null)).toEqual(["b"]);
    expect(g.size).toBe(2);
  });

  it("空なら空", () => {
    expect(groupByProject([]).size).toBe(0);
  });
});
