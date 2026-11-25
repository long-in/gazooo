import { describe, expect, it } from "vitest";

import {
  archiveUrl,
  buildManifest,
  decideManifestVersion,
  hostAllowed,
  nextManifestVersion,
  parseEnv,
  pythonUrl,
  stringifyManifest,
  urlAllowed,
  validateManifest,
} from "./gen-engine-manifest.mjs";

const C1 = "a".repeat(40);
const C2 = "b".repeat(40);
const C3 = "c".repeat(40);
const C4 = "d".repeat(40);
const SHA = "e".repeat(64);

const envText = `
# コメント
COMFYUI_TAG=v1.0.0
COMFYUI_REPO=https://github.com/o/ComfyUI
GGUF_NODE_COMMIT=${C1}
GGUF_NODE_REPO=https://github.com/o/gguf
IMPACT_PACK_COMMIT=${C2}
IMPACT_PACK_REPO=https://github.com/o/pack
IMPACT_SUBPACK_COMMIT=${C3}
IMPACT_SUBPACK_REPO=https://github.com/o/subpack
PYTHON_VERSION=3.12
TORCH_VERSION=2.0.0
TORCHVISION_VERSION=0.1.0
TORCHAUDIO_VERSION=0.2.0
TORCH_CUDA_INDEX=https://download.pytorch.org/whl/cu130
`;

const meta = {
  schemaVersion: 1,
  allowedHosts: ["github.com", "download.pytorch.org"],
  python: { patchVersion: "3.12.9", releaseTag: "20260101" },
  comfyui: { license: "GPL-3.0", envTag: "COMFYUI_TAG", envRepo: "COMFYUI_REPO" },
  torch: { license: "BSD", downloadMb: 1, diskMb: 2 },
  components: [
    {
      id: "gguf",
      name: "GGUF",
      license: "Apache-2.0",
      expectNode: "N1",
      downloadMb: 1,
      diskMb: 2,
      envCommit: "GGUF_NODE_COMMIT",
      envRepo: "GGUF_NODE_REPO",
    },
    {
      id: "face",
      name: "Face",
      license: "GPL",
      expectNode: "N2",
      downloadMb: 1,
      diskMb: 2,
      envCommit: "IMPACT_PACK_COMMIT",
      envRepo: "IMPACT_PACK_REPO",
      extra: { name: "Subpack", expectNode: "N3", envCommit: "IMPACT_SUBPACK_COMMIT", envRepo: "IMPACT_SUBPACK_REPO" },
    },
  ],
};

const env = parseEnv(envText);

/** 取得済みとして、すべてのソースに同じ SHA-256 を入れた既存マニフェストを作る。 */
function filled(commitOfComfy = C4) {
  const { manifest } = buildManifest({
    env,
    meta,
    existing: { comfyui: { tag: "v1.0.0", commit: commitOfComfy } },
  });
  const fill = (s: { sha256: string; sizeBytes: number }) => Object.assign(s, { sha256: SHA, sizeBytes: 10 });
  fill(manifest.python["windows-x64"]);
  fill(manifest.python["macos-arm64"]);
  fill(manifest.comfyui.source);
  for (const c of manifest.components) {
    fill(c.source);
    if (c.extra) fill(c.extra.source);
  }
  manifest.manifestVersion = "2026.11.1";
  return manifest;
}

describe("parseEnv", () => {
  it("KEY=value を読み、コメントと空行を無視する", () => {
    expect(parseEnv("# c\n\nA=1\n B = x=y \r\nC")).toEqual({ A: "1", B: "x=y" });
  });
});

describe("URL の組み立てと許可判定", () => {
  it("アーカイブはコミット固定の URL(末尾のスラッシュは無視)", () => {
    expect(archiveUrl("https://github.com/o/r/", C1)).toBe(`https://github.com/o/r/archive/${C1}.zip`);
  });

  it("Python の配布ファイルの URL は %2B でエスケープする", () => {
    expect(pythonUrl("20261110", "3.12.16", "x86_64-pc-windows-msvc")).toBe(
      "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/cpython-3.12.16%2B20261110-x86_64-pc-windows-msvc-install_only.tar.gz",
    );
  });

  it("許可ホストとそのサブドメインだけを、https で通す", () => {
    expect(hostAllowed("codeload.github.com", ["github.com"])).toBe(true);
    expect(hostAllowed("evilgithub.com", ["github.com"])).toBe(false);
    expect(urlAllowed("https://github.com/x", ["github.com"])).toBe(true);
    expect(urlAllowed("http://github.com/x", ["github.com"])).toBe(false);
    expect(urlAllowed("https://evil.example.com/x", ["github.com"])).toBe(false);
    expect(urlAllowed("not a url", ["github.com"])).toBe(false);
  });
});

