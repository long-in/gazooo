# 04. IPC API(Tauri コマンド / イベント)

フロントエンドと Rust 間のインターフェース定義。**この資料が型の正**であり、フロントの `src/lib/tauri/` は本定義に従う。
命名: コマンドは `snake_case`、イベントは `domain://name`。エラーはすべて `AppError { code, message, details? }`。

## 共通型

```ts
type JobStatus = "queued" | "running" | "completed" | "failed" | "cancelled";
type ModelKind = "sd15" | "sdxl" | "flux" | "flux_gguf" | "anima" | "qwen_image" | "krea2"; // v0.9(17)

interface GenerationParams { /* 03-data-model.md の params_json と同一 */ }

interface Job {
  id: string; status: JobStatus; position: number;
  params: GenerationParams; error?: { code: string; message: string };
  imageIds: string[]; createdAt: number; startedAt?: number; finishedAt?: number;
}

interface ImageItem {
  id: string; jobId: string; width: number; height: number; seed: number;
  isFavorite: boolean; createdAt: number;
  filePath: string;   // 絶対パス。フロントが convertFileSrc で URL に変換する
  thumbPath: string;  // 同上(サムネイル。生成できなかった場合は filePath と同一)
}

interface Page<T> { items: T[]; nextCursor?: string }
```

## コマンド

### エンジン

| コマンド | 引数 | 戻り値 | 説明 |
|---|---|---|---|
| `engine_status` | – | `EngineStatus`(下記) | 現在の接続状態(即時にヘルスチェックを実行して返す) |
| `engine_list_models` | – | `EngineModels`(下記) | `/object_info/<Node>` から取得。未接続時は `ENGINE_UNREACHABLE` |
| `engine_reconnect` | – | `EngineStatus` | 即時に再接続を試み、結果を `engine://status` でも通知 |

```ts
interface EngineStatus {
  connected: boolean;
  url: string;
  version?: string;        // ComfyUI のバージョン
  device?: string;         // 先頭デバイス名(例: "mps", "NVIDIA GeForce RTX ...")
  vramFree?: number;       // bytes
  features: {
    ggufNode: boolean;               // ComfyUI-GGUF(UnetLoaderGGUF)の有無
    faceDetailer: boolean;           // Impact Pack(FaceDetailer)と Subpack(UltralyticsDetectorProvider)の有無(v0.2)
    controlnetPreprocessors: ("canny" | "depth" | "openpose")[];  // 利用できる前処理。canny はコアの Canny ノード(v0.2)
    inpaint: boolean;                // Inpaint に必要なコアノード(13 §3.3)がすべてある(v0.5)
    imageTag: boolean;               // WD14 Tagger(WD14Tagger|pysssss)の有無(v0.34。39)
  };
  error?: "unreachable" | "bad_response";  // connected=false のときの理由
}

interface EngineModels {
  checkpoints: string[];   // CheckpointLoaderSimple.ckpt_name
  unets: string[];         // UNETLoader.unet_name
  ggufUnets: string[];     // UnetLoaderGGUF.unet_name(ノード未導入なら空)
  clips: string[];         // CLIPLoader.clip_name
  vaes: string[];          // VAELoader.vae_name
  loras: string[];         // LoraLoader.lora_name
  controlnets: string[];   // ControlNetLoader.control_net_name(v0.2)
  faceDetectors: string[]; // UltralyticsDetectorProvider.model_name のうち "bbox/" で始まるもの(v0.2。ノード未導入なら空)
  samplers: string[];      // KSampler.sampler_name
  schedulers: string[];    // KSampler.scheduler
}
```

接続状態の監視: Rust 側が 5 秒間隔でヘルスチェックを行い、状態が変化したときだけ `engine://status` を発火する。ComfyUI の URL が設定で変更されたら即座に再接続する。
`/object_info` は全ノード分だと巨大なため、必要なノードのみ `/object_info/<NodeClass>` で個別取得する。
ComfyUI のバージョンにより選択肢の形式が異なる(`[[...], {}]` と `["COMBO", {"options": [...]}]`)ため、両方を受け付ける。

### ジョブ

| コマンド | 引数 | 戻り値 | 説明 |
|---|---|---|---|
| `job_enqueue` | `{ params: GenerationParams; count?: number }` | `Job[]` | キューへ追加。`count`(既定 1、最大 20)回分を個別ジョブとして追加する。`seedRandomize` が true なら各ジョブごとにランダムなシードを確定して保存、false ならシードを `seed, seed+1, …` とする |
| `job_list` | `{ statuses?: JobStatus[]; cursor?: string; limit?: number }` | `Page<Job>` | 一覧 |
| `job_cancel` | `{ id }` | `void` | queued は削除、running は ComfyUI へ interrupt |
| `job_reorder` | `{ id; newPosition }` | `void` | queued のみ。`newPosition` は待機中ジョブ内の 0 始まりの位置 |
| `job_retry` | `{ id }` | `Job` | failed / cancelled を同じパラメータで複製して再投入 |
| `job_clear_finished` | – | `number` | 完了・失敗・キャンセル済みジョブの履歴クリア。画像を持たないジョブは削除、画像を持つジョブはキュー画面から隠す(画像・パラメータは残る。03 参照)。戻り値は対象件数 |

