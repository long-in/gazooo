# 41. versions.env からエンジンマニフェストを自動生成する(v0.36、M36)

> ステータス: 🟢 承認済み(2026-11-23。07 Q-319〜327 は推奨どおり)。段階 A・C を実装(08 §7・23 §3 への追記は反映済み)。段階 B(`--verify`)は同じスクリプトに実装するが、週次の CI ジョブは Q-326 のとおり追加しない。

## 1. 目的

ComfyUI のバージョン・カスタムノードのコミットは、現在 **2 か所** に書かれている。

| 場所 | 使う人 | 内容 |
|---|---|---|
| `tools/comfyui/versions.env`(08) | 開発用スクリプト(`comfyui:setup`) | タグ・コミット・PyTorch のバージョン |
| `src-tauri/src/services/engine_install/manifest.json`(23) | アプリ内インストーラ | 上記に加え、**取得 URL・SHA-256・サイズ・ライセンス・検証ノード名** など |

現状は、両者の一致を Rust のテスト(`manifest.rs` の `matches_versions_env`)で検証している。この方式には次の問題がある。

- **更新が手作業**: コミットを 1 つ上げるたびに、`manifest.json` の `commit` / `source.url` / `source.sha256` / `source.sizeBytes` の 4 項目を手で直す。SHA-256 は zip を手元で取得して計算するしかなく、ミスしやすい。
- **ミスに気づくのが遅い**: ずれは `cargo test` で初めて分かる。しかも「ずれた」ことしか分からず、直し方は人が調べる。
- **ノードの追加が重い**: 39(WD14 Tagger)の追加では、`versions.env`・`setup.ps1` / `setup.sh`・`manifest.json`・テストの 4 か所を同時に直した(39 §7)。

そこで、`versions.env` を **バージョンの唯一の基準(single source of truth)** とし、`manifest.json` のうち **機械的に決まる部分を生成する**。人が書くのは、機械で決まらない情報(ライセンス・検証ノード名・目安サイズ)だけにする。

スコープ外: `versions.env` の廃止(開発用スクリプトが読むため残す。§8 の将来案を参照)、モデルカタログ(12)の生成、PyTorch の SHA-256 検証(23 §9 のとおり固定バージョン + 公式インデックスのまま)、ComfyUI の自動更新(23 §1 のとおり対象外)。

## 2. 現状の整理(manifest.json の各項目の出どころ)

| 項目 | 出どころ | 生成できるか |
|---|---|---|
| `comfyui.tag` | `COMFYUI_TAG` | ○(そのまま) |
| `comfyui.commit` | タグが指すコミット | ○(`git ls-remote` で解決。注釈付きタグは `^{}` の値) |
| `components[].commit`・`extra.commit` | `*_COMMIT` | ○(そのまま) |
| `components[].repo`・`extra.repo` | `*_REPO` | ○(そのまま) |
| `*.source.url` | `<repo>/archive/<commit>.zip` | ○(組み立てるだけ) |
| `*.source.sha256`・`sizeBytes` | 上記 URL の zip | ○(取得して計算) |
| `torch.version` / `torchvision` / `torchaudio` / `windowsIndex` | `TORCH_*` / `TORCH_CUDA_INDEX` | ○(そのまま) |
| `python.version`(3.12.16)・`python.*.url`・`sha256` | python-build-standalone のリリース | △ `versions.env` には **メジャー.マイナー(`PYTHON_VERSION=3.12`)しか無い**。パッチ版とリリース日付は §4.3 で扱う |
| `license`・`expectNode`・`name`・`downloadMb`・`diskMb`・`id` | 人が決める | ✕(別の静的ファイルに置く。§4.2) |
| `allowedHosts`・`schemaVersion` | 人が決める | ✕(静的) |
| `manifestVersion` | 変更のたびに更新 | △(§4.5) |

## 3. 方式の比較

