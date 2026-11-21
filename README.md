# gazooo

ローカルで動く画像生成デスクトップアプリ(Windows / macOS)。画像生成エンジンには [ComfyUI](https://github.com/comfyanonymous/ComfyUI) を使い、Stable Diffusion(SD1.5 / SDXL)と Flux.1(通常 / GGUF)で生成できます。
生成した画像・プロンプト・パラメータはローカルの SQLite で管理され、ギャラリーから検索・再現・書き出しができます。外部への送信はありません(接続先は、設定した ComfyUI だけです)。

> 開発は「設計 → 人間レビュー → 実装」のスペック駆動で進めています。方針は [AGENTS.md](AGENTS.md)、設計資料は [docs/](docs/README.md) を参照してください。

## 使うには

1. **ComfyUI を用意して起動します**(このアプリは ComfyUI に接続するだけです)。
   - 既にお持ちなら、そのまま使えます(既定の接続先は `http://127.0.0.1:8188`。設定画面で変更できます)。
   - 無ければ、このリポジトリのスクリプトで導入できます(開発者向け。[docs/08](docs/08-local-comfyui.md)):
     ```
     npm run comfyui:setup   # ComfyUI(固定バージョン)と GGUF ノードを導入
     npm run comfyui:start   # 127.0.0.1:8188 で起動
     ```
2. **モデルを配置します**(`models/checkpoints`、Flux なら `models/unet`・`clip`・`vae`)。モデルのライセンス(特に Flux.1 dev は非商用)は、各自でご確認ください。
3. gazooo を起動します。サイドバーの接続バッジが緑になれば、生成できます。

### 推奨環境

| | Windows | macOS |
|---|---|---|
| OS | Windows 10(22H2)/ 11 | macOS 13 以降(Apple Silicon) |
| GPU / メモリ | VRAM 8GB 以上の NVIDIA GPU | ユニファイドメモリ 16GB 以上(Flux は量子化モデルを推奨) |

### インストーラについて(未署名)

現時点のインストーラは **コード署名・公証をしていません**。

- Windows: SmartScreen の警告が出ます。「詳細情報」→「実行」で進めます。
- macOS: 初回は Gatekeeper の警告が出ます。macOS 15 以降は「システム設定 → プライバシーとセキュリティ → このまま開く」、それ以前は右クリック →「開く」で進めます。

## 開発

### 必要なもの

- Node.js 26 以上(npm 10 以上)
- Rust(`rust-toolchain.toml` で固定。rustup が自動で選びます)
- Windows: Visual Studio Build Tools 2022(「C++ によるデスクトップ開発」)、WebView2(Windows 11 は標準搭載)
- macOS: Xcode Command Line Tools

### コマンド

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

### 依存バージョンの固定

ビルドのたびにバージョンが変わらないよう、依存は完全一致で固定し、`package-lock.json` と `src-tauri/Cargo.lock` をコミットしています。詳細は [AGENTS.md](AGENTS.md) を参照してください。

### CI・リリース

- `develop` への PR / push で、Windows と macOS の両方で lint・型チェック・テストを実行します(`.github/workflows/ci.yml`)。`develop` への push では、インストーラも生成します。
- タグ `vX.Y.Z` を push すると、インストーラをビルドして GitHub Releases に **下書き** を作ります(`.github/workflows/release.yml`)。公開前に [リリース前チェックリスト](docs/09-release-checklist.md) を実施してください。

## サードパーティのモデルとライセンス

アプリは、利用者の操作で次のモデル・ソフトウェアをダウンロードして使います(同意画面で取得元・サイズ・ライセンスを確認してから導入します)。

| 名称 | 用途 | ライセンス |
|---|---|---|
| llama.cpp | ローカル LLM のランタイム | MIT |
| Qwen3.5-9B | AI サポート・プロンプト翻訳(ローカル LLM) | Apache-2.0 |
| **PLaMo 3 Translate 8B** | プロンプト翻訳の「翻訳専用モデル」(任意) | [PLaMo Community License](src-tauri/src/services/llm/licenses/plamo-community-license-ja.txt)([English](src-tauri/src/services/llm/licenses/plamo-community-license-en.txt)) |

**Built with PLaMo.** 翻訳専用モデルは Preferred Networks, Inc. の PLaMo 3 Translate 8B です。モデルはこのリポジトリには含まれず、利用者が設定画面から導入したときだけ Hugging Face から取得します。個人・学術利用は無料です。**商用目的で使う場合は、PLaMo Community License に従い Preferred Networks への事前登録が必要です**(売上 10 億円超は別契約)。規約の全文は、アプリの設定(プロンプト翻訳 > 翻訳専用モデル)からも読めます。

## データの場所

| | Windows | macOS |
|---|---|---|
| データベース・画像 | `%APPDATA%\dev.gazooo.desktop\` | `~/Library/Application Support/dev.gazooo.desktop/` |
| ログ | `%LOCALAPPDATA%\dev.gazooo.desktop\logs\` | `~/Library/Logs/dev.gazooo.desktop/` |

画像の保存先は、設定画面で変更できます(既存の画像を移動するかを選べます)。アンインストールしてもデータは残ります。

## 既知の制限

コード署名なし / アイコンは暫定 / 自動アップデートなし / 生成中のプレビュー、LoRA、img2img は未対応 / AMD・Intel GPU(Windows)は検証対象外。
