# 02. アーキテクチャ

## 1. 全体構成

```
┌──────────────────────────── Tauri App ────────────────────────────┐
│  WebView (WebView2 / WKWebView)                                    │
│  Next.js 16 静的エクスポート + React 19 + Tailwind 4 + shadcn/ui   │
│        │  invoke(command) / listen(event)                          │
│  ──────┼───────────────────────────────────────────────────────── │
│  Rust (src-tauri)                                                  │
│   ├─ commands/   Tauri コマンド(薄い層。入力検証・変換のみ)       │
│   ├─ services/   アプリケーションロジック                           │
│   │    ├─ JobQueue   ジョブキュー & ワーカー                        │
│   │    ├─ ImageStore 画像/サムネイルのファイル管理                  │
│   │    └─ Settings                                                 │
│   ├─ engine/     画像生成エンジン抽象 (trait GenerationEngine)       │
│   │    └─ comfyui/  ComfyUI アダプタ (HTTP + WebSocket)             │
│   └─ db/         rusqlite + マイグレーション + リポジトリ           │
└────────────────────────────────────────────────────────────────────┘
          │ HTTP/WS (127.0.0.1:8188)                │ ファイル
          ▼                                         ▼
     ComfyUI(別プロセス・ユーザーが起動)     {app_data}/gazooo.db, images/
```

## 2. ディレクトリ構成(実装時)

```
gazooo/
├─ AGENTS.md / CLAUDE.md / docs/
├─ package.json, package-lock.json
├─ next.config.ts          # output: "export", images.unoptimized: true
├─ src/                    # Next.js (App Router)
│   ├─ app/                # ルート: /, /gallery, /queue, /presets, /settings
│   ├─ components/ui/      # shadcn/ui
│   ├─ components/         # 機能別コンポーネント
│   ├─ lib/tauri/          # invoke / listen ラッパー(型付き)
│   ├─ lib/i18n/
│   └─ stores/             # 状態管理
└─ src-tauri/
    ├─ Cargo.toml, tauri.conf.json, capabilities/
    └─ src/
        ├─ main.rs, lib.rs
        ├─ commands/
        ├─ services/
        ├─ engine/{mod.rs, comfyui/}
        └─ db/{mod.rs, migrations/, repo/}
```

パッケージマネージャは **npm**(当初は pnpm で決定したが、2026-11-18 にユーザー指示で変更。07 参照)。

## 3. 主要な設計判断

### 3.1 Next.js 静的エクスポート + Tauri
- `output: "export"` で `out/` を生成し、Tauri の `frontendDist` に指定。開発時は `devUrl: http://localhost:3000`。
- 全ルートはクライアントコンポーネント中心。動的ルート(`/gallery/[id]`)は使わず、クエリパラメータ(`/gallery?id=…`)で表現する(静的エクスポートで未知の ID を扱えないため)。
- `next/image` は使わず `<img>` を使う(`images.unoptimized: true`)。

### 3.2 ローカル画像の表示
- 画像は `{app_data}/images/` に保存し、DB にはルートからの相対パスを保持(OS 間・保存先移動に強い)。
- WebView からは Tauri の **asset protocol**(`convertFileSrc`)で読み込む。`assetProtocol.scope` は画像保存ディレクトリのみに限定。保存先を変更可能にするため、スコープは実行時に動的に許可する(`asset_protocol_scope` API)。
- 一覧ではサムネイル(長辺 384px の JPEG。`image` クレートの `png` / `jpeg` のみ有効化)を生成して使用し、原寸は詳細表示でのみ読み込む。ゴミ箱への移動は `trash` クレート(Windows / macOS 対応)。

### 3.3 エンジン抽象

```rust
#[async_trait]
pub trait GenerationEngine: Send + Sync {
    async fn health(&self) -> Result<EngineStatus, EngineError>;
    async fn list_models(&self) -> Result<EngineModels, EngineError>;   // checkpoints / unet / vae / loras
    async fn submit(&self, req: GenerationRequest) -> Result<RemoteJobId, EngineError>;
    fn subscribe(&self, id: &RemoteJobId) -> BoxStream<'static, EngineEvent>; // Progress/Preview/Done/Error
    async fn fetch_image(&self, r: &ImageRef) -> Result<Vec<u8>, EngineError>;
    async fn cancel(&self, id: &RemoteJobId) -> Result<(), EngineError>;
}
```

