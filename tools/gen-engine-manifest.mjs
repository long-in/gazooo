// アプリ内インストーラのマニフェスト(manifest.json)を、versions.env と manifest-meta.json から生成・検査する(docs/41)。
//
// 使い方:
//   node tools/gen-engine-manifest.mjs            生成(manifest.json を書き換える)
//   node tools/gen-engine-manifest.mjs --check    検査のみ(オフライン。差があれば exit 1)
//   node tools/gen-engine-manifest.mjs --verify   全ソースを再取得して SHA-256 とタグのコミットを照合(オンライン)
//   --manifest-version <値>                       manifestVersion を上書きする(生成時のみ)
//
// 追加の依存は使わない(node:crypto・組み込みの fetch・git のみ)。
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const GENERATED_NOTE =
  "tools/gen-engine-manifest.mjs が versions.env と manifest-meta.json から生成します。手で編集せず、npm run engine-manifest を実行してください(docs/41)";
const MAX_BYTES = 200 * 1024 * 1024;
const MAX_REDIRECTS = 5;
const FETCH_TIMEOUT_MS = 120_000;

const PYTHON_TARGETS = {
  "windows-x64": "x86_64-pc-windows-msvc",
  "macos-arm64": "aarch64-apple-darwin",
};
const PBS_RELEASES = "https://github.com/astral-sh/python-build-standalone/releases/download";

/**
 * `KEY=value` 形式(# 始まりはコメント)を読む。
 * @param {string} text
 * @returns {Record<string, string>}
 */
export function parseEnv(text) {
  /** @type {Record<string, string>} */
  const env = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    env[line.slice(0, eq).trim()] = line.slice(eq + 1).trim();
  }
  return env;
}

/** ホスト名が許可リストと一致する(またはそのサブドメイン)か。Rust の `DownloadPolicy::host_allowed` と同じ規則。 */
export function hostAllowed(host, allowedHosts) {
  const h = host.toLowerCase();
  return allowedHosts.some((a) => h === a || h.endsWith(`.${a}`));
}

/** https かつ許可ホストか。 */
export function urlAllowed(url, allowedHosts) {
  let u;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  return u.protocol === "https:" && hostAllowed(u.hostname, allowedHosts);
}

/** `<repo>/archive/<commit>.zip` */
export function archiveUrl(repo, commit) {
  return `${repo.replace(/\/+$/, "")}/archive/${commit}.zip`;
}

/** python-build-standalone の配布ファイルの URL。 */
export function pythonUrl(releaseTag, patchVersion, target) {
  return `${PBS_RELEASES}/${releaseTag}/cpython-${patchVersion}%2B${releaseTag}-${target}-install_only.tar.gz`;
}

/** 内容が変わったときに日付版(`YYYY.M.n`)を進める。同じ年月なら末尾を +1、替わっていれば `.1`。 */
export function nextManifestVersion(prev, now) {
  const prefix = `${now.getFullYear()}.${now.getMonth() + 1}.`;
  if (typeof prev === "string" && prev.startsWith(prefix)) {
    const n = Number(prev.slice(prefix.length));
    if (Number.isInteger(n) && n >= 1) return `${prefix}${n + 1}`;
  }
  return `${prefix}1`;
}

const HEX40 = /^[0-9a-f]{40}$/;
const HEX64 = /^[0-9a-f]{64}$/;

function need(env, key, problems) {
  const v = env[key];
  if (!v) problems.push(`versions.env に ${key} がありません`);
  return v ?? "";
}

/**
 * マニフェストを組み立てる(純関数。ネットワークは使わない)。
 * SHA-256・サイズは `known`(URL → {sha256, sizeBytes})から引く。無い URL は `missing` に入れる。
 * @param {{ env: Record<string,string>, meta: any, existing?: any, resolveTagCommit?: (tag: string, repo: string) => string }} input
 * @returns {{ manifest: any, missing: string[], problems: string[] }}
 */