| 案 | 内容 | 評価 |
|---|---|---|
| **A(推奨)** `versions.env` + 静的メタ → 生成 | `versions.env`(基準)と `manifest-meta.json`(静的な情報)から、`manifest.json` を Node のスクリプトで生成する。CI で「生成結果とコミット済みの `manifest.json` が一致するか」を検査 | 手作業の 4 項目がなくなる。`manifest.json` は従来どおりコミットするので、Rust 側(`include_str!`)・ビルド・実行時の挙動は **変わらない**。オフラインでもビルドできる |
| B ビルド時に生成(`build.rs`) | `cargo build` のたびに生成する | **不採用**。ビルドがネットワークに依存する(SHA-256 の取得)。再現性と AGENTS.md §2(ビルドのタイミングで変わらない)に反する |
| C 逆向き(`manifest.json` を基準にし、`versions.env` を生成) | 基準をマニフェストへ移す | 開発用スクリプトが bash / PowerShell で `versions.env` を直接読むため、生成物のコミットが必要になり、結局二重になる。得るものが少ない |
| D 現状維持 | 手作業 + 一致テスト | 手間が減らない |

→ **案 A** を採用する。

## 4. 設計(案 A)

### 4.1 ファイル構成

```text
tools/comfyui/
  versions.env              # 基準(人が編集。従来どおり)
  manifest-meta.json        # 新規: 機械で決まらない静的な情報(人が編集)
tools/
  gen-engine-manifest.mjs   # 新規: 生成・検査スクリプト(Node 標準 API のみ。追加の依存なし)
  gen-engine-manifest.test.ts  # 新規: Vitest(オフラインで動く純関数のテスト)
src-tauri/src/services/engine_install/
  manifest.json             # 生成物(コミットする。先頭の `_generated` で生成物と分かるようにする)
  manifest.rs               # 変更は最小限(§6)
```

- **追加の npm / crate 依存は入れない**(`node:crypto`・`node:fs`・組み込みの `fetch`、`git` コマンドのみ。AGENTS.md §2 の固定対象が増えない)。Node 26 以上が前提(26)なので `fetch` が使える。
- スクリプトは **開発者の端末(Windows / macOS)** で実行する。`git` が必要(開発者はすでに持っている。**アプリのインストーラは引き続き git 不要**。23 §3)。

### 4.2 `manifest-meta.json`(静的な情報)

`manifest.json` の構造から、**生成できる項目を除いたもの**。コンポーネントの `id` をキーに、`versions.env` のキー名を結びつける。

```json
{
  "schemaVersion": 1,
  "allowedHosts": ["github.com", "objects.githubusercontent.com", "…"],
  "python": { "...": "§4.3 のとおり" },
  "comfyui": { "license": "GPL-3.0", "envTag": "COMFYUI_TAG", "envRepo": "COMFYUI_REPO" },
  "torch": { "license": "BSD-3-Clause", "downloadMb": 3500, "diskMb": 8000 },
  "components": [
    {
      "id": "gguf",
      "name": "ComfyUI-GGUF",
      "license": "Apache-2.0",
      "expectNode": "UnetLoaderGGUF",
      "downloadMb": 20,
      "diskMb": 60,
      "envCommit": "GGUF_NODE_COMMIT",
      "envRepo": "GGUF_NODE_REPO"
    },
    {
      "id": "face",
      "...": "...",
      "envCommit": "IMPACT_PACK_COMMIT",
      "envRepo": "IMPACT_PACK_REPO",
      "extra": {
        "name": "ComfyUI-Impact-Subpack",
        "expectNode": "UltralyticsDetectorProvider",
        "envCommit": "IMPACT_SUBPACK_COMMIT",
        "envRepo": "IMPACT_SUBPACK_REPO"
      }
    }
  ]
}
```

- **新しいノードを足す手順**(例: 39 の WD14 Tagger): `versions.env` に `*_COMMIT` / `*_REPO` を足す → `manifest-meta.json` に 1 件足す → `npm run engine-manifest` で `manifest.json` を再生成する。`commit` / `url` / `sha256` / `sizeBytes` は手で書かない。
- 並び順は `manifest-meta.json` の配列順(= 現行の `manifest.json` と同じ)。

### 4.3 Python の扱い(論点 Q-322)

`manifest.json` の `python` は、パッチ版(`3.12.16`)と python-build-standalone のリリース日付(`20261110`)に依存し、URL・SHA-256 は OS ごとに 2 つある。`versions.env` には `PYTHON_VERSION=3.12` しか無く、開発用スクリプトはシステムの Python を使うため、**パッチ版とリリース日付は開発用スクリプトに関係がない**。

推奨は **`manifest-meta.json` に `python.patchVersion` と `python.releaseTag` を置き**、URL(ファイル名の組み立て)・SHA-256・サイズをスクリプトが生成する案。`versions.env` には追加しない(開発用スクリプトが使わないキーを増やさない)。生成時に `PYTHON_VERSION`(3.12)と `patchVersion`(3.12.16)の整合を検査する(現行のテストと同じ条件)。

