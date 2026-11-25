import { describe, expect, it } from "vitest";

import { cargoPackageVersion, findMismatches } from "./check-version.mjs";

const ok = { packageJson: "0.1.0", cargo: "0.1.0", tauriConf: "0.1.0" };

describe("cargoPackageVersion", () => {
  it("[package] の version を取り出し、依存の version には惑わされない", () => {
    const toml = `[package]\nname = "gazooo"\nversion = "1.2.3"\nedition = "2024"\n\n[dependencies]\nserde = { version = "=1.0.0" }\n`;
    expect(cargoPackageVersion(toml)).toBe("1.2.3");
  });

  it("[package] が無い・version が無い場合は null", () => {
    expect(cargoPackageVersion(`[dependencies]\nversion = "9"\n`)).toBeNull();
    expect(cargoPackageVersion(`[package]\nname = "x"\n`)).toBeNull();
  });
});

describe("findMismatches", () => {
  it("すべて一致していれば問題なし(タグの v 接頭辞は無視)", () => {
    expect(findMismatches(ok)).toEqual([]);
    expect(findMismatches({ ...ok, tag: "v0.1.0" })).toEqual([]);
  });

  it("食い違いを検出して、どこがいくつかを示す", () => {
    const problems = findMismatches({ ...ok, cargo: "0.2.0" });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("src-tauri/Cargo.toml=0.2.0");
    expect(problems[0]).toContain("package.json=0.1.0");
  });

  it("タグが違う場合も検出する", () => {
    expect(findMismatches({ ...ok, tag: "v0.1.1" })[0]).toContain("タグ v0.1.1=0.1.1");
  });

  it("読み取れないものは、食い違いとは別に報告する", () => {
    const problems = findMismatches({ ...ok, tauriConf: undefined });
    expect(problems).toEqual(["src-tauri/tauri.conf.json: バージョンを読み取れません"]);
  });
});