export function buildManifest({ env, meta, existing, resolveTagCommit }) {
  /** @type {string[]} */
  const problems = [];
  /** @type {string[]} */
  const missing = [];

  /** @type {Map<string, {sha256: string, sizeBytes: number}>} */
  const known = new Map();
  const remember = (s) => {
    if (s?.url && s.sha256 && s.sizeBytes) known.set(s.url, { sha256: s.sha256, sizeBytes: s.sizeBytes });
  };
  if (existing) {
    remember(existing.python?.["windows-x64"]);
    remember(existing.python?.["macos-arm64"]);
    remember(existing.comfyui?.source);
    for (const c of existing.components ?? []) {
      remember(c.source);
      remember(c.extra?.source);
    }
  }
  const source = (url) => {
    const k = known.get(url);
    if (!k) {
      missing.push(url);
      return { url, sha256: "", sizeBytes: 0 };
    }
    return { url, sha256: k.sha256, sizeBytes: k.sizeBytes };
  };
  const commitOf = (key) => {
    const v = need(env, key, problems);
    if (v && !HEX40.test(v)) problems.push(`${key} は 40 桁の小文字 16 進のコミットにしてください`);
    return v;
  };

  // Python(パッチ版とリリース日付は meta。メジャー.マイナーは versions.env と整合させる)
  const pyMajorMinor = need(env, "PYTHON_VERSION", problems);
  const patch = meta.python.patchVersion;
  if (pyMajorMinor && !patch.startsWith(`${pyMajorMinor}.`)) {
    problems.push(`PYTHON_VERSION(${pyMajorMinor})と manifest-meta.json の python.patchVersion(${patch})が一致しません`);
  }
  const python = { version: patch };
  for (const [name, target] of Object.entries(PYTHON_TARGETS)) {
    python[name] = source(pythonUrl(meta.python.releaseTag, patch, target));
  }

  // ComfyUI(タグが同じなら既存のコミットを使い、変わったときだけ解決する)
  const tag = need(env, meta.comfyui.envTag, problems);
  const comfyRepo = need(env, meta.comfyui.envRepo, problems);
  let comfyCommit = "";
  if (existing?.comfyui?.tag === tag && existing.comfyui.commit) {
    comfyCommit = existing.comfyui.commit;
  } else if (resolveTagCommit && tag && comfyRepo) {
    try {
      comfyCommit = resolveTagCommit(tag, comfyRepo);
    } catch (e) {
      problems.push(`ComfyUI のタグ ${tag} をコミットに解決できません: ${e instanceof Error ? e.message : e}`);
    }
  } else if (tag) {
    problems.push(`ComfyUI のタグが ${tag} に変わっています。npm run engine-manifest で生成してください`);
  }
  if (comfyCommit && !HEX40.test(comfyCommit)) problems.push(`ComfyUI のコミットが不正です: ${comfyCommit}`);
  const comfyui = {
    tag,
    commit: comfyCommit,
    license: meta.comfyui.license,
    source: source(archiveUrl(comfyRepo, comfyCommit)),
  };

  const torch = {
    version: need(env, "TORCH_VERSION", problems),
    torchvision: need(env, "TORCHVISION_VERSION", problems),
    torchaudio: need(env, "TORCHAUDIO_VERSION", problems),
    windowsIndex: need(env, "TORCH_CUDA_INDEX", problems),
    license: meta.torch.license,
    downloadMb: meta.torch.downloadMb,
    diskMb: meta.torch.diskMb,
  };

  const components = meta.components.map((m) => {
    const repo = need(env, m.envRepo, problems);
    const commit = commitOf(m.envCommit);
    const c = {
      id: m.id,
      name: m.name,
      repo,
      commit,
      license: m.license,
      expectNode: m.expectNode,
      downloadMb: m.downloadMb,
      diskMb: m.diskMb,
      source: source(archiveUrl(repo, commit)),
    };
    if (m.extra) {
      const eRepo = need(env, m.extra.envRepo, problems);
      const eCommit = commitOf(m.extra.envCommit);
      c.extra = {
        name: m.extra.name,
        repo: eRepo,
        commit: eCommit,
        expectNode: m.extra.expectNode,
        source: source(archiveUrl(eRepo, eCommit)),
      };
    }
    return c;
  });

  const manifest = {
    _generated: GENERATED_NOTE,
    schemaVersion: meta.schemaVersion,
    manifestVersion: existing?.manifestVersion ?? "",
    allowedHosts: meta.allowedHosts,
    python,
    comfyui,
    torch,
    components,
  };
  return { manifest, missing, problems };
}

/**
 * Rust の `Manifest::validate()` と同じ検査(許可ホスト・https・SHA-256・サイズ・ID)。
 * @returns {string[]}
 */