### ギャラリー

| コマンド | 引数 | 戻り値 | 説明 |
|---|---|---|---|
| `image_list` | `{ query?: string; favoritesOnly?: boolean; cursor?: string; limit?: number }` | `Page<ImageItem>` | 新しい順。`query` はプロンプトの部分一致。`limit` 既定 60・最大 200。`nextCursor` は不透明な文字列(キーセット方式。03 参照) |
| `image_get` | `{ id }` | `{ image: ImageItem; job: Job }` | 詳細(パラメータ含む) |
| `image_set_favorite` | `{ id; value: boolean }` | `void` | |
| `image_delete` | `{ ids: string[]; deleteFiles: boolean }` | `void` | DB の記録を削除する。`deleteFiles` が true なら原寸とサムネイルを **OS のゴミ箱へ移動**(Q-10)。ファイルが既に無くてもエラーにしない。完了後に `image://deleted` を発火 |
| `image_export` | `{ id; destPath }` | `void` | 原寸をコピーする。`destPath` は保存ダイアログで選んだ絶対パス |
| `image_reveal` | `{ id }` | `void` | エクスプローラー / Finder で表示 |
| `image_export_many` | `{ ids: string[] }` | `string \| null` | フォルダ選択ダイアログ → 選択画像の原寸を保存開始(`jobId`。キャンセルは `null`。最大 1000 枚。docs/44)。イベント `image-export://progress` `{ jobId, done, total }` / `image-export://done` `{ jobId, saved, failed, cancelled, error }` |
| `image_export_cancel` | `{ jobId }` | `void` | 一括保存の中止 |
| `image_count_non_favorite` | `{ scope? }` | `{ total: number; favorites: number }` | (v0.49・実装済み。docs/49)スコープ内のお気に入り以外・お気に入りの件数。確認ダイアログ用 |
| `image_delete_non_favorite` | `{ scope?; expectedCount: number }` | `{ deleted: number; failed: number }` | (v0.49・実装済み。docs/49)スコープ内のお気に入り以外を削除(ファイルはゴミ箱へ)。実数が `expectedCount` を超えていたら何も消さず `STALE_COUNT` |
| `image_export_open_dir` | – | `void` | 前回の一括保存の保存先を開く |

### 入力画像(v0.2、ControlNet の参照画像。設計: 10 §6)

| コマンド | 引数 | 戻り値 | 説明 |
|---|---|---|---|
| `input_image_pick` | – | `InputImage \| null` | **Rust 側でファイル選択ダイアログを開き**、選ばれた PNG / JPEG を取り込む。キャンセル時は `null` |
| `input_image_pick_many` | – | `InputImage[]` | **Rust 側でファイル選択ダイアログ(複数選択)を開き**、選ばれた PNG / JPEG をまとめて取り込む(顔の入れ替えの対象画像。docs/42)。1 回に最大 50 枚(超えた分は無視)。キャンセル時は空 |
| `input_image_from_gallery` | `{ imageId: string }` | `InputImage` | ギャラリーの画像を参照画像として取り込む(`IMAGE_NOT_FOUND` あり) |
| `input_image_get` | `{ path: string }` | `InputImage` | 既存の参照画像の情報を取得する(プリセット・再現の復元時に存在確認も兼ねる。無ければ `IMAGE_NOT_FOUND`) |
| `mask_save` | リクエスト本文に PNG のバイト列(raw body) | `InputImage` | Inpaint のマスク(マスクエディタが描いた PNG)を検証し、グレースケール PNG として `inputs/` に保存する(v0.5。設計: 13 §3.4) |

```ts
interface InputImage {
  path: string;      // 画像保存先からの相対パス("inputs/<uuid>.png")。GenerationParams.controlnet.units[].image に入れる
  filePath: string;  // 絶対パス。フロントが convertFileSrc で表示する
  width: number; height: number;
}
```

- 取り込みは PNG へ再エンコードして `inputs/` に保存する。形式・サイズが不正なら `VALIDATION`(20MB、長辺 8192px まで)。
- ファイルのパスをフロントから受け取らない設計(任意ファイル読み取りの防止)。
- `job_enqueue` は、有効な Inpaint の画像・マスクが保存先に無ければ `IMAGE_NOT_FOUND`、縦横の不一致・全黒のマスクは `VALIDATION`(`inpaint.mask`)で断る。`params.inpaint` の形は 13 §3.2。
- `assistant_ask` は、今の ComfyUI で使える機能(直近の接続確認の結果。通信しない)を質問に添えて送る(13 §4.2)。引数は変わらない。
- `job_enqueue` は、有効な ControlNet の参照画像が保存先に無ければ `IMAGE_NOT_FOUND` で断る。待機中に消えた場合は、ジョブが `failed`(`IMAGE_NOT_FOUND`)になる。

