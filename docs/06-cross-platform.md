# 06. クロスプラットフォーム・ビルド・CI

## 1. 対応マトリクス

| 項目 | Windows 10/11 (x64) | macOS 13+ (Apple Silicon) |
|---|---|---|
| WebView | WebView2(Evergreen)。インストーラでブートストラップ | WKWebView |
| 配布形式 | NSIS(`.exe`)を第一候補、MSI も可 | `.dmg` / `.app` |
| データ保存先 | `%APPDATA%\<identifier>\` | `~/Library/Application Support/<identifier>/` |
| ログ保存先 | `%LOCALAPPDATA%\<identifier>\logs` | `~/Library/Logs/<identifier>/` |
| 署名 | 将来(コードサイニング証明書) | 将来(Developer ID + 公証) |
| ComfyUI の典型構成 | 公式ポータブル版 + NVIDIA GPU(CUDA) | Python venv + PyTorch MPS |

macOS Intel(x86_64)は MVP では非対象(ビルド自体は可能にしておく)。

## 2. Windows で壊れやすい点と対策

| 問題 | 対策 |
|---|---|
| パス区切り(`\` と `/`) | `std::path::PathBuf` / Tauri path API を使用。DB には `/` 区切りの相対パスを保存し、読み込み時に `PathBuf` へ変換 |
| 長いパス(260 文字) | 画像パスは短く保つ(`YYYY/MM/<uuid>.png`)。ユーザー指定の保存先が長い場合は警告 |
| ファイル名に使えない文字 | ファイル名は UUID ベースで生成し、ユーザー入力を直接使わない。エクスポート名はサニタイズ |
| ファイルロック | 削除時にリトライ(ウイルス対策ソフトの一時ロック対策)。DB は WAL でプロセス内単一接続 |
| `localhost` の IPv6 解決 | 既定値は `127.0.0.1` を使用 |
| SQLite のリンク | `rusqlite` の `bundled` feature で同梱。システム SQLite に依存しない |
| TLS | `reqwest` は rustls + **ring** プロバイダ(OpenSSL にも aws-lc にも依存しない。aws-lc は Windows で CMake/NASM を要求することがある)。`ComfyUiClient::new` が生成前にプロバイダを登録する(未登録だと reqwest が panic するため) |
| 外部プロセス(将来の ComfyUI 起動) | `CREATE_NO_WINDOW` フラグでコンソール非表示、終了時はプロセスツリーごと停止 |
| 改行コード | リポジトリに `.gitattributes`(`* text=auto eol=lf`、`*.bat` は CRLF)を置く |
| 日本語ユーザー名のパス | UTF-8 / UTF-16 変換を意識し、`OsString` 経由で扱う。結合テストに日本語パスを含める |
| WebView2 の `asset:` URL | Windows では `http://asset.localhost`、macOS では `asset://localhost`。`convertFileSrc` を必ず使い、CSP 両方許可 |

## 3. 開発環境

### 共通
- Node.js 26 以上(v0.18、設計: 26。レビュー待ち。従来は 22 LTS 以上。npm 10 以上。`engines` で要件を明示)、Rust 1.99.1(`rust-toolchain.toml` で固定)
- 依存ライブラリは完全一致で固定し、ロックファイルをコミットする(AGENTS.md「依存バージョンの固定」)

### Windows
- Visual Studio Build Tools 2022(「C++ によるデスクトップ開発」)、WebView2 Runtime(Win11 は標準搭載)
- 推奨シェル: PowerShell 7。スクリプトは npm scripts(`package.json`)に寄せ、OS 固有のシェルスクリプトを避ける

### macOS
- Xcode Command Line Tools

### 主要コマンド(`package.json` scripts)

| コマンド | 内容 |
|---|---|
| `npm run dev` | Next.js 開発サーバー(Tauri の `beforeDevCommand` から呼ぶ) |
| `npm run tauri dev` | アプリを開発モードで起動 |
| `npm run build` | Next.js 静的エクスポート(`out/`) |
| `npm run tauri build` | リリースビルド・インストーラ生成 |
| `npm run lint` / `npm test` | ESLint・Vitest |
| `cargo test` / `cargo clippy` | `src-tauri/` で実行 |

## 4. Tauri 設定の要点(`tauri.conf.json`)

- `build.frontendDist: "../out"`、`build.devUrl: "http://localhost:3000"`
- `build.beforeDevCommand: "npm run dev"`、`build.beforeBuildCommand: "npm run build"`
- `app.security.csp` は 02 §3.9 の方針。`assetProtocol.enable: true` + 限定 scope
- `bundle.targets`: `["nsis", "dmg", "app"]`、Windows の WebView2 は `downloadBootstrapper`
- `identifier`: 例 `dev.gazooo.desktop`(07 で確認)

## 5. CI(GitHub Actions)