export function validateManifest(m) {
  const problems = [];
  const hosts = m.allowedHosts;
  const sources = [
    ["python windows", m.python["windows-x64"]],
    ["python macos", m.python["macos-arm64"]],
    ["comfyui", m.comfyui.source],
  ];
  for (const c of m.components) {
    sources.push([c.id, c.source]);
    if (c.extra) sources.push([c.extra.name, c.extra.source]);
  }
  for (const [name, s] of sources) {
    if (!urlAllowed(s.url, hosts)) problems.push(`${name}: 許可されていない取得元です`);
    if (!HEX64.test(s.sha256)) problems.push(`${name}: SHA-256 は小文字 16 進 64 桁`);
    if (!(s.sizeBytes > 0)) problems.push(`${name}: サイズが 0`);
  }
  if (!urlAllowed(m.torch.windowsIndex, hosts)) problems.push("PyTorch のインデックスが許可されていません");
  const ids = m.components.map((c) => c.id);
  if (new Set(ids).size !== ids.length) problems.push("コンポーネント ID が重複しています");
  if (ids.some((id) => !/^[a-z0-9-]+$/.test(id))) problems.push("コンポーネント ID に使えない文字があります");
  if (!m.comfyui.source.url.includes(m.comfyui.commit)) problems.push("comfyui: URL がコミットに固定されていません");
  for (const c of m.components) {
    if (!c.source.url.includes(c.commit)) problems.push(`${c.id}: URL がコミットに固定されていません`);
    if (c.extra && !c.extra.source.url.includes(c.extra.commit)) {
      problems.push(`${c.extra.name}: URL がコミットに固定されていません`);
    }
  }
  return problems;
}

/** キー順を組み立て順で固定し、インデント 2・末尾に改行の JSON にする(LF)。 */
export function stringifyManifest(m) {
  return `${JSON.stringify(m, null, 2)}\n`;
}

/** manifestVersion と `_generated` を除いた内容(内容が変わったかの比較用)。 */
function contentOf(m) {
  const rest = { ...m };
  delete rest.manifestVersion;
  delete rest._generated;
  return JSON.stringify(rest);
}

/**
 * `manifestVersion` を決める。内容が既存と同じなら既存の値のまま、変わっていれば進める。
 * @param {{ manifest: any, existing?: any, now: Date, override?: string }} input
 * @returns {string}
 */
export function decideManifestVersion({ manifest, existing, now, override }) {
  if (override) return override;
  if (existing && contentOf(existing) === contentOf(manifest) && existing.manifestVersion) {
    return existing.manifestVersion;
  }
  return nextManifestVersion(existing?.manifestVersion, now);
}

// ---------------------------------------------------------------- ネットワーク(生成・--verify のみ)

/** タグが指すコミットを `git ls-remote` で解決する(注釈付きタグは `^{}` の値)。 */
function resolveTagCommitViaGit(tag, repo) {
  const out = execFileSync("git", ["ls-remote", "--tags", repo, `refs/tags/${tag}`, `refs/tags/${tag}^{}`], {
    encoding: "utf8",
    timeout: 60_000,
  });
  /** @type {Record<string, string>} */
  const refs = {};
  for (const line of out.split("\n")) {
    const [sha, ref] = line.trim().split(/\s+/);
    if (sha && ref) refs[ref] = sha;
  }
  const sha = refs[`refs/tags/${tag}^{}`] ?? refs[`refs/tags/${tag}`];
  if (!sha) throw new Error("タグが見つかりません");
  return sha;
}

/** 許可ホストの https のみ取得する(リダイレクト先も確認)。SHA-256 とサイズを返す。 */
async function fetchHash(url, allowedHosts) {
  let current = url;
  for (let i = 0; i <= MAX_REDIRECTS; i++) {
    if (!urlAllowed(current, allowedHosts)) throw new Error(`許可されていない取得元です: ${current}`);
    const res = await fetch(current, {
      redirect: "manual",
      signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
      headers: { "user-agent": "gazooo-manifest-generator" },
    });
    if (res.status >= 300 && res.status < 400) {
      const loc = res.headers.get("location");
      if (!loc) throw new Error(`リダイレクト先がありません: ${current}`);
      current = new URL(loc, current).toString();
      continue;
    }
    if (!res.ok || !res.body) throw new Error(`取得に失敗しました(HTTP ${res.status}): ${current}`);
    const hash = createHash("sha256");
    let size = 0;
    for await (const chunk of res.body) {
      size += chunk.length;
      if (size > MAX_BYTES) throw new Error(`サイズが上限(${MAX_BYTES} バイト)を超えました: ${current}`);
      hash.update(chunk);
    }
    return { sha256: hash.digest("hex"), sizeBytes: size };
  }
  throw new Error(`リダイレクトが多すぎます: ${url}`);
}

// ---------------------------------------------------------------- 実行

function paths() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  return {
    env: path.join(root, "tools/comfyui/versions.env"),
    meta: path.join(root, "tools/comfyui/manifest-meta.json"),
    manifest: path.join(root, "src-tauri/src/services/engine_install/manifest.json"),
  };
}

function readJson(p) {
  try {
    return JSON.parse(readFileSync(p, "utf8"));
  } catch {
    return undefined;
  }
}

function fail(lines) {
  for (const l of lines) console.error(`ERROR: ${l}`);
  process.exit(1);
}