### プリセット

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `preset_list` | – | `Preset[]`(名前の昇順) |
| `preset_save` | `{ id?; name; params }` | `Preset` |
| `preset_delete` | `{ id }` | `void` |

```ts
interface Preset { id: string; name: string; params: GenerationParams; createdAt: number; updatedAt: number }
```

- `name` は前後の空白を除いて 1〜60 文字。**同じ名前が既にあり、かつ `id` が別(または未指定)の場合は `VALIDATION`(`details.field = "name"`)**。上書きしたいときは、フロントが既存プリセットの `id` を指定して保存する(確認ダイアログはフロントの責務)。
- `params` は `GenerationParams` として解釈できればよい(プロンプトが空でも保存できる。生成時の検証は `job_enqueue` で行う)。
- 存在しない `id` の更新・削除は `PRESET_NOT_FOUND`。

### 設定

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `settings_get` | – | `Settings` |
| `settings_update` | `{ patch: SettingsPatch }` | `Settings` |

```ts
interface Settings {
  comfyuiUrl: string;            // 既定 http://127.0.0.1:8188(http/https のみ)
  imagesDir: string | null;      // null = {app_data}/images(絶対パスのみ)
  theme: "light" | "dark" | "system";
  language: "ja" | "en" | null;  // null = OS 追従
  generationDefaults: GenerationParams | null;
  uiState: Record<string, unknown> | null;  // v0.21: パネルの開閉・幅・密度(29 §6)。null = 既定
}
// 未指定キーは変更しない。imagesDir / language に null を渡すと既定に戻す。
// 検証エラー時は一切書き込まれず VALIDATION(details.field)を返す。
type SettingsPatch = Partial<Settings>;
```

`uiState`(画面の状態。03 の `ui.state`)はオブジェクトの JSON で、Rust 側はそのまま保存する(オブジェクト以外は `VALIDATION`、`null` で既定に戻る)。形式の検証はフロントエンド(`lib/ui-state.ts`)が行う。

`generationDefaults`(生成フォームの下書き。03 の `generation.default_params`)は `GenerationParams | null` で、フォームの入力内容を自動保存・復元するために使う(null = 組み込みの既定値)。Rust 側は JSON としてそのまま保存し、`job_enqueue` の時点で検証する。
**`settings_update` は `imagesDir` を受け付けない**(渡すと `VALIDATION`)。保存先の変更は、既存画像の扱いを伴うため専用コマンドで行う(下記)。

### AI サポート(v0.3、設計: 11 §8)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `assistant_status` | – | `{ hasKey; verified; model; consent }` |
| `assistant_set_key` | `{ key }` | `{ verified }`(401 は保存せず `ASSISTANT_AUTH`) |
| `assistant_clear_key` | – | – |
| `assistant_update` | `{ model?; consent?; autoDelete? }` | `status` |
| `assistant_prompt_get` / `_set` / `_reset`(v0.22) | – / `{ text }` / – | `{ text; defaultText; customized; modelGuide }` / `{ customized }` / `{ text }`(11 §12.5) |
| `assistant_ask` | `{ conversationId: string \| null; text; params: GenerationParams }` | `{ requestId; conversationId; assistantMessageId }` |
| `translate_status` | なし | `{ provider; model; ready; reason }`(34 §5)。**v0.30 追補(34 §9.5。実装済み)**: `{ provider; model; mtModel; plain: Readiness; polish: Readiness }` |
| `translate_update` | `{ patch: { provider?; model?; mtModel? } }`(`mtModel` は v0.30 追補) | `translate_status` と同じ(更新後) |
| `translate_run` | `{ requestId; text; target: { kind; familyId; reference } }` | `{ text: string | null }`(中止は `null`。34)。**v0.30 追補(34 §9.5。承認済み・未実装)**: 引数に `mode: "plain" \| "polish"`(必須。`target` は `polish` のみ必須)、戻り値に `notice?: "polish_failed"` |
| `translate_cancel` | `{ requestId }` | `void` |
| `translate_mt_status` | なし | `LlmStatus`(翻訳専用モデルの導入・起動状態。34 §9.14) |
| `translate_mt_install` / `translate_mt_install_cancel` / `translate_mt_remove` | なし / `{ installId }` / なし | `{ installId }` / `void` / `LlmStatus`(導入・中止・削除。イベント `translate-mt://state`・`translate-mt://install-progress`) |
| `translate_mt_license` | `{ lang: "ja" \| "en" }` | `{ lang; text }`(PLaMo Community License の全文。アプリに同梱) |
| `assistant_conversation_list` / `_get` / `_delete` | – / `{ id }` / `{ ids \| null }`(null = すべて) | 会話の一覧 / 取得 / 削除(最大 30 会話、11 §4.1) |
| `assistant_suggestion_applied` | `{ messageId; index }` | 提案を適用済みとして記録 |
| `assistant_cancel` | `{ requestId }` | – |