ワークフローは `.github/workflows/` にある。**すべての Action は、タグではなく 40 桁のコミット SHA で固定する**(AGENTS.md「依存バージョンの固定」と同じ方針。コメントに対応するバージョンを残す)。

### `ci.yml`

- トリガー: `develop` への PR と push。同じ ref の古い実行は自動で中止する。権限は `contents: read` のみ。
- **`check` ジョブ**(`windows-latest` と `macos-latest` の両方。`fail-fast: false`):
  1. Node.js 26(v0.18。従来は 22。`cache: npm`)と、`rust-toolchain.toml` のツールチェーンを導入(`rustup toolchain install`。rustup 1.28 以降は自動導入されないため明示する)。`Swatinem/rust-cache` で `src-tauri/target` をキャッシュ。
  2. `npm ci` → `node tools/check-version.mjs`(バージョンの整合) → `npm run lint` → `npm run typecheck` → `npm test`
  3. `cargo fmt --check` / `cargo clippy --locked --all-targets -- -D warnings` / `cargo test --locked`
  4. ComfyUI 開発用スクリプトの構文検査(Windows: PowerShell の `Parser.ParseFile`、macOS: `bash -n`)。実行はしない(GPU・巨大な依存が必要なため)。
- **`bundle` ジョブ**(`develop` への **push のときだけ**。PR ではインストーラを作らない。R-06 の対策): `npm run tauri build -- --bundles <種別>` を実行し、インストーラをアーティファクトとして保存する(保持 14 日)。
  - Windows: `nsis`(`*-setup.exe`)/ macOS: `app,dmg`(`.dmg`)。種別は OS ごとに明示する(設定の `targets` に依存しない)。

### `release.yml`

- トリガー: タグ `v*`。権限は `contents: write`。
- `verify`: `node tools/check-version.mjs <タグ>` で、タグと `package.json` / `Cargo.toml` / `tauri.conf.json` のバージョンが一致することを確認する。
- `release`: `tauri-apps/tauri-action` で両 OS のインストーラをビルドし、**GitHub Releases に下書きとして登録**する。公開は、09 の手動検証を終えてから人間が行う。
- リリースは、CI が成功した `develop` のコミットから切る(リリース用ワークフローはテストを再実行しない)。

### 署名について(Q-17)

MVP ではコード署名(Windows)・Developer ID 署名と公証(macOS)をしない。Windows は SmartScreen の警告、macOS は Gatekeeper の警告が出る。macOS はアドホック署名のみ行う(未署名のバンドルは Apple Silicon で起動できないことがあるため)。
macOS 15(Sequoia)以降は、警告を右クリック →「開く」では回避できず、「システム設定 → プライバシーとセキュリティ → このまま開く」で許可する。リリースノートと README に明記する。

## 6. 動作検証方針

- 自動: Rust ユニットテスト(DB、ワークフロー生成、エンジン応答のモック、ジョブキュー、ギャラリー、保存先の移動)、フロントのロジックテスト。**両 OS の CI で実行**。
- ComfyUI を必要とするテストは `#[ignore]` とし、起動済みの ComfyUI に対して `cargo test -- --ignored real_comfyui` で手動実行する(08 §6)。実際のゴミ箱を使うテストは `-- --ignored real_os_trash`。
- 手動(リリース前): [09-release-checklist.md](09-release-checklist.md)。Windows 11 / macOS 各 1 台で、インストール → ComfyUI 接続 → SDXL/Flux 生成 → ギャラリー → 再起動後の永続化 → アンインストール。
- エンジンのモック: ComfyUI 互換のモックサーバー(HTTP + WebSocket。`engine/comfyui/testutil.rs`)で、GPU のない CI でも結合テストが動く。

## 7. インストーラ

| 項目 | Windows | macOS |
|---|---|---|
| 形式 | NSIS(`*-setup.exe`)。ユーザー単位のインストール(管理者権限不要) | `.dmg`(中身は `gazooo.app`) |
| WebView2 | 未導入の環境ではブートストラップが取得する | 不要(WKWebView) |
| 署名 | なし(SmartScreen の警告) | **アドホック署名**(`signingIdentity: "-"`)。Developer ID 署名・公証はなし(Gatekeeper の警告)。バンドル全体を署名しないと `codesign --verify` が失敗するため、アドホック署名は必須 |
| 最低 OS | Windows 10 | macOS 13(`minimumSystemVersion`) |
| 言語 | 日本語・英語 | – |
| アンインストール | 設定 → アプリ。**ユーザーデータ(`%APPDATA%\dev.gazooo.desktop` の DB・画像)は残る** | `.app` を削除。データ(`~/Library/Application Support/dev.gazooo.desktop`)は残る |
| アイコン | 原画は `assets/icon.svg`(山の写真フレーム+生成の星)。更新時は `npm run tauri -- icon assets/icon.svg` で再生成し、不要な `android/` `ios/` `64x64.png` は削除する。`src/app/favicon.ico` は `icon.ico` と同一にする | 同左 |
