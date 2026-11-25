# AGENTS.md — gazooo 開発方針

このファイルは本プロジェクトの開発方針を永続化したものです。AI エージェント・人間の開発者ともに必ず従ってください。
方針を変更する場合は、このファイルを更新してレビューを受けてください。

## 1. プロジェクト概要

ローカルで動作する画像生成デスクトップアプリ(Windows / macOS 対応)。
画像生成エンジンには ComfyUI(Stable Diffusion / Flux.1 等)を利用する想定。詳細は `docs/` を参照。

## 2. 技術スタック

| 領域 | 採用技術 |
|---|---|
| デスクトップシェル | Tauri 2(Rust) |
| フロントエンド | Next.js 16(静的エクスポート `output: "export"`)+ React 19 |
| スタイリング | Tailwind CSS 4 + shadcn/ui |
| DB | SQLite(`rusqlite`、`bundled` feature 必須) |
| 対応 OS | Windows 10/11、macOS(Apple Silicon)。Linux は対象外(壊さない努力はする) |

制約:
- Next.js は **静的エクスポート前提**。サーバー機能(API Routes, Server Actions, SSR, ISR, `next/image` の最適化等)は使用禁止。
- バックエンド処理・DB・ファイル I/O・外部プロセス制御は **Rust 側** に置く。フロントエンドは Tauri の `invoke` / イベント経由でのみ利用する。
- **Windows で動作すること** を常に維持する(詳細は `docs/06-cross-platform.md`)。OS 依存コードは分離し、パス操作は `std::path` / Tauri path API を使い、区切り文字をハードコードしない。

### 依存バージョンの固定

ビルドのタイミングでライブラリのバージョンが変わらないよう、次を守る。

- `package.json` の依存は **完全一致のバージョン**(`^` / `~` を付けない)。`.npmrc` の `save-exact=true` により `npm install <pkg>` も固定で入る。
- `src-tauri/Cargo.toml` の直接依存は **`=x.y.z`** で固定する。
- Rust ツールチェーンは `rust-toolchain.toml` で固定する。パッケージマネージャは **npm**(Node 26 以上に付属の npm 10 以上)とし、`package.json` の `engines` で要件を示す。pnpm / yarn は使わない(`package-lock.json` が複数種類になるため)。
- `package-lock.json` と `src-tauri/Cargo.lock` は必ずコミットする。ビルド・CI では `npm ci` と `cargo ... --locked` を使う。
- バージョン更新は、依存更新を目的とした変更として個別に行う(`npm install <pkg>@<version>` / `cargo update -p <crate>` → 固定値を書き換え → lint・テスト・ビルドで検証)。
- 組み合わせ制約がある依存(`rusqlite` と `rusqlite_migration`、TypeScript と typescript-eslint 等)は `docs/02-architecture.md` の更新方針に従う。

## 3. 開発プロセス: スペック駆動開発

必ず次の順序で進める。**設計レビュー完了前に実装コードを書いてはならない。**

1. **設計(Spec)**: ルート直下の `docs/` に設計資料を作成・更新する。
2. **人間によるレビュー**: ユーザーが `docs/` をレビューし、承認する。
3. **実装**: 承認後にのみ実装へ移る。実装は承認済みの設計に従う。
4. **設計の同期**: 実装中に設計との差異が必要になった場合は、先に `docs/` を更新して(または更新案を提示して)承認を得てから実装する。

### 2 回目以降の変更(仕様変更・機能追加)

- 仕様変更・機能追加の指示を受けたら、**まず `docs/` を更新**し、レビュー待ちにする。承認が出るまで実装しない(軽微な修正・バグ修正は、設計に影響しない範囲で直接対応してよい)。
- 設計資料の承認状態は `docs/README.md` のステータス表で管理する。

### docs の構成

`docs/README.md` が索引とレビューステータス。各資料は日本語で記述する。
ファイルを追加・削除したら `docs/README.md` も更新すること。

## 4. Git / ブランチ運用

- ベースブランチは **`develop`**。
- 作業は常に `develop` ブランチ上で開始する(ファイル編集は develop のワーキングツリーで行う)。
- **コミット・プッシュの指示があるまで、コミットもプッシュもしない。**
- コミット・プッシュの指示を受けたら、**対応内容に即した作業ブランチを `develop` から作成し、そのブランチでコミットする**。`develop` に直接コミットしない。
  - ブランチ名: `<種別>/<内容を表す英語の短い kebab-case>`
  - 種別: `feature/`(機能追加)、`fix/`(バグ修正)、`docs/`(設計資料)、`refactor/`、`chore/`(設定・依存・CI)、`test/`
  - 例: `docs/initial-design`、`feature/image-generation-queue`、`fix/windows-path-handling`
  - 例外: リポジトリ初期化直後の `develop`(コミット 0 件)には、分岐元として空の initial commit を 1 回だけ作成してよい(承認済み)。
- プッシュ・PR 作成は、指示があった場合のみ行う。PR のマージ先は `develop`。
- コミットメッセージは Conventional Commits 形式(`feat:`, `fix:`, `docs:`, `chore:` …)。件名は簡潔に、必要なら本文で理由を書く。
- 秘密情報(`.env`、トークン、APIキー等)はコミットしない。

## 5. 実装規約(実装フェーズで適用)

### Rust
- `cargo fmt` / `cargo clippy -- -D warnings` を通す。
- エラーは `thiserror` で型付けし、Tauri コマンドの戻り値はシリアライズ可能なエラー型に変換する。`unwrap()` / `expect()` は原則禁止(初期化時の不変条件を除く)。
- `rusqlite` は同期 API のため、Tauri コマンド内では `spawn_blocking` 等でブロッキングを避ける。DB スキーマ変更は必ずマイグレーションで行う。
- 外部プロセス(ComfyUI 等)の起動・終了は Rust 側で管理し、アプリ終了時に確実にクリーンアップする。

### フロントエンド
- TypeScript strict。ESLint / Prettier(または Biome)を通す。
- UI は shadcn/ui コンポーネントを基本とし、Tailwind CSS 4 のユーティリティで構成する。
- Tauri API 呼び出しは `src/lib/tauri/` の薄いラッパーに集約し、コンポーネントから直接 `invoke` を呼ばない。
- IPC の型は Rust 側の定義と一致させる(`docs/04-ipc-api.md` が正)。

### テスト
- Rust: ドメインロジック・DB 層・エンジンアダプタにユニットテスト。
- フロント: ロジック部分は Vitest、主要 UI フローはコンポーネントテスト。
- Windows / macOS 双方で CI(GitHub Actions)を回す想定。

## 6. エージェント向け注意事項

- 設計が未承認の機能について、実装コードを生成しない。
- 不明点・設計判断が必要な点は `docs/07-open-questions.md` に記録し、ユーザーに確認する。
- ユーザーの個人情報(メールアドレス等)を外部サービスへのリクエストに含めない。
- 破壊的操作(削除・強制プッシュ・履歴改変)は事前確認なしに行わない。

<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->