- キーは戻り値・イベントに含めない。同意なし `ASSISTANT_NO_CONSENT`、キーなし `ASSISTANT_NO_KEY`、実行中の再送 `ASSISTANT_BUSY`。
- 応答はイベント `assistant://delta` / `assistant://suggestion` / `assistant://done` / `assistant://error`(下記)。

### モデル取得(v0.4、設計: 12 §8。レビュー待ち)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `models_catalog` | – | `{ catalog; installed; modelsDir; modelsDirSource; freeBytes }`(外部へは通信しない。自動検出のため ComfyUI へは問い合わせる) |
| `models_download_start` | `{ familyId; variantId; consent?: boolean }` | `{ downloadId }` |
| `models_download_cancel` | `{ downloadId; discard?: boolean }` | – |
| `models_download_list` | – | `DownloadState[]` |
| `models_verify` | `{ familyId; variantId }` | `{ ok: boolean }`(導入済みファイルのハッシュ照合) |
| `models_set_dir` | `{ path: string \| null }` | `{ modelsDir; source }`(null で自動検出) |
| `models_engine_dir_status` | – | `EngineDirStatus` = `{ engineModelsDir: string \| null; applied; mismatch; hasCustomConfig; canApply; reason?: "remote" \| "engine_unknown" \| "no_setting" \| "foreign" }`(v0.6。ComfyUI の参照先と反映状況。08 §6.2) |
| `models_apply_dir_to_engine` | `{ clear?: boolean }` | `EngineDirStatus`(v0.6。保存先を ComfyUI の `extra_model_paths.yaml` に反映 / `clear` で解除。再起動は別途必要。エラー `MODELS_EXTRA_PATHS_FOREIGN` / `MODELS_DIR_UNKNOWN`) |

- 型・検証・取得の仕様は 12 が正。`settings_update` は `engine.comfyui.models_dir` を受け付けない(`models_set_dir` で検証つきで変更)。
- イベント `models://progress` / `models://done`(下記)。エラー: `MODELS_DIR_UNKNOWN` / `MODELS_NO_CONSENT` / `MODELS_HOST_NOT_ALLOWED` / `MODELS_DISK_FULL` / `MODELS_DOWNLOAD_FAILED` / `MODELS_HASH_MISMATCH` / `MODELS_ALREADY_INSTALLED` / `MODELS_NOT_FOUND`。

### 保存先の変更とデータの移行(Q-11、v0.52 で改訂。設計: 51)

| コマンド | 引数 | 戻り値 | 説明 |
|---|---|---|---|
| `storage_change_images_dir` | `{ scope: "global" \| "user"; newDir: string \| null }` | – | 画像の保存先を切り替える(**ファイルは移さない**)。`newDir = null` は、個別なら全体設定、全体なら標準値に戻す |
| `storage_migration_plan` | `{ target: "imagesGlobal" \| "imagesUser" \| "models"; newDir: string \| null }` | `MigrationPlan` | 移行の内容を確認する(何も変更しない)。`newDir = null` は、全体なら標準値、個別なら全体設定に戻す(モデルでは不可) |
| `storage_migration_start` | 同上 | `MigrationReport` | 移行を実行する(完了まで返らない)。成功すると移行先が保存先になる。事前確認はやり直す |
| `storage_migration_cancel` | – | – | コピー中なら中止する |
| `storage_migration_quit` | – | – | 中止して(切り替え以降なら完了を待って)アプリを終了する |

- `MigrationPlan` / `MigrationReport` の形は [51 §7](51-storage-migration.md)、型の定義は `src/lib/tauri/storage.ts`。
- イベント: `storage://migration-progress`(`{ phase: "copying" | "switching" | "cleaning"; doneBytes; totalBytes; doneFiles; totalFiles; currentFile? }`)、`storage://close-requested`(移行中にアプリを閉じようとした)。
- エラー: `STORAGE_BUSY` / `STORAGE_CANCELLED` / `STORAGE_NO_SPACE` / `STORAGE_CONFLICT` / `VALIDATION`(`details.field = "newDir"`)。
- 移行中は、画像・保存先・ComfyUI・ユーザーを変えるコマンドを `STORAGE_BUSY` で断る(対象の一覧は `commands/storage.rs`)。
- `storage_change_images_dir` は、相対パス・書き込めない場所(`VALIDATION`)と、生成ジョブの実行中(`JOB_INVALID_STATE`)は実行しない。
- 画像の保存先が変わったら、新しい保存先を asset プロトコルのスコープに追加する。

