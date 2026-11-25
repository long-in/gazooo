// アプリのバージョンが、package.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json で一致しているか確認する。
// リリース(タグ `v*`)では、タグのバージョンとも一致していることを確認する。
//
// 使い方: node tools/check-version.mjs [タグ名(例: v0.1.0)]
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Cargo.toml の `[package]` セクションの version を取り出す。
 * @param {string} toml
 * @returns {string | null}
 */
export function cargoPackageVersion(toml) {
  const section = toml.match(/^\[package\]\s*$([\s\S]*?)(?=^\[|\Z)/m);
  const version = section?.[1].match(/^version\s*=\s*"([^"]+)"/m);
  return version?.[1] ?? null;
}

/**
 * 各ソースのバージョンを比べ、食い違い(または欠落)を説明する文字列の配列で返す。空なら一致。
 * @param {{ packageJson?: string | null, cargo?: string | null, tauriConf?: string | null, tag?: string }} input
 * @returns {string[]}
 */
export function findMismatches({ packageJson, cargo, tauriConf, tag }) {
  const versions = [
    ["package.json", packageJson],
    ["src-tauri/Cargo.toml", cargo],
    ["src-tauri/tauri.conf.json", tauriConf],
  ];
  if (tag) versions.push([`タグ ${tag}`, tag.replace(/^v/, "")]);

  const problems = versions.filter(([, v]) => !v).map(([name]) => `${name}: バージョンを読み取れません`);
  const known = versions.filter(([, v]) => v);
  const distinct = new Set(known.map(([, v]) => v));
  if (distinct.size > 1) {
    problems.push(`バージョンが一致しません: ${known.map(([n, v]) => `${n}=${v}`).join(", ")}`);
  }
  return problems;
}

function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const read = (p) => readFileSync(path.join(root, p), "utf8");
  const problems = findMismatches({
    packageJson: JSON.parse(read("package.json")).version,
    cargo: cargoPackageVersion(read("src-tauri/Cargo.toml")),
    tauriConf: JSON.parse(read("src-tauri/tauri.conf.json")).version,
    tag: process.argv[2],
  });
  if (problems.length > 0) {
    for (const p of problems) console.error(`ERROR: ${p}`);
    process.exit(1);
  }
  console.log(`OK: version ${JSON.parse(read("package.json")).version}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