ファイル名は python-build-standalone の規則(`cpython-<版>+<日付>-<target>-install_only.tar.gz`)で組み立てる。規則が変わって 404 になった場合は、生成が失敗する(§4.4)ので気づける。

### 4.4 生成スクリプト `tools/gen-engine-manifest.mjs`

```text
node tools/gen-engine-manifest.mjs            # 生成(manifest.json を書き換える)
node tools/gen-engine-manifest.mjs --check    # 検査のみ(書き換えない。差があれば exit 1)
node tools/gen-engine-manifest.mjs --verify   # 全ソースを再取得して SHA-256 を照合(exit 1 で不一致)
```

npm scripts(`package.json`): `engine-manifest`(生成)、`engine-manifest:check`(検査)、`engine-manifest:verify`(再取得して照合)。

**生成の流れ**

1. `versions.env` と `manifest-meta.json` を読む。キーの欠落・形式不正(コミットが 40 桁の 16 進でない等)はエラー。
2. `COMFYUI_TAG` をコミットに解決する。**既存の `manifest.json` の `comfyui.tag` が同じなら、既存の `comfyui.commit` を使う**(ネットワークを使わない。冪等にするため)。タグが変わったときだけ `git ls-remote --tags <repo> <tag> <tag>^{}` で解決する(注釈付きタグは `^{}` の値を採る)。解決できなければエラー。タグが指すコミットの再確認は `--verify` で行う。
3. 各ソースの URL を組み立てる(`<repo>/archive/<commit>.zip`、ComfyUI も **コミット** で固定する。現行どおり)。
4. SHA-256・サイズは **既存の `manifest.json` を参照**する。**URL が同じなら既存の値をそのまま使い、再取得しない**(= 通常の更新は、変わったソースだけ取得する)。URL が新規・変更なら取得して計算する。
5. 取得は許可ホスト(`allowedHosts`)の **https のみ**。リダイレクト先も許可ホストかを確認する(23 §6 の方針をスクリプトにも適用)。タイムアウトと最大サイズ(1 ファイル 200MB)を設ける。
6. 結果を **キー順を固定した JSON(インデント 2、末尾に改行、LF)** で書き出す。**改行・並びを固定**し、Windows / macOS で同じバイト列になるようにする(差分が出ない)。
7. 書き出す前に、Rust 側の `validate()` と同じ検査(許可ホスト・https・SHA-256 は小文字 16 進 64 桁・サイズ > 0・ID の重複と文字種)をスクリプト側でも行う。

**`--check`(オフライン)**

ネットワークを使わず、次を検査する。

- `manifest.json` を再生成した結果(手順 4 で既存の SHA-256 を使い回す)が、コミット済みの `manifest.json` と **一致する**。
- 一致しない場合は、`npm run engine-manifest` の実行を案内する(実装では、`versions.env` のコミットを変えて再生成していない場合は「SHA-256 が未取得のソースがある」と、その URL を表示する。それ以外の不一致は「`manifest.json` が `versions.env` / `manifest-meta.json` と一致しない」と表示する)。
- **限界**: SHA-256 とサイズは既存の `manifest.json` の値を信頼するため、URL が変わっていない限り、これらだけを手で書き換えても `--check` は通る。実物との照合は `--verify` の役割である(§10)。
- URL が新規・変更なのに SHA-256 が既存に無い場合は「`npm run engine-manifest` で取得が必要」と表示して失敗する。
- `COMFYUI_TAG` のコミット解決(手順 2)は **ネットワークが要る** ため、`--check` では行わず、`manifest.json` の `comfyui.commit` が `source.url` に含まれることだけを確認する(現行テスト `source_urls_are_pinned_to_the_commit` と同じ)。タグ → コミットの正しさは `--verify` で確認する。

**`--verify`(オンライン)**

全ソースを取得し直して、SHA-256・サイズが `manifest.json` と一致するか、タグが指すコミットが `comfyui.commit` と一致するかを確認する。**GitHub のアーカイブ zip は、通常は同じコミットなら同じ内容になるが、保証されない**(GitHub 側の変更でバイト列が変わることがある)ため、`--verify` は **失敗しても自動的にはビルドを止めない運用**にする(§5)。

### 4.5 `manifestVersion`(論点 Q-323)