async function main() {
  const args = process.argv.slice(2);
  const mode = args.includes("--check") ? "check" : args.includes("--verify") ? "verify" : "generate";
  const vi = args.indexOf("--manifest-version");
  const override = vi >= 0 ? args[vi + 1] : undefined;
  if (vi >= 0 && !override) fail(["--manifest-version に値がありません"]);

  const p = paths();
  const env = parseEnv(readFileSync(p.env, "utf8"));
  const meta = JSON.parse(readFileSync(p.meta, "utf8"));
  const existing = readJson(p.manifest);
  const now = new Date();

  if (mode === "check") {
    if (!existing) fail(["manifest.json を読み取れません。npm run engine-manifest で生成してください"]);
    const { manifest, missing, problems } = buildManifest({ env, meta, existing });
    if (missing.length > 0) {
      problems.push(...missing.map((u) => `SHA-256 が未取得のソースがあります(npm run engine-manifest で取得が必要): ${u}`));
    }
    if (problems.length > 0) fail(problems);
    manifest.manifestVersion = decideManifestVersion({ manifest, existing, now });
    const problems2 = validateManifest(manifest);
    if (problems2.length > 0) fail(problems2);
    const expected = stringifyManifest(manifest);
    const actual = readFileSync(p.manifest, "utf8");
    if (expected !== actual) {
      fail([
        "manifest.json が versions.env / manifest-meta.json と一致しません(手で編集した、または versions.env を変えて再生成していません)。npm run engine-manifest を実行して差分を確認してください",
      ]);
    }
    console.log("OK: manifest.json は versions.env と一致しています");
    return;
  }

  if (mode === "verify") {
    if (!existing) fail(["manifest.json を読み取れません"]);
    const problems = [];
    const sources = [
      ["python windows", existing.python["windows-x64"]],
      ["python macos", existing.python["macos-arm64"]],
      ["comfyui", existing.comfyui.source],
    ];
    for (const c of existing.components) {
      sources.push([c.id, c.source]);
      if (c.extra) sources.push([c.extra.name, c.extra.source]);
    }
    for (const [name, s] of sources) {
      try {
        const h = await fetchHash(s.url, existing.allowedHosts);
        if (h.sha256 !== s.sha256 || h.sizeBytes !== s.sizeBytes) {
          problems.push(`${name}: 取得した内容が manifest.json と違います(sha256=${h.sha256}, size=${h.sizeBytes})`);
        } else {
          console.log(`OK: ${name}`);
        }
      } catch (e) {
        problems.push(`${name}: ${e instanceof Error ? e.message : e}`);
      }
    }
    try {
      const sha = resolveTagCommitViaGit(existing.comfyui.tag, env[meta.comfyui.envRepo]);
      if (sha !== existing.comfyui.commit) {
        problems.push(`comfyui: タグ ${existing.comfyui.tag} が指すコミット(${sha})が manifest.json(${existing.comfyui.commit})と違います`);
      } else {
        console.log(`OK: タグ ${existing.comfyui.tag} → ${sha}`);
      }
    } catch (e) {
      problems.push(`comfyui: タグの確認に失敗しました: ${e instanceof Error ? e.message : e}`);
    }
    if (problems.length > 0) fail(problems);
    return;
  }

  // 生成
  const { manifest, missing, problems } = buildManifest({
    env,
    meta,
    existing,
    resolveTagCommit: resolveTagCommitViaGit,
  });
  if (problems.length > 0) fail(problems);

  // 取得が必要なソース(URL が新規・変更)だけ取得する。comfyui の URL はコミット確定後に組み立て直す。
  if (missing.length > 0) {
    const fetched = new Map();
    for (const url of missing) {
      console.log(`取得: ${url}`);
      try {
        fetched.set(url, await fetchHash(url, meta.allowedHosts));
      } catch (e) {
        fail([e instanceof Error ? e.message : String(e)]);
      }
    }
    const fill = (s) => {
      const h = fetched.get(s.url);
      if (h) Object.assign(s, h);
    };
    fill(manifest.python["windows-x64"]);
    fill(manifest.python["macos-arm64"]);
    fill(manifest.comfyui.source);
    for (const c of manifest.components) {
      fill(c.source);
      if (c.extra) fill(c.extra.source);
    }
  }

  manifest.manifestVersion = decideManifestVersion({ manifest, existing, now, override });
  const invalid = validateManifest(manifest);
  if (invalid.length > 0) fail(invalid);

  const text = stringifyManifest(manifest);
  const before = existing ? readFileSync(p.manifest, "utf8") : "";
  if (text === before) {
    console.log("変更なし: manifest.json は最新です");
    return;
  }
  writeFileSync(p.manifest, text, "utf8");
  console.log(`更新しました: manifest.json(manifestVersion ${manifest.manifestVersion})。git diff で差分を確認してください`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => fail([e instanceof Error ? e.message : String(e)]));
}