### ComfyUI の起動管理(v0.7、設計: 15 §7。承認済み)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `engine_managed_get` | – | `{ config; state; suggestions? }`(設定・状態・パスの候補) |
| `engine_managed_set` | `{ config }` | `{ config }`(絶対パス・存在・ポート範囲を検証。有効化で接続先 URL を同期) |
| `engine_process_start` / `engine_process_stop` | – | `EngineProcState` |
| `engine_process_restart` | `{ force?: boolean }` | `EngineProcState`(実行中のジョブがあり `force` でなければ `ENGINE_BUSY`) |
| `engine_process_logs` | `{ lines?: number }` | `{ lines: string[] }` |

- 型は 15 が正。イベント `engine://proc-state` / `engine://log` / `engine://recovery`(自動復旧の通知 `{ phase: "restarting" | "recovered" | "gave_up"; attempt; requeuedJobs? }`。47 §4.4)。`ManagedEngineConfig.autoRecover` を追加(既定 false)。エラー: `ENGINE_PORT_IN_USE` / `ENGINE_LAUNCH_FAILED` / `ENGINE_START_TIMEOUT` / `ENGINE_BUSY` / `ENGINE_NOT_MANAGED`。
- (v0.14・設計: 22)`engine_managed_get` の `suggestions` に、任意の `source`(`running` / `app` / `manual` / `portable` / `desktop` / `dev`。候補の出どころ)が加わる。起動中の ComfyUI がないときは、アプリ内蔵の既知の場所の一覧(アプリが導入したものが最優先)から候補を求める。
- (v0.25・設計: 32。レビュー待ち)表示の高速化のため、`engine_managed_get` / `engine_managed_set` は `{ config; state }` を即時に返し(**`suggestions` を返さない**。探索をしない)、候補は新設の `engine_managed_suggest`(引数なし・戻り値 `Suggestions`)で別に取得する。未設定項目への初期値入れは、フロントが候補の到着後に行う。起動中の ComfyUI への問い合わせは全体 1.5 秒で打ち切る。

### ComfyUI のインストール(v0.15、設計: 23。承認済み)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `engine_install_status` | – | `InstallView`(状態・導入の記録・更新の有無・導入先・基本/コンポーネントの一覧(サイズ・ライセンス)・接続する取得元ホスト) |
| `engine_install_preflight` | `{ components: string[] }` | `Preflight`(対応 OS・空き容量と必要量・GPU の有無・導入先)。**何もダウンロードしない** |
| `engine_install_start` | `{ request: { components: string[]; accepted: boolean } }` | `InstallState`(バックグラウンドで進む。`accepted` が `false` なら `VALIDATION`。実行中のジョブ・起動中の ComfyUI があると `ENGINE_BUSY`、容量不足は `ENGINE_INSTALL_DISK_FULL`) |
| `engine_install_cancel` | – | – |
| `engine_install_remove` | `{ keepData: boolean }` | `InstallView`(`keepData` なら `data/` を残す。管理設定が削除した導入を指していれば無効に戻す) |
| `engine_install_logs` | `{ lines?: number }` | `{ lines: string[] }`(最新 500 行まで) |

- 型は 23 が正。イベント `engine://install-state`(`InstallState`)/ `engine://install-log`(`{ line }`)。
- エラー: `ENGINE_INSTALL_CANCELLED` / `ENGINE_INSTALL_DISK_FULL` / `ENGINE_INSTALL_HASH_MISMATCH` / `ENGINE_INSTALL_HOST_NOT_ALLOWED` / `ENGINE_INSTALL_DOWNLOAD_FAILED` / `ENGINE_INSTALL_EXTRACT_FAILED` / `ENGINE_INSTALL_ARCHIVE_UNSAFE` / `ENGINE_INSTALL_COMMAND_FAILED` / `ENGINE_INSTALL_VERIFY_FAILED` / `ENGINE_INSTALL_UNSUPPORTED` / `ENGINE_INSTALL_REMOVE_FAILED`、共通で `ENGINE_BUSY`。


## イベント(Rust → フロント)