describe("buildManifest", () => {
  it("versions.env の値から commit・url・torch を組み立てる", () => {
    const { manifest, problems } = buildManifest({ env, meta, existing: { comfyui: { tag: "v1.0.0", commit: C4 } } });
    expect(problems).toEqual([]);
    expect(manifest.comfyui).toMatchObject({ tag: "v1.0.0", commit: C4 });
    expect(manifest.comfyui.source.url).toBe(`https://github.com/o/ComfyUI/archive/${C4}.zip`);
    expect(manifest.torch).toMatchObject({ version: "2.0.0", torchvision: "0.1.0", torchaudio: "0.2.0" });
    expect(manifest.components[0].commit).toBe(C1);
    expect(manifest.components[1].extra.commit).toBe(C3);
    expect(manifest.python.version).toBe("3.12.9");
  });

  it("URL が既知なら SHA-256 を再利用し、未知の URL だけ missing に入れる", () => {
    const existing = filled();
    const same = buildManifest({ env, meta, existing });
    expect(same.missing).toEqual([]);
    expect(same.manifest.components[0].source.sha256).toBe(SHA);

    const changed = buildManifest({ env: { ...env, GGUF_NODE_COMMIT: C2 }, meta, existing });
    expect(changed.missing).toEqual([`https://github.com/o/gguf/archive/${C2}.zip`]);
  });

  it("ComfyUI のタグが同じならコミットを再利用し、変わったら解決関数を呼ぶ", () => {
    const existing = filled();
    let called = 0;
    const resolve = (tag: string) => {
      called++;
      return tag === "v2.0.0" ? C2 : C1;
    };
    buildManifest({ env, meta, existing, resolveTagCommit: resolve });
    expect(called).toBe(0);
    const r = buildManifest({ env: { ...env, COMFYUI_TAG: "v2.0.0" }, meta, existing, resolveTagCommit: resolve });
    expect(called).toBe(1);
    expect(r.manifest.comfyui.commit).toBe(C2);
  });

  it("タグが変わって解決できない(オフライン)ときは問題として報告する", () => {
    const r = buildManifest({ env: { ...env, COMFYUI_TAG: "v2.0.0" }, meta, existing: filled() });
    expect(r.problems[0]).toContain("v2.0.0");
  });

  it("キーの欠落・コミット形式の不正・PYTHON_VERSION の不整合を報告する", () => {
    const noRepo: Record<string, string> = { ...env };
    delete noRepo.COMFYUI_REPO;
    expect(buildManifest({ env: noRepo, meta, existing: filled() }).problems.join("\n")).toContain("COMFYUI_REPO");
    expect(
      buildManifest({ env: { ...env, GGUF_NODE_COMMIT: "abc" }, meta, existing: filled() }).problems.join("\n"),
    ).toContain("GGUF_NODE_COMMIT");
    expect(
      buildManifest({ env: { ...env, PYTHON_VERSION: "3.13" }, meta, existing: filled() }).problems.join("\n"),
    ).toContain("PYTHON_VERSION");
  });
});

describe("validateManifest", () => {
  it("取得済みのマニフェストは通る", () => {
    expect(validateManifest(filled())).toEqual([]);
  });

  it("許可ホスト外・http・不正な SHA-256・サイズ 0・ID の重複と文字種を拒否する", () => {
    const m = filled();
    m.comfyui.source.url = "https://evil.example.com/x.zip";
    m.components[0].source.sha256 = "ABC";
    m.components[1].source.sizeBytes = 0;
    m.components[1].id = "gguf";
    const text = validateManifest(m).join("\n");
    expect(text).toContain("許可されていない取得元");
    expect(text).toContain("SHA-256");
    expect(text).toContain("サイズが 0");
    expect(text).toContain("重複");

    const bad = filled();
    bad.components[0].id = "Bad_ID";
    expect(validateManifest(bad).join("\n")).toContain("使えない文字");

    const http = filled();
    http.comfyui.source.url = http.comfyui.source.url.replace("https://", "http://");
    expect(validateManifest(http).join("\n")).toContain("許可されていない取得元");
  });
});

describe("stringifyManifest", () => {
  it("インデント 2・LF・末尾に改行で、キー順が組み立て順のまま", () => {
    const text = stringifyManifest(filled());
    expect(text.endsWith("}\n")).toBe(true);
    expect(text).not.toContain("\r");
    expect(text.split("\n")[1]).toMatch(/^ {2}"_generated"/);
    expect(Object.keys(JSON.parse(text))).toEqual([
      "_generated",
      "schemaVersion",
      "manifestVersion",
      "allowedHosts",
      "python",
      "comfyui",
      "torch",
      "components",
    ]);
  });
});

describe("manifestVersion", () => {
  it("同じ年月なら +1、月が替われば .1", () => {
    expect(nextManifestVersion("2026.11.1", new Date(2026, 10, 23))).toBe("2026.11.2");
    expect(nextManifestVersion("2026.11.9", new Date(2026, 10, 23))).toBe("2026.11.10");
    expect(nextManifestVersion("2026.11.2", new Date(2026, 11, 1))).toBe("2026.12.1");
    expect(nextManifestVersion(undefined, new Date(2026, 0, 5))).toBe("2026.1.1");
    expect(nextManifestVersion("壊れた値", new Date(2026, 0, 5))).toBe("2026.1.1");
  });

  it("内容が変わっていなければ既存の版のまま、変わっていれば進める。上書きも可", () => {
    const existing = filled();
    const now = new Date(2026, 10, 23);
    const same = filled();
    expect(decideManifestVersion({ manifest: same, existing, now })).toBe("2026.11.1");

    const changed = filled();
    changed.components[0].commit = C2;
    expect(decideManifestVersion({ manifest: changed, existing, now })).toBe("2026.11.2");
    expect(decideManifestVersion({ manifest: changed, existing, now, override: "x.y.z" })).toBe("x.y.z");
  });

  it("_generated の有無だけの違いでは版を進めない", () => {
    const legacy: Record<string, unknown> = { ...filled() };
    delete legacy._generated;
    expect(decideManifestVersion({ manifest: filled(), existing: legacy, now: new Date(2026, 10, 23) })).toBe("2026.11.1");
  });
});
