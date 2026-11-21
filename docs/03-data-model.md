# 03. データモデル(SQLite)

- 配置: `{app_data_dir}/gazooo.db`(Windows: `%APPDATA%\<identifier>\`、macOS: `~/Library/Application Support/<identifier>/`)
- PRAGMA: `journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL`
- ID は UUID v7 文字列(TEXT)。時刻は UTC の Unix ミリ秒(INTEGER)。
- スキーマ変更は `src-tauri/src/db/migrations/` の連番マイグレーションのみで行う。

## ER 図

```
presets            jobs 1 ──< images >── 0..1 (favorite 等はカラム)
settings(kv)       jobs 1 ──< job_events (任意: v0.2)
```

## テーブル定義(v1)

```sql
-- 生成ジョブ(1 リクエスト = 1 ジョブ、N 枚の画像を持つ)
CREATE TABLE jobs (
  id            TEXT PRIMARY KEY,
  status        TEXT NOT NULL CHECK (status IN
                  ('queued','running','completed','failed','cancelled')),
  position      INTEGER NOT NULL,              -- キュー内の並び順(小さいほど先)
  engine        TEXT NOT NULL,                 -- 'comfyui'
  params_json   TEXT NOT NULL,                 -- GenerationRequest の JSON(下記)
  prompt_text   TEXT NOT NULL,                 -- 検索用に非正規化
  model_name    TEXT NOT NULL,
  error_code    TEXT,
  error_message TEXT,
  remote_id     TEXT,                          -- ComfyUI の prompt_id
  created_at    INTEGER NOT NULL,
  started_at    INTEGER,
  finished_at   INTEGER
);
CREATE INDEX idx_jobs_status_position ON jobs(status, position);
CREATE INDEX idx_jobs_created_at ON jobs(created_at DESC);

-- 生成画像
CREATE TABLE images (
  id            TEXT PRIMARY KEY,
  job_id        TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  file_path     TEXT NOT NULL,                 -- 画像ルートからの相対パス('/' 区切りで保存)
  thumb_path    TEXT NOT NULL,
  width         INTEGER NOT NULL,
  height        INTEGER NOT NULL,
  seed          INTEGER NOT NULL,
  file_size     INTEGER NOT NULL,
  is_favorite   INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);
CREATE INDEX idx_images_created_at ON images(created_at DESC);
CREATE INDEX idx_images_favorite ON images(is_favorite, created_at DESC);
CREATE INDEX idx_images_job ON images(job_id);

-- プロンプト全文検索(MVP は LIKE 検索で開始し、必要なら FTS5 に移行)
-- CREATE VIRTUAL TABLE jobs_fts USING fts5(prompt_text, content='jobs', content_rowid='rowid');

-- プリセット
CREATE TABLE presets (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  params_json TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- 設定(キー・バリュー)
CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
```

## `params_json`(GenerationRequest)

```json
{
  "schemaVersion": 1,
  "model": { "kind": "flux_gguf", "name": "flux1-dev-Q4_K_S.gguf",
             "clip": ["t5xxl_fp8.safetensors", "clip_l.safetensors"], "vae": "ae.safetensors" },
  "prompt": "…",
  "negativePrompt": "",
  "width": 1024, "height": 1024,
  "steps": 20, "cfg": 1.0, "guidance": 3.5,
  "sampler": "euler", "scheduler": "simple",
  "seed": 123456789, "seedRandomize": false,
  "batchSize": 1
}
```
(キーは IPC(04)と同じ camelCase。`seed` は 0〜2^53-1(JS で安全に扱える範囲)。)

### 生成補助機能のフィールド(v0.2、設計: 10)

任意のオブジェクトで、**無ければ無効**。追加は前方互換(古いデータはそのまま読める)のため `schemaVersion` は 1 のまま、DB マイグレーションも不要(`params_json` は JSON のため)。

```json
{
  "faceDetailer": { "enabled": true, "detector": "bbox/face_yolov8m.pt",
                    "denoise": 0.4, "prompt": "", "confidence": 0.5 },
  "controlnet": { "enabled": true, "units": [
    { "image": "inputs/0190….png", "model": "control_v11p_sd15_canny.pth",
      "preprocessor": "canny", "strength": 1.0, "startPercent": 0.0, "endPercent": 1.0 }
  ] }
}
```

- `faceDetailer.enabled` / `controlnet.enabled` が false(またはフィールドなし)なら無効。無効のときも設定(参照画像など)は残せる(フォームの切り替えで消えないようにするため)。
- `preprocessor`: `canny | depth | openpose | none`。`units` は 0〜1 件(v0.2)。
- `controlnet.units[].image` は **画像保存先からの相対パス**(`/` 区切り)。
- 両フィールドとも `sd15` / `sdxl` のときのみ指定できる(検証は 10 §7)。
- v0.5: `inpaint: { enabled, image, mask, denoise, growMask, feather, compositeBack }`(13 §3.2)。`image` / `mask` は `inputs/<uuid>.png`。`sd15` / `sdxl` のみ、`faceDetailer` とは同時に有効にできない。`params_json` の拡張のみでマイグレーションは不要。

- `model.kind`: `sd15 | sdxl | flux | flux_gguf | anima | qwen_image | krea2`。種別ごとに使われるフィールドが異なる(Flux は `cfg=1.0` 固定で `guidance` を使用、ネガティブ不使用)。v0.9(17): `anima` / `qwen_image` / `krea2` は `model.name`=拡散モデル、`model.clip`=テキストエンコーダ 1 つ、`model.vae`=VAE(必須)。`cfg` を使い(`guidance` は不使用)、ネガティブは Anima・Qwen-Image 2.1 で使い Krea 2 では使わない。顔補正・ControlNet・Inpaint は SD 1.5 / SDXL のみ。
- `schemaVersion` により後方互換を管理。未知フィールドは保持する。

## 設定キー(`settings`)

| key | 型 | 既定値 |
|---|---|---|
| `engine.comfyui.url` | string | `http://127.0.0.1:8188` |
| `storage.images_dir` | string \| null | null(= `{app_data}/images`) |
| `ui.theme` | `"light"\|"dark"\|"system"` | `dark`(v0.21 で `system` から変更。保存済みの値は変わらない) |
| `ui.state` | object | なし(v0.21。パネルの開閉・幅・表示の密度。29 §6) |
| `ui.language` | `"ja"\|"en"` | OS ロケール |
| `generation.default_params` | GenerationRequest | 組み込み既定 |

### AI サポートの設定とデータ(v0.3、設計: 11 §4・§9)

| key | 型 | 既定値 |
|---|---|---|
| `assistant.api_key` | string | 未設定(**平文**。`settings_get` では返さない) |
| `assistant.key_verified` | boolean | false(キーの検証結果。キー削除時に一緒に消える) |
| `assistant.model` | string(`claude-haiku-5-5` / `claude-sonnet-5-5` / `claude-opus-5-5`) | `claude-sonnet-5-5` |
| `assistant.consent` | boolean | false |
| `assistant.auto_delete` | boolean | true |
| `assistant.provider` | `"anthropic" \| "local" \| "local_external"` | `anthropic`(v0.23。11 §13・30 §6) |
| `assistant.local_base_url` | string | `http://127.0.0.1:11434/v1`(`local_external` の接続先。ループバックのみ) |
| `assistant.local_model` | string | `qwen3.5:9b`(`local_external` のモデル名) |
| `llm.idle_minutes` | number | 5(0 / 1 / 5 / 15 / 30。0 = 自動停止なし。30 §5) |
| `llm.release_before_generation` | boolean | true(生成開始前に LLM を停止する。30 §5) |

### モデル取得の設定(v0.4、設計: 12 §9。レビュー待ち)

| key | 型 | 既定値 |
|---|---|---|
| `engine.comfyui.models_dir` | string \| null | null(= ComfyUI の `/system_stats` から自動検出) |
| `models.consents` | string[](同意済みの `familyId/variantId`) | `[]` |

`settings` のキー追加のみで、マイグレーションは不要(ダウンロードの状態はメモリ上のみ)。

### マイグレーション 0003(AI サポートの会話履歴)

```sql
CREATE TABLE assistant_conversations (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_assistant_conv_updated ON assistant_conversations(updated_at DESC);

CREATE TABLE assistant_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
  role            TEXT NOT NULL CHECK (role IN ('user','assistant')),
  text            TEXT NOT NULL,
  params_json     TEXT,              -- user: 送信時点のフォーム内容
  suggestions_json TEXT,             -- assistant: 検証済みの提案と適用済みフラグ
  created_at      INTEGER NOT NULL
);
CREATE INDEX idx_assistant_msg_conv ON assistant_messages(conversation_id, created_at);
```

- 自動削除(`assistant.auto_delete = true`)のとき、会話は **最大 30 件**。保存のたびに、`updated_at` が古い会話から超過分を削除する(メッセージは `ON DELETE CASCADE`)。`false` のときは上限なし。

## マイグレーション 0002(キュー表示の非表示フラグ)

`images.job_id` は `ON DELETE CASCADE` のため、ジョブを消すと画像の記録(と再現に必要なパラメータ)も失われる。
「履歴クリア(画像は残す)」を実現するため、`jobs` に `queue_hidden INTEGER NOT NULL DEFAULT 0` を追加する。
- `job_clear_finished`: 終了済み(completed / failed / cancelled)のジョブのうち、**画像を持たないものは削除**、**画像を持つものは `queue_hidden = 1` にして**キュー画面から隠す(ギャラリー・再現には影響しない)。
- `job_list` は `queue_hidden = 1` を返さない。

## ファイル配置

> サムネイルのフォルダを `.thumbs` ではなく `thumbs` にしているのは、Tauri の asset プロトコルのスコープが、Unix 系ではドット始まりのパスをワイルドカードに一致させない既定(`require_literal_leading_dot`)のため。隠しフォルダにすると WebView からサムネイルを読めなくなるおそれがある。

```
{images_dir}/
  2026/11/<image_id>.png          # 原寸(ComfyUI から取得した PNG、メタデータ埋め込み)
  thumbs/<image_id>.jpg           # サムネイル(長辺 384px の JPEG、品質 85)
  inputs/<uuid>.png               # ControlNet の参照画像(v0.2。ギャラリー(images テーブル)には載せない)
```

- `inputs/` の画像は取り込み時に PNG へ再エンコードし、メタデータを除去する(10 §6)。保存先の変更では原寸・サムネイルと同様に移動の対象。

- 原寸 PNG には生成パラメータをテキストチャンクとして保持し、DB を失っても再現可能にする(ComfyUI が `SaveImage` で埋め込むメタデータをそのまま保存する)。
- サムネイルは画像の保存時に生成する。M2 で保存済みの画像(`thumb_path` が原寸と同じもの)は、起動時にバックグラウンドで生成してバックフィルする。サムネイルを作れない形式の場合は原寸を指したままにする(表示は壊れない)。
- 画像の並び(ギャラリー)は `created_at` 降順、同値は `id` 降順。ページングは `(created_at, id)` のキーセット方式(新しい画像が追加されても重複・欠落しない)。
- 検索は `jobs.prompt_text` の部分一致(LIKE。`%` `_` `\` はエスケープ)。
- 検証範囲: 幅・高さは 64 の倍数かつ 256〜2048、steps 1〜150、cfg 0〜30、枚数 1〜8、プロンプト必須。Flux 系は clip 2 本と vae が必須。
- 保存先の変更は専用コマンドで行い、既存画像を **移動する**(コピー → 設定切替 → 旧ファイルをゴミ箱へ)か **移動しない**(設定のみ切替。DB は相対パスのため旧画像は保存先を戻すまで表示できない)かを選べる(04「保存先の変更」)。

## マイグレーション方針

- `0001_init.sql` で上記を作成。以降は前方互換のみ(ダウングレードは非対応)。
- マイグレーション前に DB ファイルのバックアップ(`gazooo.db.bak-<version>`)を作成。

### デバッグログの設定(v0.24、設計: 31 §2。承認済み・実装済み)

`settings` に `debug.enabled`(boolean、既定 false)を追加する。スキーマ変更・マイグレーションはなし。`settings_update` では変更できず、`debug_set_enabled` で行う。

## 追記(v0.31・承認済み・実装済み): ユーザー機能(設計: 36)

- **DB の分割**: `{app_data_dir}/app.db`(端末共通: `users`・`device_settings`)と、ユーザーごとの `{app_data_dir}/users/<user_id>/gazooo.db`(本資料のテーブル一式。スキーマ・マイグレーションは従来どおり)。画像の既定の保存先は `users/<user_id>/images`。
- **設定キーの分類**(`settings_get` / `settings_update` の API は変えず、Rust 側のキー分類表で振り分ける): 端末共通 = `engine.comfyui.url`・`engine.comfyui.managed`・`engine.comfyui.models_dir`・`debug.enabled`・`llm.*`(`app.db` の `device_settings`)。それ以外はすべてユーザーごと(`settings`)。分類にないキーはユーザーごと。
- 従来の `gazooo.db` は初回起動時に最初のユーザー「ユーザー 1」へ引き継ぐ(`gazooo.db.pre-users` として残す。画像は移動せず `storage.images_dir` を旧位置に固定)。詳細は [36-users.md](36-users.md) §3・§4・§8。