| イベント | ペイロード | 発火タイミング |
|---|---|---|
| `engine://status` | `engine_status` の戻り値 | 接続状態の変化 |
| `job://updated` | `Job` | ステータス変更・追加・並び替え |
| `job://progress` | `{ jobId; step; totalSteps; node?: string; phase?: "generate" \| "face_detail" }` | 生成中(最大 10 回/秒)。`phase` はどの段階の進捗かを示す(v0.2。進捗を出したノードが顔補正なら `face_detail`、それ以外は `generate`。UI は顔補正のときだけ「顔補正中」と表示) |
| `job://preview` | `{ jobId; dataUrl: string }` | (M2 では未実装。将来) |
| `image://created` | `ImageItem` | 画像が保存された時 |
| `image://deleted` | `{ ids: string[] }` | 削除時 |
| `assistant://delta` | `{ requestId; text }` | AI の回答の断片(v0.3) |
| `assistant://suggestion` | `{ requestId; index; summary; changes }` | 検証済みの設定変更の提案(v0.3) |
| `assistant://done` | `{ requestId; cancelled }` | 回答完了 |
| `assistant://error` | `{ requestId; code; message }` | 失敗 |
| `models://progress` | `DownloadState`(12 §8) | モデル取得の進捗(最大 4 回/秒。v0.4) |
| `models://done` | `DownloadState` | 取得の完了・失敗・キャンセル。完了時はフロントが `engine_list_models` を再取得する |
| `engine://install-state` | `InstallState`(23) | ComfyUI のインストールの状態・手順・ダウンロード進捗の変化(v0.15) |
| `engine://install-log` | `{ line }` | インストールのログ 1 行(pip などの出力。最新 500 行をアプリが保持) |

## エラーコード(初期)

| code | 意味 | UI の対応 |
|---|---|---|
| `ENGINE_UNREACHABLE` | ComfyUI に接続できない | 設定へのリンクと再接続ボタン |
| `ENGINE_BAD_RESPONSE` | 接続先が ComfyUI として期待する応答を返さない(別サービス / 非互換バージョン) | 設定の URL 確認を促す |
| `ENGINE_MODEL_MISSING` | 指定モデルが存在しない(`/prompt` が `value_not_in_list` を返した) | モデル再選択を促す |
| `ENGINE_NODE_MISSING` | 必要なカスタムノードが無い(例: GGUF、顔補正の Impact Pack、ControlNet の前処理)。`details.node` にノード名 | 導入手順を表示 |
| `ENGINE_EXECUTION_FAILED` | 生成失敗(OOM 等)。`details` に原文 | 解像度縮小などを提案 |
| `FACE_NOT_DETECTED` | 顔の入れ替えの後段合成で、対象画像から顔が検出できなかった(docs/46 §6 E) | 顔が大きく写る画像を使う、または「顔以外を元のまま保つ」を切って再生成 |
| `PRESET_NOT_FOUND` | 存在しないプリセット | トースト |
| `JOB_NOT_FOUND` / `JOB_INVALID_STATE` | 不正なジョブ操作(存在しない ID、状態が合わない操作) | トースト |
| `IMAGE_NOT_FOUND` | 画像/ファイル欠損 | トースト、一覧から除外提案 |
| `VALIDATION` | 入力不正。`details.field` | フォームにインライン表示 |
| `STORAGE_ERROR` | ファイル I/O 失敗 | トースト |
| `DB_ERROR` | DB 失敗 | 再起動案内 |
| `ASSISTANT_NO_KEY` / `ASSISTANT_NO_CONSENT` | API キー未登録 / 同意未取得(v0.3) | 設定への案内 / 同意ダイアログ |
| `ASSISTANT_AUTH` | API キーが無効(401/403) | キーの再登録を促す |
| `ASSISTANT_RATE_LIMIT` | 制限・過負荷(429/529) | 待って再送を促す |
| `ASSISTANT_UNREACHABLE` | api.anthropic.com に接続できない | ネットワーク確認を促す |
| `ASSISTANT_API_ERROR` | その他の API エラー。`details` はステータスとリクエスト ID のみ | トースト |
| `ASSISTANT_BUSY` | 別の相談が実行中 | 完了または停止を待つ |
| `ASSISTANT_CONVERSATION_NOT_FOUND` | 会話が存在しない | 一覧を更新 |
| `INTERNAL` | 想定外 | 詳細ログ参照案内 |

## 追記(v0.11・承認済み): img2img

- `job_enqueue` の `params.img2img`(`{ enabled, image, denoise }`)を追加する。新しいコマンドは無し(元画像は既存の `input_image_*`)。`engine_status.features` に `img2img` を加える。Inpaint が有効なときは `VALIDATION`。詳細は [19-image-to-anime.md](19-image-to-anime.md) §4・§7。

## 追記(v0.16・レビュー待ち): 参照画像(Qwen-Image 2.1)

- `job_enqueue` の `params.reference`(`{ enabled, images: string[], resolution }`。`images` は 1〜4 枚、`resolution` は 768 / 1024 / 1536)を追加する。新しいコマンドは無し(参照画像は既存の `input_image_*`)。`engine_status.features` に `referenceEdit` を加える。`qwen_image` 以外・他の画像入力機能との併用・枚数や `resolution` の範囲外は `VALIDATION`。詳細は [24-qwen-reference-image.md](24-qwen-reference-image.md) §4・§6。

