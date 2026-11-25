import { describe, expect, it } from "vitest";

import {
  activeEntry,
  decodeEntry,
  encodeEntry,
  sameEntry,
  scopeMatches,
  selectEntry,
  stripScope,
  validEntry,
} from "./scope";

describe("selectEntry / activeEntry(docs/40 §5.1)", () => {
  it("すべて・お気に入り・未分類・プロジェクトの状態", () => {
    expect(selectEntry("all")).toEqual({ scope: "all", favoritesOnly: false });
    expect(selectEntry("favorites")).toEqual({ scope: "all", favoritesOnly: true });
    expect(selectEntry("unassigned")).toEqual({ scope: "unassigned", favoritesOnly: false });
    expect(selectEntry({ projectId: "p1" })).toEqual({ scope: { projectId: "p1" }, favoritesOnly: false });
  });

  it("お気に入りは「すべて」+ スイッチ ON と同じ(スイッチと常に同期)", () => {
    expect(activeEntry({ scope: "all", favoritesOnly: true })).toBe("favorites");
    expect(activeEntry({ scope: "all", favoritesOnly: false })).toBe("all");
  });

  it("プロジェクト内でスイッチを ON にしても、強調はそのプロジェクトのまま", () => {
    expect(activeEntry({ scope: { projectId: "p1" }, favoritesOnly: true })).toEqual({ projectId: "p1" });
    expect(activeEntry({ scope: "unassigned", favoritesOnly: true })).toBe("unassigned");
  });

  it("各項目の往復で、選んだ項目が強調される", () => {
    for (const e of ["all", "favorites", "unassigned", { projectId: "p9" }] as const) {
      expect(sameEntry(activeEntry(selectEntry(e)), e)).toBe(true);
    }
  });
});

describe("encodeEntry / decodeEntry", () => {
  it("往復できる", () => {
    for (const e of ["all", "favorites", "unassigned", { projectId: "0190-abc" }] as const) {
      expect(decodeEntry(encodeEntry(e))).toEqual(e);
    }
  });

  it("壊れた値は「すべて」", () => {
    for (const raw of [undefined, null, 1, {}, "", "project:", "nope", "Project:x"]) {
      expect(decodeEntry(raw)).toBe("all");
    }
  });
});

describe("scopeMatches", () => {
  it("all は常に合い、未分類・プロジェクトは所属で判定する", () => {
    expect(scopeMatches("all", null)).toBe(true);
    expect(scopeMatches("all", "p1")).toBe(true);
    expect(scopeMatches("unassigned", null)).toBe(true);
    expect(scopeMatches("unassigned", "p1")).toBe(false);
    expect(scopeMatches({ projectId: "p1" }, "p1")).toBe(true);
    expect(scopeMatches({ projectId: "p1" }, "p2")).toBe(false);
    expect(scopeMatches({ projectId: "p1" }, null)).toBe(false);
  });
});

describe("validEntry", () => {
  it("存在しないプロジェクトは「すべて」へ戻す", () => {
    const ids = new Set(["p1"]);
    expect(validEntry({ projectId: "p1" }, ids)).toEqual({ projectId: "p1" });
    expect(validEntry({ projectId: "gone" }, ids)).toBe("all");
    expect(validEntry("favorites", ids)).toBe("favorites");
  });
});

describe("stripScope(docs/40 §4.3)", () => {
  it("「このプロジェクト」は現在のプロジェクト(なし = 未分類)、「すべて」は全画像", () => {
    expect(stripScope(false, "p1")).toEqual({ projectId: "p1" });
    expect(stripScope(false, null)).toBe("unassigned");
    expect(stripScope(true, "p1")).toBe("all");
  });
});