現在 `2026.11.1` のように手で書いている。生成スクリプトは、**内容(`manifestVersion` を除く部分)が変わったときだけ** 日付版を進める。

- 同じ月の 2 回目以降は末尾を +1(`2026.11.1` → `2026.11.2`)、月が替われば `2026.12.1`。
- 日付は **ローカルの日付** を使う(スクリプトの実行日)。`--check` は日付に依存させない(内容が同じなら一致扱い)。

推奨は自動更新。手で上げたい場合はスクリプトの `--manifest-version <値>` で上書きできる。

### 4.6 生成物であることの明示

`manifest.json` の先頭に `"_generated"` を置く(JSON にコメントが書けないため)。

```json
{
  "_generated": "tools/gen-engine-manifest.mjs が versions.env と manifest-meta.json から生成します。手で編集せず、npm run engine-manifest を実行してください(docs/41)",
  "schemaVersion": 1,
  ...
}
```

`Manifest`(Rust)は `deny_unknown_fields` を使っていないため、`_generated` はそのまま読み飛ばされる(`manifest.rs` の構造体は変更不要。実装時にテストで確認する)。

## 5. CI

| ジョブ | 内容 | 失敗の扱い |
|---|---|---|
| 既存の CI(`ci.yml`) | `node tools/gen-engine-manifest.mjs --check` を、`check-version` と同じ位置に追加する(オフライン・高速) | **必須(落ちたら CI 失敗)** |
| 既存の `cargo test --locked` | `manifest.rs` のテストは **残す**(§6)。生成と二重の検査になるが、Rust 側だけでも整合が確認できる安全網とする | 必須 |
| 新規: 週次(+ 手動)`engine-manifest-verify`(任意) | `--verify` でアーカイブの内容が変わっていないか確認 | **通知のみ**(ビルドは止めない。失敗は Issue / ログで知る) |

- GitHub Actions の追加は **別の変更として扱う**(06 の CI 方針に従い、必須のジョブは `--check` のみ。週次ジョブの追加は §7 の段階 B で、実施するかを Q-326 で確認する)。
- `--check` は `git` もネットワークも使わないため、Windows / macOS のどちらのランナーでも動く。

## 6. Rust 側の変更(最小)

- `manifest.rs` と `manifest.json` の読み込み(`include_str!`)・`validate()` は **変更しない**。実行時の挙動・IPC・DB への影響は **なし**。
- 既存のテスト(`matches_versions_env`・`source_urls_are_pinned_to_the_commit` 等)は **そのまま残す**。生成スクリプトが誤って壊した場合の最後の砦になる。
- 追加するテスト: `_generated` を含む JSON が `Manifest::embedded()` で読めること(= 将来 `deny_unknown_fields` を付けて壊す変更を防ぐ)。

## 7. 実装の段階

| 段階 | 内容 |
|---|---|
| A | `manifest-meta.json` の作成、`gen-engine-manifest.mjs`(生成・`--check`)、Vitest(URL の組み立て・検査・JSON の整形・`manifestVersion` の進め方を **ネットワークなし** で)、`package.json` の scripts、CI に `--check` を追加。**現在の `manifest.json` を最初の生成結果と一致させる**(意味のある差が出ないこと = 受け入れ基準 a) |
| B | `--verify` と、週次の確認ジョブ(任意) |
| C | ドキュメントの追記(08 §7・23 §3。§9) |

段階 A だけで、手作業の削減と CI の整合検査は達成できる。

## 8. 将来案(今回は行わない)

- 開発用スクリプト(`comfyui:setup`)を、`manifest.json` を読んで導入する形にして、`versions.env` を廃止する案。スクリプトが bash / PowerShell で、JSON を扱うには Node 経由(`tools/comfyui/run.mjs` はすでに Node)にする必要がある。その場合、`git` を使う現行の導入経路が不要になる。**開発用スクリプトを今後も残すかどうか**(前の議論)とあわせて、インストーラの実機検証(特に macOS)が済んだ段階で決める。
- ソースの取得元が GitHub アーカイブである点の強化(例: 署名付きタグの検証)。現状は「コミット固定 + SHA-256」で、SHA-256 は **改ざんの検出ではなく再現性の確認**(初回の取得時の値を信頼している)であることに注意する(§10 リスク)。

## 9. 他の資料への追記案(承認後に反映)