- `GenerationRequest` はエンジン非依存の汎用パラメータ(プロンプト、モデル、サンプラー、解像度 等)。
- ComfyUI アダプタが「モデル種別 × パラメータ → ComfyUI ワークフロー JSON」に変換する。

### 3.4 ComfyUI アダプタ

- **ワークフロー生成**: モデル種別(`sd15` / `sdxl` / `flux` / `flux_gguf`)ごとに、API 形式のワークフロー JSON を **Rust コードで組み立てる**(`engine/comfyui/workflow.rs`)。テンプレートファイル+置換ではなくコードにするのは、型安全にでき、バンドルしたリソースのパス解決(Windows/macOS 差)が不要で、リンクの整合性をユニットテストで検証できるため。ユーザー定義ワークフローのインポートは将来。
- **生成補助機能(v0.2、10 参照)**: 顔補正(Impact Pack の `FaceDetailer`)と ControlNet を、同じ「Rust コードで組み立てる」方式で追加する。`workflow.rs` は肥大化を避けて `workflow/{mod.rs, base.rs, controlnet.rs, face_detailer.rs}` に分割し、基本の生成グラフに対して「conditioning への ControlNet 適用」「VAEDecode 後の顔補正」を差し込む形にする。対象は `sd15` / `sdxl` のみ。必要なカスタムノードは `/object_info` で有無を検出し、無ければ機能を無効化して導入手順を案内する(GGUF と同じ扱い)。
- **入力画像のアップロード(ControlNet)**: 参照画像はアプリが `{images_dir}/inputs/` に保管し、生成時に `POST /upload/image`(multipart)で ComfyUI の `input` へ送って `LoadImage` で読む。これに伴い `reqwest` の `multipart` feature を有効化する(依存の変更のため、固定バージョンの更新手順に従う)。
- **完了検知と進捗**: 完了・失敗の判定は `/history/{prompt_id}` のポーリング(500ms)で行い、確実性を優先する。ステップ進捗は WebSocket の `progress` を受信できた場合に上乗せする(ベストエフォート。接続先が https の場合は WebSocket を使わず進捗は不定表示)。プレビュー画像(バイナリフレーム)は M2 では扱わない。
- **通信**:
  - `GET /system_stats` … ヘルスチェック
  - `GET /object_info` … モデル・サンプラー・スケジューラ一覧(`CheckpointLoaderSimple`, `UNETLoader`, `UnetLoaderGGUF`, `KSampler` 等の入力候補から取得)
  - `POST /prompt` … ジョブ投入(`client_id` を付与)
  - `WS /ws?clientId=…` … `progress`, `executing`, `executed`, `execution_error` と、バイナリのプレビュー画像を受信
  - `GET /history/{prompt_id}` / `GET /view` … 結果画像の取得
  - `POST /interrupt` / `POST /queue`(delete)… キャンセル
  - `POST /upload/image` … 参照画像のアップロード(v0.2、ControlNet)
- **Flux の扱い**: モデルは `unet` 系(+ `clip`/`vae` 別ファイル)ローダーを使う。GGUF 利用には ComfyUI-GGUF カスタムノードが必要。`/object_info` に該当ノードがあるかを検出し、無ければ UI で導入手順を案内する。
- ComfyUI の WebSocket クライアントは `tokio-tungstenite`、HTTP は `reqwest`(rustls)。

### 3.5 ジョブキュー

- アプリ側(Rust)で独自にキューを管理する(ComfyUI のキューには 1 ジョブずつ投入)。理由: 並べ替え・キャンセル・永続化・再起動復元をアプリ主導で行うため。
- ワーカーは 1 タスク(tokio)で、`queued` のジョブを優先度/順序で取り出し実行。
- 状態遷移: `queued → running → completed | failed | cancelled`。起動時に `running` のまま残っているものは `interrupted` 扱いで `failed`(再実行可能)に遷移。
- 1 ジョブで N 枚生成(`batch_size`)。結果画像は 1 枚ごとに DB へ記録。
- 進捗は `job://progress` イベントで UI へプッシュ(スロットリング: 最大 10 回/秒)。

