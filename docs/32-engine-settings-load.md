# 32. 設定 > ComfyUI の起動管理の表示高速化(v0.25、M28)

> ステータス: 🟢 承認済み・実装済み(2026-11-21。07 Q-181〜185 は推奨どおり)。15 §7・22 §2 の `engine_managed_get` の分割。差異は §6。

## 1. 現状の問題

設定 > エンジンの「ComfyUI の起動管理」は、`engine_managed_get` が返るまで **有効化スイッチが無効、状態・操作ボタンも非表示** のまま待たされる。

原因: `engine_managed_get` は、DB から読むだけで済む保存済みの設定に加え、**入力候補(`suggestions`)の探索を毎回、同期的に行ってから** 返している(22 §2 の優先順位)。

| 処理 | 遅くなる条件 | 上限 |
|---|---|---|
| ① 起動中の ComfyUI に `GET /system_stats`(`launch_args`) | ComfyUI が起動中でビジー(モデルのロード・生成中)、またはポートだけ開いて応答しない | 接続 3 秒 + 全体 10 秒 |
| ② 既知の場所の `main.py` の `is_file()` 確認(`detect_known`。Windows で 10 か所以上: Documents・Desktop・Downloads・`C:\` など) | ウイルス対策ソフトの監視、OneDrive 等のクラウド同期フォルダ、ネットワークドライブ | 1 回ずつの I/O の合計(非同期ランタイムのスレッドも占有) |
| ③ 実行ファイル周辺の探索(`detect`。祖先 8 階層 × 2 パターン) | ②で見つからなかったとき | 同上 |

候補は「未設定項目の初期値」と「候補を入力」ボタンにしか使われないのに、表示の度に毎回すべて探索している。

## 2. 方針

**設定の取得(DB のみ)と、候補の探索を分ける。** 設定・状態はすぐ表示し、候補は後から届いたら反映する。

### 2.1 IPC(04 に追記)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `engine_managed_get`(変更) | – | `{ config; state }`。**探索をしない**(DB と現在の状態のみ。即時)。`suggestions` は返さない(15 §7 で任意のため後方互換) |
| `engine_managed_suggest`(新規) | – | `Suggestions`(`mainPy?` / `python?` / `baseDirectory?` / `source?`。22 と同じ型) |
| `engine_managed_set`(変更) | `{ config }` | `{ config; state }`(`suggestions` は返さない) |

- `engine_managed_suggest` の探索順は 22 §2 のまま(起動中の ComfyUI の `argv` → 既知の場所 → 実行ファイル周辺)。
- **未設定項目の初期値入れは、Rust(旧 `view()` 内)からフロントへ移す**(Q-182)。候補が届いたとき、保存済みの設定が空で、かつユーザーが未編集の項目だけを、編集中の値(下書き)に入れる。保存はこれまでどおりユーザーの操作(確定は [保存])。

### 2.2 速度の改善(Rust)

- `engine_managed_suggest` 内のファイル確認(②③)は `spawn_blocking` で実行し、非同期ランタイムを塞がない(AGENTS.md §5)。
- ① の `/system_stats` は、この用途向けに **全体 1.5 秒のタイムアウト**(`tokio::time::timeout`)をかける(Q-183)。超えたら「起動中の候補なし」として ②へ進む。
- 候補は **キャッシュしない**(Q-184。毎回の探索はタブを開いたときの 1 回だけで、画面は待たされない)。

### 2.3 UI(05 §設定の追記は不要。挙動のみ)

- マウント時に `load()`(即時)と `loadSuggestions()`(非同期)を並行して呼ぶ。
- 候補を探索中は、「候補を入力」ボタンの位置に小さな「候補を探しています…」を表示する(Q-185)。見つからなければ何も出さない(従来と同じ)。
- 探索の失敗(エラー・タイムアウト)は黙って「候補なし」とする。他の画面を妨げない。
- 「未保存」表示(`dirty`)は従来の挙動を保つ。従来は初期値が保存済みの設定側に入っていたため、候補の自動入力だけでは「未保存」にならなかった。そこで、自動入力した値は保存済みの設定との比較の基準にも反映し、**ユーザーが編集したときだけ** 「未保存」にする(ただし [保存] を押すまで DB には書かない)。

## 3. 影響範囲

- Rust: `commands/engine_process.rs`(`view()` の分割、`engine_managed_suggest` の追加と登録 `lib.rs`)。`services/engine_process/{config,known}.rs` は変更なし。
- フロント: `lib/tauri/engine-process.ts`(ラッパー追加。`ManagedView` から `suggestions` を外す)、`stores/engine-process-store.ts`(`suggestions` を別の状態に)、`components/settings/engine-process-section.tsx`(候補の遅延反映)。
- 他の呼び出し元(`engine_managed_get` を使うバッジ・バナー・再起動ボタン等)は、`suggestions` を使っていないことを実装時に確認する。
- データ・マイグレーション: なし。

## 4. テスト

- Rust: `engine_managed_get` が応答しない ComfyUI(モックで遅延)でも即時に返ること。`engine_managed_suggest` が 1.5 秒で打ち切って既知の場所へ進むこと。
- Vitest: 候補が遅れて届いたとき、空の項目だけに入ること(入力済み・ユーザー編集済みの項目は上書きしない)。
- 手動: Windows 実機で、ComfyUI 起動中(生成中)に設定を開き、起動管理が即表示されること。

## 5. 実装順(承認後)

1. 04 の更新を確定 → Rust の分割とタイムアウト・`spawn_blocking`。
2. フロントのラッパー・ストア・セクションの更新。
3. テスト・`cargo clippy -D warnings`・ESLint・ビルド。

## 6. 実装との差異

- Rust の試験: 候補探索(`engine_managed_suggest`)は Tauri の `AppHandle` に依存するため、タイムアウトの単体試験は未作成(§4 の Rust 項目)。タイムアウトは `tokio::time::timeout` による 1.5 秒の打ち切りのみ。
- フロントは、ユーザーが触れた項目(`touched`)を覚え、候補が遅れて届いても触れていない空欄だけに入れる(Q-182 の実装方法)。
