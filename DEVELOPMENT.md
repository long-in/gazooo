# gazooo 開発者向けメモ

利用者向けの説明は [README.md](README.md) を参照してください。開発は「設計 → 人間レビュー → 実装」のスペック駆動で進めています。方針は [AGENTS.md](AGENTS.md)、設計資料は [docs/](docs/README.md) にあります。

## 必要なもの

- Node.js 26 以上(npm 10 以上)
- Rust(`rust-toolchain.toml` で固定。rustup が自動で選びます)
- Windows: Visual Studio Build Tools 2022(「C++ によるデスクトップ開発」)、WebView2(Windows 11 は標準搭載)
- macOS: Xcode Command Line Tools

## コマンド

| コマンド | 内容 |
|---|---|
| `npm ci` | 依存をロックファイルどおりに導入 |
| `npm run tauri dev` | アプリを開発モードで起動(フロントは HMR、Rust は自動再ビルド) |
| `npm run dev` | フロントエンドだけをブラウザで確認(Tauri の機能は動かない) |
| `npm run lint` / `npm run typecheck` / `npm test` | ESLint / 型チェック / Vitest |
| `cargo test --locked`(`src-tauri/` で) | Rust のテスト(`cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --check` も) |
| `npm run tauri build` | リリースビルドとインストーラ生成(`src-tauri/target/release/bundle/`) |
| `node tools/check-version.mjs` | package.json / Cargo.toml / tauri.conf.json のバージョン整合を確認 |

ComfyUI が必要なテストは通常は実行されません。ComfyUI を起動して `cargo test -- --ignored real_comfyui`(`src-tauri/` で)で実行します。

## 依存バージョンの固定

ビルドのたびにバージョンが変わらないよう、依存は完全一致で固定し、`package-lock.json` と `src-tauri/Cargo.lock` をコミットしています。詳細は [AGENTS.md](AGENTS.md) を参照してください。

## CI・リリース

- `develop` への PR / push で、Windows と macOS の両方で lint・型チェック・テストを実行します(`.github/workflows/ci.yml`)。`develop` への push では、インストーラも生成します。
- タグ `vX.Y.Z` を push すると、インストーラをビルドして GitHub Releases に **下書き** を作ります(`.github/workflows/release.yml`)。公開前に [リリース前チェックリスト](docs/09-release-checklist.md) を実施してください。