### 3.6 DB アクセス

- `rusqlite`(`bundled` feature)。接続は WAL モード、`foreign_keys=ON`。
- 書き込み用の単一接続を `Mutex` で保持し、DB 呼び出しは `tokio::task::spawn_blocking` 内で実行。読み取りが重い場面は別の読み取り接続を使う。
- マイグレーションは `rusqlite_migration` を使用(`PRAGMA user_version` 管理)。

### 3.7 エラー処理

- Rust: `thiserror` による `AppError` を定義し、`serde::Serialize` で `{ code, message, details? }` に変換して IPC に返す。
- `code` は安定した文字列(`ENGINE_UNREACHABLE`, `ENGINE_MODEL_MISSING`, `DB_ERROR` 等)で、フロントは `code` から i18n メッセージを引く。

### 3.8 ログ

- `tracing` + `tracing-appender`。ログは `{app_log_dir}` にローテーション出力。プロンプト本文は INFO 以上のログに出さない(プライバシー)。**v0.24: 設定の「デバッグ」を ON にしたときだけ DEBUG の詳細ログを出し、ログは 100MB で分割・合計 1GB 超で古い順に自動削除する(プロンプト本文は DEBUG でも出さない)。詳細は [31-debug-logging.md](31-debug-logging.md)。**

### 3.9 セキュリティ

- Tauri 2 の capability でフロントに許可する権限を明示(`core:default`、`dialog:default`、`opener:default` など必要最小限)。
- CSP: `default-src 'self'; img-src 'self' asset: http://asset.localhost data:; connect-src ipc: http://ipc.localhost`。WebView から ComfyUI へは直接通信せず、すべて Rust 経由。
- 外部 URL へのリンクは opener プラグインでシステムブラウザを使用。

### 3.10 外部 API(AI サポート、v0.3)

- 唯一の外部通信は、AI サポートの `api.anthropic.com`(設計: 11)。Rust 側のみで行い、WebView の CSP は変えない。キー未登録・未同意では通信しない。キーは設定 DB に平文で保存し(11 §4)、WebView に返さず、ログに出さない。

## 4. 使用予定クレート / パッケージ

| 区分 | 候補 |
|---|---|
| Rust | tauri 2, tauri-plugin-dialog, tauri-plugin-opener, tokio, reqwest(rustls), tokio-tungstenite, rusqlite(bundled), rusqlite_migration, serde, serde_json, thiserror, tracing, uuid, image(サムネイル), async-trait, futures, command-group(子プロセスの後始末。v0.7、15) |
| Frontend | next 16, react 19, tailwindcss 4, shadcn/ui(`base-nova` スタイル = Base UI ベース), next-themes, @tauri-apps/api, zustand, @tanstack/react-virtual, lucide-react, vitest |

※ 実装時の確定事項(2026-11-18):
- `rusqlite` は `rusqlite_migration` 2.7 が要求する **0.41** に合わせる(`bundled`)。両者のバージョンは常にセットで更新する。
- フォントは `next/font/google` を使わず **システムフォントスタック**(オフラインビルド可・Windows/macOS で日本語表示が安定)。
- 依存更新方針(2026-11-18 時点の確定):
  - TypeScript は **`~6.0.3` に固定**。7.x は typescript-eslint が未対応(peer `<6.1.0`)で lint が壊れるため。typescript-eslint の対応後に再評価する。
  - ESLint は 10.x を使用。`eslint-config-next` 配下の一部プラグインが peer 警告を出すが、主要ルール(react-hooks / next / jsx-a11y / TS)の動作は確認済み。
  - `@types/node` は最低要件の Node に合わせる。(v0.18、設計: 26。レビュー待ち)最低要件を Node 26 にするため **26.x に更新する**。
  - `rusqlite` と `rusqlite_migration` は常にセットで更新する(上記)。
- HTTP は `reqwest` 0.13(`rustls-no-provider`)+ `rustls`(`ring`)。TLS の暗号プロバイダは ring に固定する(06 参照)。
- shadcn/ui の最新テンプレートは Radix ではなく Base UI(`@base-ui/react`)を使用。クラス結合は `cn` パッケージ(clsx + tailwind-merge 相当)。