- **08 §7(更新手順)**: 手順を次のように変える。
  1. `versions.env` の値を変更する。
  2. **`npm run engine-manifest` を実行して `manifest.json` を再生成する(41)。差分(`commit` / `url` / `sha256` / `sizeBytes`)をレビューする。**
  3. `npm run comfyui:setup` を再実行して動作確認(従来どおり)。
  4. アプリと結合して動作確認。
  5. API 変更があればワークフローテンプレートを更新する(02 R-01)。
- **23 §3**: マニフェストの場所の記述が古い。`src-tauri/resources/engine-manifest.json` → **`src-tauri/src/services/engine_install/manifest.json`**(実装済みの場所は §168 の記述が正しい)に直し、「両者の一致をテストで検証する」に **「`manifest.json` は 41 のスクリプトで `versions.env` から生成し、CI で `--check` する」** を加える。§159 の「一致テスト」の記述も 41 を参照する。
- **docs/README.md**: 41 の行とマイルストーン M36 を追加する(本変更で反映済み)。

## 10. リスクと注意

| リスク | 対策 |
|---|---|
| 初回取得の zip が改ざんされていても気づけない(SHA-256 は取得時点の値を信頼する) | 取得元はコミット固定の URL(許可ホストの https のみ)。生成後の **差分は PR でレビュー** する。コミットは各リポジトリで確認してから `versions.env` に書く(現行の運用どおり)。ここで強化する範囲ではない |
| GitHub のアーカイブ zip のバイト列が変わる | `--verify` は通知のみ。変わったと分かった場合は、再生成して差分をレビューする。インストーラ側は SHA-256 不一致で導入を中止するので、利用者に不正なものが入ることはない(安全側に倒れる) |
| 生成スクリプトのバグでマニフェストが壊れる | Rust の `validate()` と既存のテストが残る。`--check` で現行の `manifest.json` と一致することを受け入れ基準にする |
| 開発者が `manifest.json` を手で編集する | `commit` / `url` / 構造の手編集は、`--check` が CI で落とす。`_generated` で注意を促す。**`sha256` / `sizeBytes` だけの手編集は、`--check`(オフライン)では検出できない**(これらは既存の値を再利用する設計で、正しさはネットワークでしか確かめられない)。検出は `--verify`(手元で実行、または Q-326 の週次ジョブ)で行う。インストーラ側は SHA-256 が実物と違えば導入を中止する(安全側)ので、利用者に不正なものが入ることはない |
| Windows と macOS で出力が変わる(改行・パス) | LF 固定・キー順固定。`.gitattributes` の扱いはリポジトリの既存設定に従う(実装時に確認) |
| `git` が無い環境で生成できない | 生成は開発者のみ。`--check` と CI は `git` 不要 |

## 11. 受け入れ基準

- (a) 段階 A の初回生成で、現在の `manifest.json` と **意味のある差が出ない**(`manifestVersion` と `_generated` のみ)。
- (b) `versions.env` の `GGUF_NODE_COMMIT` だけを変えて `--check` を実行すると、「`npm run engine-manifest` が必要」と表示して失敗する。生成すると `commit` / `url` / `sha256` / `sizeBytes` が更新され、`--check` が通る。
- (c) `versions.env` を変えていない状態で生成しても、**ネットワーク取得が 0 回**で、ファイルに差分が出ない(冪等)。
- (d) `COMFYUI_TAG` を変えると、`comfyui.commit` と URL・SHA-256・サイズが更新される。
- (e) Vitest(ネットワークなし)で、URL の組み立て・コミット形式の検査・許可ホスト以外の拒否・JSON の整形(キー順・LF・末尾改行)・`manifestVersion` の進め方を検証する。Windows / macOS の CI で通る。
- (f) Rust の `cargo test` が、変更後の `manifest.json` で通る(`_generated` を含めても読み込める)。
- (g) アプリの実行時の挙動・IPC・DB は変わらない(差分が `tools/`・`package.json`・CI・`manifest.json` に限られる)。

## 12. 実装時の変更ファイル(予定)

- 新規: `tools/gen-engine-manifest.mjs`、`tools/gen-engine-manifest.test.ts`、`tools/comfyui/manifest-meta.json`
- 変更: `src-tauri/src/services/engine_install/manifest.json`(生成物化)、`package.json`(scripts のみ。依存の追加なし)、`.github/workflows/ci.yml`(`--check` の追加)、`src-tauri/src/services/engine_install/manifest.rs`(テストの追加のみ)
- 資料: 08 §7・23 §3 の更新(§9)