## 追記(v0.12・承認済み): キュー管理

- `job_remove { ids }`(終了済みジョブを履歴から外す)、`queue_pause` / `queue_resume` / `queue_state`、イベント `queue://state` を追加する。`job_enqueue` は、カタログで種別が分かるモデル・エンコーダ・VAE が選択した種別と合わないとき `VALIDATION`(`details.field` は `model` / `clip` / `vae`)で断る。詳細は [20-ui-refresh.md](20-ui-refresh.md) §2.4・§3.4。

## 追記(v0.23・レビュー待ち): ローカル LLM

- `assistant_status` に `provider` / `localBaseUrl` / `localModel`、`assistant_update` に同名の引数を追加し、新コマンド `assistant_local_check` を加える。`local` では `ASSISTANT_NO_KEY` / `ASSISTANT_NO_CONSENT` を返さない。詳細は [11-ai-assistant.md](11-ai-assistant.md) §13.6。
- アプリ管理のローカル LLM 用に `llm_status` / `llm_install` / `llm_install_cancel` / `llm_remove` / `llm_start` / `llm_stop` / `llm_logs` / `llm_update_settings`、イベント `llm://state` / `llm://install-progress`、エラー `LLM_*` を追加する。`assistant.provider` に `local_external` を加える。詳細は [30-local-llm.md](30-local-llm.md) §8。
- 調査用ログのため `debug_status` / `debug_set_enabled` / `debug_export_logs` / `debug_open_log_dir` / `debug_clear_logs` を追加する。`settings_update` は `debug.enabled` を受け付けない(専用コマンドで変更)。`Settings` には含めない(状態は `debug_status` で取得。31 §13)。詳細は [31-debug-logging.md](31-debug-logging.md) §6。

## 追記(v0.29・承認済み・実装中): バックアップ

- 設定 > その他のバックアップ機能のため、`backup_estimate` / `backup_export` / `backup_inspect` / `backup_orphans` / `backup_restore` / `backup_cancel` / `backup_restart` とイベント `backup:progress` を追加する。型は `BackupInclude = { settings, gallery, presets }`(いずれも boolean)。パスはフロントから渡さず、Rust 側のダイアログで得る。
- エラーコード: `BACKUP_UNSUPPORTED`(形式・バージョン不正)、`BACKUP_CORRUPT`(ZIP・DB の破損)、`BACKUP_BUSY`(ジョブ実行中)、`BACKUP_NO_SPACE`(書き込み失敗: 容量不足)、`BACKUP_PATH_INVALID`(保存先がアプリのデータ領域内)。
- 詳細は [35-backup.md](35-backup.md) §5・§6。

## 追記(v0.31・承認済み・実装済み): ユーザー機能

- `user_list` / `user_current` / `user_create` / `user_login` / `user_logout` / `user_rename` / `user_delete` / `user_lock_set` / `user_lock_clear`(ユーザー系は未ログインでも呼べる)。データ系コマンドは、ログイン中のユーザーのデータだけを扱う(未ログインは `NOT_LOGGED_IN`)。フロントはユーザー ID をデータ系コマンドに渡さない。
- エラーコード: `NOT_LOGGED_IN`・`AUTH_FAILED`・`AUTH_LOCKED_OUT`(`details.retryAfterMs`)・`USER_NAME_TAKEN`・`USER_LIMIT`・`USER_LAST`・`USER_BUSY`。
- `user_delete` は `{ externalImagesDir: string | null }` を返す(作業スペースの外にあった画像保存先。自動では削除しない)。
- 詳細は [36-users.md](36-users.md) §9・§10。

## 追記(v0.33・承認済み・実装済み): ユーザーの色の変更

- `user_set_color`: 入力 `{ id, color }`(`color` は 36 §3 のパレットのキー)→ `UserSummary`。ログイン中の本人の `id` のみ(他人は `FORBIDDEN`、範囲外の色は `VALIDATION`)。詳細は [38-simple-mode-and-accent.md](38-simple-mode-and-accent.md) §3。

## 追記(v0.34・承認済み): 画像からタグを作る

- `image_tag`: 入力 `{ image: string, model?: string, threshold?: number, characterThreshold?: number, excludeTags?: string }` → `{ tags: string, model: string }`。`image` は取り込み済みの参照画像のパス(既存の `input_image_*`)。生成ジョブが実行中・待機中は `ENGINE_BUSY`、ノードが無ければ `ENGINE_NODE_MISSING`(`details.node = "WD14Tagger|pysssss"`)、範囲外・画像欠損は `VALIDATION` / `IMAGE_NOT_FOUND`。ジョブ・履歴には残らない。
- `engine_status.features` に `imageTag: boolean`、`EngineModels` に `taggers: string[]`(解析モデルの候補。ノードが無ければ空)を加える。
- 詳細は [39-image-to-prompt.md](39-image-to-prompt.md) §5・§6。

## 追記(v0.35・承認済み・実装済み): プロジェクト

- 新コマンド: `project_list`(`{ projects, totalImages, favoriteImages, unassignedImages }`)・`project_get`(`{ project, lastParams }`)・`project_create`(`{ name, color? }`)・`project_update`(`{ id, name, color }`。名前と色を**置き換える**。`color` が null なら色なし)・`project_reorder`(`{ ids }`。全プロジェクトを過不足なく)・`project_delete`(`{ id }` → 未分類へ戻った画像数)・`image_move`(`{ ids, projectId }`。`projectId` が null なら未分類。最大 500 件)。
- 既存コマンドの拡張(省略時は従来どおり): `job_enqueue` に `projectId?: string | null`、`image_list` に `scope?: "all" | "unassigned" | { projectId }`。`ImageItem` / `Job` に `projectId: string | null`。
- イベント: `project://changed`(ペイロードなし)、`image://moved`(`{ ids, projectId }`)。
- エラー: `PROJECT_NOT_FOUND`。名前の重複・長さ・色の不正は `VALIDATION`(`details.field = "name" | "color"`)。
- 詳細は [40-projects.md](40-projects.md) §7。

## 追記(v0.37・承認済み・実装済み): 顔の入れ替え

- `job_enqueue` の `params.faceSwap`(`{ enabled, target, face, keepOutside, region: "head" | "face", resolution, detector }`)を追加する。`target` は対象 1 枚(複数の対象は呼び出し側が 1 枚ずつ投入する)。`qwen_image` 以外・他の画像入力機能(`reference`・`inpaint`・`img2img`・`controlnet`・`faceDetailer`)との併用・`batchSize` が 1 以外・画像パス / `resolution` / `detector` の不正は `VALIDATION`。画像が無ければ `IMAGE_NOT_FOUND`。
- `input_image_pick_many` を追加する(上の表)。
- `engine_status.features` に `faceSwap`(参照画像つき生成 + LoRA の適用ノード + 顔入れ替え LoRA の導入)と `faceSwapComposite`(後段合成に必要な 9 ノード)を加える。詳細は [42-face-replace.md](42-face-replace.md) §4・§6・§14。

## 追記(v0.38・レビュー待ち): 画像の形式変換

- 新コマンド: `convert_pick_files`・`convert_pick_folder`(→ `ConvertEntry[]`。ダイアログは Rust 側)・`convert_pick_output_dir`・`convert_remove`(`{ ids }`)・`convert_clear`・`convert_run`(`{ ids, format, quality, webpLossless, pngCompression, background, overwrite }` → `{ runId }`)・`convert_cancel`(`{ runId }`)・`convert_open_output_dir`。`ConvertEntry = { id, name, detectedFormat, width, height, bytes, error? }`(`detectedFormat` は `webp`・`png`・`jpeg`・`gif`・`bmp`・`tiff`・`heic`。`heic` は 43 §13)。ドロップされたパスは Rust 側が受け、フロントには `id` だけ渡す。
- イベント: `convert://added`・`convert://progress`・`convert://item`・`convert://finished`。
- エラー: 出力先の未設定・書き込み不可は `VALIDATION`(`details.field = "outputDir"`)。
- 既存コマンド: `input_image_pick`・`input_image_pick_many`・`input_image_from_gallery` の取り込み対象に WebP を加える(保存は従来どおり `inputs/<uuid>.png`)。
- 詳細は [43-image-convert.md](43-image-convert.md) §8・§9。

## 追記(v0.53・承認済み・実装済み): 全体設定と個別設定(52)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `settings_scope_get` | – | 個別設定にしている組の一覧 |
| `settings_scope_set` | `{ group; overridden: boolean }` | – (個別にすると全体の値を写す。戻すと個別の値を消す) |
| `settings_scope_reset_global` | `{ group }` | – (全体設定の組を標準値に戻す) |
| `user_delete_preview` | `{ id }` | 削除で一緒に消える画像フォルダ(`string \| null`) |

- `group`: `theme` / `language` / `assistantConnection` / `assistantPrompt` / `assistantAutoDelete` / `translate` / `convert`。
- `settings_get`・`assistant_status`・`assistant_prompt_*`・`translate_status`・`convert_settings_*`・`convert_pick_output_dir` に任意の引数 `scope` を、`settings_update`・`assistant_update`・`translate_update` の `patch` に `scope` を追加した。読み取りの `"global"` は全体の値、省略時は実際に使う値。書き込みの省略時は個別。個人の項目は `scope` に関係なくユーザーに保存する。
- `settings_get({ scope: "global" })` は、ログイン前も呼べる。
- `Settings` に `imagesRoot` を追加した。
