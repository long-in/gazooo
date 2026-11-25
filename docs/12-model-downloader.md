# 12. モデル取得機能(v0.4)

> ステータス: 🟡 レビュー待ち(2026-11-19 起案)。決定してほしい点は §12 と 07 B4(Q-32〜38)。
> **経緯**: 起案後、ユーザーから「モデルのダウンロード機能は最新版で開発してください」と指示があり、**推奨案(Q-32〜38)のまま M9 として実装した**。この指示を実装への着手の許可として扱ったが、設計資料の承認(🟢)は **ユーザーのレビューで行う**(未承認のまま)。レビューで方針が変わった場合は、実装を直す。

## 1. 目的とスコープ

ComfyUI の `models/` フォルダへ、必要なモデルを **アプリ内からダウンロードして配置** できるようにする(F-14)。
これまでは「モデルが見つかりません。models フォルダに配置してください」と表示するだけで、取得元の調査・ファイル選び・配置はユーザーが手作業で行う必要があった(08 §5、10 §3)。

**やること**
- **専用ページ「モデル取得」(`/models`)** を設ける。モデルの種類ごとに、複数のバージョン(ベースモデル・精度・サイズ)から **ユーザーが選んで** ダウンロードする。
- 取得したファイルを、ComfyUI が読む正しいサブフォルダ(`checkpoints/` `controlnet/` `ultralytics/bbox/`)へ置く。
- 取得済みかどうかの表示、進捗表示、キャンセル、再開、完全性の検証(SHA-256)。
- 生成画面などの「モデルがありません」案内から、このページへ誘導する。

**やらないこと(v0.4)**
- 任意の URL・Civitai・Hugging Face の検索や、アカウント認証が必要な(gated)モデルの取得(§12 Q-33)。
- LoRA の取得、および Flux 本体(safetensors。約 24GB)の取得(Flux は GGUF 版のみ対象。§4.2)。
- ComfyUI 本体・カスタムノードの導入(08 の開発用スクリプトの範囲のまま)。
- モデルの削除・移動(§12 Q-36)。

## 2. 方針と、既存方針との関係

| 既存の方針 | この機能での扱い |
|---|---|
| 01 非ゴール「モデルの配布・ダウンロードマーケット」 | **マーケットは作らない。** アプリが同梱する **厳選カタログ**(固定の取得元・ハッシュ)から選ぶだけ。01 の非ゴールの文言を「任意モデルの検索・配布マーケット」に明確化する |
| 01 プライバシー「通信先は設定されたエンジンのみ」 | AI サポート(11)に続く **2 つ目の例外**。**ユーザーが「ダウンロード」を押したときだけ**、許可リストのホスト(§6)へ接続する。ページを開いただけ・起動時・バックグラウンドでは通信しない。送るのは取得するファイルの URL への GET のみで、ユーザー情報・プロンプト・画像は一切送らない |
| 08 §1「モデルは自動ダウンロードしない」 | 開発用スクリプト(`comfyui:setup`)は従来どおり **自動ダウンロードしない**。アプリの機能は **ユーザーの明示操作ごと** の取得なので矛盾しない(08 に注記) |
| 10 §3「アプリ自身は設定されたエンジン以外へ通信しない」 | 上記と同じ例外として、10 に参照を追記 |
| 02 / AGENTS.md「バックエンド処理・ファイル I/O は Rust 側」 | ダウンロード・検証・配置はすべて Rust。WebView からは通信しない(**CSP の `connect-src` は変更しない**) |

## 3. 前提: モデルフォルダの特定

ダウンロードは **アプリと同じ PC の ComfyUI** にだけ配置できる(リモートの ComfyUI のフォルダには書けない)。

1. 設定 `engine.comfyui.models_dir`(新設、既定は null = 自動検出)が指定されていれば、それを使う。
2. null なら、ComfyUI の `GET /system_stats` の `system.argv` から検出する。
   - `--base-directory <dir>` があれば `<dir>/models`(08 のセットアップがこの形式)。
   - なければ、`argv[0]`(`main.py` のパス)の親ディレクトリ + `models`。
   - 接続先が `localhost` / `127.0.0.1` / `::1` 以外の場合は検出しない(リモートとみなす)。
3. 特定できない場合は、取得ボタンを無効にし、「ComfyUI の models フォルダを設定してください」と、設定画面への導線(フォルダ選択ダイアログ)を表示する。`MODELS_DIR_UNKNOWN`。
4. 特定したフォルダが存在し、書き込めることを確認する(取得開始前)。
5. 取得先サブフォルダはカタログの値だけを使い(§4)、ファイル名は固定。**ユーザー入力や配布元の応答から配置パスを組み立てない**(パストラバーサル対策)。

**保存先設定は ComfyUI の検索先を変えない(v0.6 追記・🟢 承認済み)**: 上記 1 の設定はアプリのダウンロード先だけを決める。ComfyUI にそのフォルダを認識させる手順(画面の [ComfyUI に反映] による `extra_model_paths.yaml` の書き込みと、ComfyUI の再起動)と、食い違いの警告は 08 §6.2 に従う。

ComfyUI は配置後の新規ファイルを再起動なしで認識する(08 の検証で確認済み)。取得完了後にアプリは `engine_list_models` を再取得する。

## 4. カタログ

アプリに **同梱する JSON**(`src-tauri/resources/models/catalog.json`)として持つ。更新はアプリのリリースに載せる(実行時にカタログを外部から取得しない。§12 Q-32)。

### 4.1 構造

```ts
interface ModelCatalog { schemaVersion: 1; families: ModelFamily[] }

interface ModelFamily {
  id: string;                    // 例 "controlnet-canny"
  category: "checkpoint" | "controlnet" | "detector" | "flux";
  name: { ja: string; en: string };
  description: { ja: string; en: string };
  usedBy?: string[];             // 例 ["F-12 ControlNet"]。UI でどの機能に必要かを示す
  variants: ModelVariant[];
}

interface ModelVariant {
  id: string;                    // 例 "sdxl-fp16"
  label: { ja: string; en: string };   // 例「SDXL 用 / fp16」
  baseModel?: "sd15" | "sdxl";   // フィルタ・不一致の警告に使う
  precision?: "fp8" | "fp16" | "fp32";
  recommended?: boolean;
  source: {
    url: string;                 // https の固定 URL(リビジョン固定の resolve/<commit>/…)
    repo: string;                // 例 "diffusers/controlnet-canny-sdxl-1.0"(表示・出典リンク用)
  };
  sizeBytes: number;
  sha256: string;                // 取得後に必ず照合する
  destDir: "checkpoints" | "controlnet" | "ultralytics/bbox" | "unet" | "clip" | "vae";
  fileName: string;              // 配置するファイル名(固定)
  license: { name: string; url: string; commercialUse: boolean | "see-license" };
  requiresConsent: boolean;      // true なら取得前に確認ダイアログ(§7)
  note?: { ja: string; en: string };   // 例「pickle 形式。信頼できる配布元のみ」
}
```

- `source.url` は **リビジョン(コミット)を固定** した URL にする(`main` ではなく)。配布元の差し替えで挙動が変わらず、ハッシュと一致し続ける(AGENTS.md の「バージョン固定」の方針に合わせる)。
- `sha256` は、カタログ作成時に配布元の公開値と、実際にダウンロードしたファイルの両方で確認して記入する。
- カタログの追加・更新は、依存更新と同様に **単独の変更** として行い、ユニットテスト(スキーマ検証・ホスト許可リスト・パス検証)を通す。

### 4.2 初期カタログ(案)

配布元はいずれも認証不要(gated でない)ことを 2026-11-19 に確認済み。**バージョンの選択が必要なもの** は、ファミリー内の複数 variant として並べ、ユーザーが選ぶ。

**チェックポイント(`checkpoints/`)**

| ファミリー | variant | サイズ | 配布元 | ライセンス |
|---|---|---|---|---|
| Stable Diffusion 1.5 | `v1-5-pruned-emaonly.safetensors`(推奨) | 約 4.27GB | stable-diffusion-v1-5/stable-diffusion-v1-5 | CreativeML OpenRAIL-M |
| Stable Diffusion XL | `sd_xl_base_1.0.safetensors`(推奨) | 約 6.94GB | stabilityai/stable-diffusion-xl-base-1.0 | OpenRAIL++ |

**ControlNet(`controlnet/`)**。種類(Canny / Depth / OpenPose)ごとにファミリーを作り、**ベースモデル(SD1.5 / SDXL)× サイズ** を variant として選ぶ。ベースモデルと合わないものを選ぶと生成時にエラーになるため(10 §3)、UI でベースモデルを必ず見せる。

| ファミリー | variant | サイズ | 配布元 |
|---|---|---|---|
| Canny | SDXL / xinsir 版(推奨) | 約 2.5GB | xinsir/controlnet-canny-sdxl-1.0(Apache-2.0、2024-07 更新) |
| Canny | SDXL / diffusers 版 | 約 2.5GB | diffusers/controlnet-canny-sdxl-1.0(OpenRAIL++、2023-09 更新)。`fp16.safetensors` |
| Canny | SD1.5 / fp16(推奨) | 約 723MB | comfyanonymous/ControlNet-v1-1_fp16_safetensors(`control_v11p_sd15_canny_fp16`) |
| Depth | SDXL / xinsir 版(推奨) | 約 2.5GB | xinsir/controlnet-depth-sdxl-1.0(Apache-2.0、2024-07 更新) |
| Depth | SDXL / diffusers 版 | 約 2.5GB | diffusers/controlnet-depth-sdxl-1.0(OpenRAIL++、2024-04 更新)。`fp16.safetensors` |
| Depth | SD1.5 / fp16(推奨) | 約 723MB | 同上(`control_v11f1p_sd15_depth_fp16`) |
| OpenPose | SDXL / xinsir 版(推奨) | 約 2.5GB | xinsir/controlnet-openpose-sdxl-1.0(Apache-2.0、2024-07 更新) |
| OpenPose | SDXL / thibaud 版 | 約 5GB | thibaud/controlnet-openpose-sdxl-1.0(`OpenPoseXL2`、fp32 のみ。ライセンスは配布元の表記を確認して記載) |
| OpenPose | SD1.5 / fp16(推奨) | 約 723MB | 同上(`control_v11p_sd15_openpose_fp16`) |

- **最新性の確認(2026-11-19)**: SD1.5 用 ControlNet v1.1、SD 1.5、SDXL base 1.0 は、それぞれの系列の **最終版**(後継のバージョンアップはない)。SDXL 用 ControlNet は、上表の xinsir 版が diffusers / thibaud 版より新しく(2024 年)、配布元の自己申告では精度も高い。ComfyUI の `ControlNetLoader` でそのまま使える(アプリのワークフローを変えずに済む)ため、SDXL は **xinsir 版を推奨・旧版を代替** とする。比較の根拠は配布元の自己申告のため、実装時に実機で生成結果を確認してから推奨を確定する。
- **xinsir の Union 版(1 ファイルで全種類)は初期カタログに含めない**。ComfyUI 側で `SetUnionControlNetType` ノードが追加で必要で、10 §5 のワークフローの変更を伴うため(別件の設計変更として検討)。
- SD3.5 など別系列のベースモデルと、Flux 用の ControlNet は、10 の対象外(ControlNet は `sd15` / `sdxl` のみ)のため含めない。

- ControlNet ファイルは、アプリの「前処理に応じた自動切り替え」(10 §8)が働くよう、**ファイル名に `canny` / `depth` / `openpose` を含める**(`fileName` で固定)。
- SD1.5 用の comfyanonymous 配布分は、配布元にライセンス表記がない。元の lllyasviel/ControlNet-v1-1(OpenRAIL 系)を出典として明記し、ライセンス欄は「出典の表記に従う」(`commercialUse: "see-license"`)にする。実装時に再確認し、確認できないものはカタログに入れない。

**Flux(`unet/` `clip/` `vae/`)**(生成画面の「Flux(GGUF)」で使う。**ページの「Flux」タブ**にまとめる)。Flux の生成には、本体(GGUF)・テキストエンコーダ 2 種・VAE の **すべて** が要るため、ファミリーの説明に必要な組み合わせを書く。生成画面で Flux のモデルが足りないときは、この「Flux」タブへ案内する。

| ファミリー | 配置先 | variant | サイズ | 配布元 / ライセンス |
|---|---|---|---|---|
| FLUX.1 schnell(GGUF) | `unet/` | Q4_K_S(推奨)/ Q5_K_S / Q8_0 | 6.8GB / 8.3GB / 12.7GB | city96/FLUX.1-schnell-gguf、Apache-2.0(同意不要) |
| FLUX.1 dev(GGUF) | `unet/` | Q4_K_S(推奨)/ Q5_K_S / Q8_0 | 6.8GB / 8.3GB / 12.7GB | city96/FLUX.1-dev-gguf、**FLUX.1 [dev] Non-Commercial License(非商用。同意ダイアログあり)** |
| テキストエンコーダ clip_l | `clip/` | clip_l | 246MB | comfyanonymous/flux_text_encoders、Apache-2.0 |
| テキストエンコーダ t5xxl | `clip/` | fp8(推奨)/ fp16 | 4.9GB / 9.8GB | 同上 |
| VAE ae | `vae/` | ae.safetensors | 335MB | Comfy-Org/Lumina_Image_2.0_Repackaged、Apache-2.0 |

- 公式の `ae.safetensors`(black-forest-labs/FLUX.1-schnell・dev)は **認証が必要(gated)** で、アプリは認証が要るモデルを扱わない(Q-33)。代わりに、Comfy-Org の再配布(認証不要)を使う。**公式と SHA-256 が完全に一致する**(`afc8e282…9e38`、335,304,388 バイト。公式・Comfy-Org の 3 つのリポジトリで一致を確認)ので、同一のファイルである。この一致はカタログのテストでも固定している。
- GGUF の量子化は、Q4_K_S を推奨(開発用の RTX 2060 12GB・Mac 16GB でも扱いやすい。08 §5)。Q8_0 は高品質だがサイズが大きく、VRAM に余裕が要る。
- GGUF を使うには、ComfyUI に ComfyUI-GGUF ノードが必要(08 のセットアップで導入済み)。variant の `note` で案内する。
- 画面の対応は、clip_l →「テキストエンコーダ (CLIP-L)」、t5xxl →「テキストエンコーダ (T5)」、ae →「VAE」。
- Flux 本体の safetensors 版(約 24GB)は対象外(§1)。

**検出モデル(`ultralytics/bbox/`)**(顔補正 F-11。配布元は Bingsu/adetailer、Apache-2.0)

| ファミリー | variant | サイズ |
|---|---|---|
| 顔検出 | `face_yolov9c.pt`(**推奨**・最高精度) / `face_yolov8m.pt`(標準・動作確認済み) / `face_yolov8s.pt` / `face_yolov8n_v2.pt` | 51.6MB / 52MB / 22.5MB / 6.2MB |
| 手検出 | `hand_yolov9c.pt`(**推奨**・最高精度) / `hand_yolov8s.pt`(標準・動作確認済み) / `hand_yolov8n.pt` | 51.6MB / 22.5MB / 6.2MB |

- **最新性の確認(2026-11-19)**: 配布元の Bingsu/adetailer は 2024-11 から更新がなく、より新しい公開モデルは見つからなかった。精度(mAP50)は顔が v8m 0.737 → **v9c 0.748**、手が v8s 0.794 → **v9c 0.810** で、v9c が最高。「最新版で」との指示により **v9c を推奨** にした。確認済みの範囲は、導入済みの ultralytics 8.4.215(Windows の開発用 ComfyUI)で `face_yolov9c.pt` を `YOLO()` として読み込めること(検出タスク・クラス `face`)まで。**Impact Subpack 経由の顔補正(`FaceDetailer`)での動作は未確認**のため、09 の検証項目で確認する。問題があれば推奨を v8m / v8s に戻す(カタログの `recommended` を入れ替えるだけでよい)。

- 検出モデルの `.pt` は **pickle 形式**で、読み込み時に任意コードが実行されうる(08 §6.1)。カタログの固定ハッシュと一致したファイルだけを配置することで、取得元の改ざん・差し替えを防ぐ。この注意書きを variant の `note` に表示する。

## 5. 画面(`/models`)

ルートに `/models` を追加し、サイドナビに「モデル」を置く(05 §2)。

```
┌ モデル取得 ─────────────────────────────────────────────┐
│ 保存先: C:\...\comfyui\data\models  [変更]   空き 90GB   │
│ [チェックポイント] [ControlNet] [検出モデル]   ベース: [すべて▾]│
├─────────────────────────────────────────────────────────┤
│ ControlNet — Canny      (ControlNet で使用)              │
│   輪郭から構図を指定します。                             │
│   バージョン: ◉ SDXL / fp16  2.5GB  ✔ 導入済み           │
│               ○ SD1.5 / fp16 723MB                       │
│   ライセンス: OpenRAIL++ ・出典: diffusers/…   [ダウンロード]│
│ ─────────────────────────────────────────────────────── │
│ ControlNet — Depth …                                     │
├─────────────────────────────────────────────────────────┤
│ ダウンロード中: controlnet-depth… ▓▓▓▓░░ 62%  38MB/s [キャンセル]│
└─────────────────────────────────────────────────────────┘
```

- **カテゴリのタブ**と、**ベースモデルのフィルタ**(すべて / SD1.5 / SDXL)。
- ファミリーごとのカードに、**variant のラジオ選択**(ラベル・サイズ・導入済みバッジ)。推奨 variant は既定で選択済み。
- 各カードに、ライセンス名(リンク)と出典リポジトリ(リンク)を表示する。
- 「ダウンロード」: 導入済み variant では無効(「導入済み」と表示)。`requiresConsent` の variant では確認ダイアログ(§7)を経る。
- 画面下部(または共通の固定領域)に **ダウンロードキュー**(§8)。ページを離れても取得は続行し、ミニ進捗はサイドナビのバッジで示す。
- **他画面からの導線**: 生成画面でモデル・ControlNet・検出モデルが無いときの案内(10 §8)に「モデル取得ページを開く」ボタンを置き、該当カテゴリのタブへ遷移する(`/models?category=controlnet&base=sdxl`)。
- 状態: モデルフォルダ未設定 / オフライン(取得時のみ判明)/ 容量不足 / 取得失敗(再試行)を、05 §6 の方針でそれぞれ表示。
- i18n(日本語・英語)、キーボード操作、`aria-valuenow` つきの進捗(05 §7)。

## 6. ダウンロードの仕様(Rust)

- HTTP クライアントは既存の `reqwest`(ストリーミング)を使う。**TLS 検証は無効化しない。**
- **許可リスト**: `https` のみ。ホストはカタログの URL のホスト(`huggingface.co`)と、そのリダイレクト先として妥当な `huggingface.co` / `hf.co` のサブドメインに限る(Hugging Face は大きなファイルを CDN サブドメインへリダイレクトする)。許可外のホストへのリダイレクトは中止(`MODELS_HOST_NOT_ALLOWED`)。リダイレクトは最大 5 回。
- **一時ファイル**: 配置先と同じフォルダに `<fileName>.part` として書き、検証後に `rename` で確定する(中途半端なファイルを ComfyUI に見せない)。
- **再開**: `.part` が残っていれば `Range` ヘッダで続きから取得する(サーバーが非対応なら最初から)。再開前に、既存の `.part` のサイズが `sizeBytes` 以下であることを確認する。
- **検証**: 全量受信後に SHA-256 を計算し、カタログの値と照合する。不一致なら `.part` を削除して `MODELS_HASH_MISMATCH`(自動再試行はしない)。サイズ超過も同様に中止。
- **事前確認**: 開始前に空き容量(`sizeBytes` の 1.1 倍以上)を確認(`MODELS_DISK_FULL`)。配置先に同名ファイルが既にある場合は **上書きしない**(導入済みとして扱う。ハッシュ照合は「検証」操作で行える。§12 Q-36)。
- **キャンセル**: 受信を中止し、`.part` は(再開用に)残すか削除するかを選べる(既定は残す。キューの「破棄」で削除)。
- **同時実行は 1 件**(キューで順番待ち)。帯域と書き込みの競合を避け、進捗表示を単純にするため。
- **タイムアウト**: 接続 30 秒、受信の無通信 60 秒で中断し、再試行を促す(`MODELS_DOWNLOAD_FAILED`)。
- **アプリ終了時**: 取得を中断し `.part` を残す。次回起動時は自動再開せず、キューにも復元しない(ユーザー操作で再開)。
- パスは `std::path` で組み立てる(Windows の長いパス・区切り文字に注意。06 の方針)。ファイル名は固定値のみで、区切り文字・`..` を含むカタログはロード時に拒否する。
- ログには URL とファイル名、結果のみを出す(トークン等は扱わない)。

## 7. ライセンスと同意

- 各 variant にライセンス名とリンクを表示する(§5)。
- `requiresConsent: true`(例: OpenRAIL 系で使用制限がある、`commercialUse` が `"see-license"` など)の variant は、**初回のダウンロード前にライセンス確認ダイアログ**を出し、リンク先を確認のうえ同意した場合のみ開始する。同意は variant 単位で DB に保存し、2 回目以降は再表示しない(§9)。
- アプリはモデルを再配布しない(配布元から直接取得する)。ライセンスの遵守は利用者の責任である旨を、ダイアログに記載する。
- Flux dev のような **非商用ライセンス** や、アカウント認証が必要なモデルは、v0.4 のカタログに入れない。

## 8. IPC(04 に追記する内容)

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `models_catalog` | – | `{ catalog: ModelCatalog; installed: InstalledStatus[]; modelsDir: string \| null; modelsDirSource: "setting" \| "detected" \| "unknown"; freeBytes: number \| null }`(外部へは通信しない。保存先の自動検出のため、設定された ComfyUI へ `/system_stats` を問い合わせることはある) |
| `models_download_start` | `{ familyId; variantId; consent?: boolean }` | `{ downloadId }`(同意が必要で `consent` が無ければ `MODELS_NO_CONSENT`) |
| `models_download_cancel` | `{ downloadId; discard?: boolean }` | – |
| `models_download_list` | – | `DownloadState[]`(キューと直近の完了・失敗) |
| `models_verify` | `{ familyId; variantId }` | `{ ok: boolean }`(導入済みファイルのハッシュ照合。通信しない) |
| `models_set_dir` | `{ path: string \| null }` | `{ modelsDir; source }`(存在・書き込みを検証。null で自動検出に戻す) |

```ts
interface InstalledStatus { familyId: string; variantId: string; installed: boolean; sizeMatches: boolean }
interface DownloadState {
  downloadId: string; familyId: string; variantId: string;
  status: "queued" | "downloading" | "verifying" | "done" | "failed" | "cancelled";
  receivedBytes: number; totalBytes: number; bytesPerSec?: number;
  error?: { code: string; message: string };
}
```

- **導入済み判定**: `destDir/fileName` が存在し、サイズが `sizeBytes` と一致すること(通常の画面表示ではハッシュを計算しない。数 GB の計算を避けるため)。厳密な確認は `models_verify`。
- イベント: `models://progress`(`DownloadState`、最大 4 回/秒)、`models://done`(`DownloadState`。完了時はフロントが `engine_list_models` を再取得)。
- `settings_update` に `engine.comfyui.models_dir` は含めず、専用コマンド `models_set_dir` で検証つきで変更する(`imagesDir` と同様の扱い)。
- 新エラーコード: `MODELS_DIR_UNKNOWN`(フォルダ未特定・書き込み不可)/ `MODELS_NO_CONSENT` / `MODELS_HOST_NOT_ALLOWED` / `MODELS_DISK_FULL` / `MODELS_DOWNLOAD_FAILED`(通信失敗・タイムアウト・HTTP エラー。`details.status`)/ `MODELS_HASH_MISMATCH` / `MODELS_ALREADY_INSTALLED` / `MODELS_NOT_FOUND`(カタログに無い ID)。

## 9. 設定とデータ(03 に追記する内容)

| key | 型 | 既定値 |
|---|---|---|
| `engine.comfyui.models_dir` | string \| null | null(= `/system_stats` から自動検出) |
| `models.consents` | string[](同意済みの `familyId/variantId`) | `[]` |

- ダウンロードの状態は **メモリ上のみ**(DB に履歴テーブルは作らない)。再起動で消えてよい(`.part` はディスクに残る)。
- DB マイグレーションは不要(`settings` テーブルのキー追加のみ。AGENTS.md §5 のスキーマ変更に当たらない)。

## 10. セキュリティ・プライバシーのまとめ

- 通信は Rust のみ。WebView の CSP は変更しない(R-05 と同様、WebView に外部通信を許さない)。
- 通信は **ユーザーの明示操作時のみ**、許可リストのホストのみ、https のみ。リクエストにユーザー識別情報・プロンプト・画像・API キーを含めない(User-Agent は `gazooo` + アプリのバージョンのみ。メールアドレス等は含めない)。
- 取得物は固定ハッシュで検証してから配置する。pickle 形式(`.pt`)は特に、ハッシュ不一致のものを絶対に配置しない。
- 配置先はカタログ由来の固定サブフォルダとファイル名のみ。既存ファイルは上書きしない。
- Windows / macOS の双方で同一動作(パス・空き容量取得は OS 差異を吸収する)。

## 11. 実装方針(承認後)・テスト・受け入れ基準

**マイルストーン M9**: カタログ(JSON+スキーマ検証)、ダウンロード基盤(許可リスト・`.part`・再開・ハッシュ検証・キャンセル・キュー)、モデルフォルダ検出、IPC、`/models` ページ、他画面からの導線、i18n、09 への検証項目追加。

**テスト**
- Rust: カタログのスキーマ・許可リスト・パス検証(`..`・区切り文字・不正ホストの拒否)のユニットテスト。ローカルのモックサーバーで、正常取得・ハッシュ不一致・`Range` 再開・キャンセル・非許可ホストへのリダイレクト・タイムアウト・容量不足を検証(ネットワークに出ない)。`system_stats` の `argv` からのフォルダ検出(`--base-directory` あり/なし、リモート URL)。
- フロント(Vitest): variant 選択・導入済み表示・フィルタ・進捗表示のロジックと、主要フローのコンポーネントテスト。
- 実機(09): 小さいモデル(顔検出 `face_yolov8n.pt` 6MB)での取得 → ComfyUI の候補に出る確認、Windows / macOS 双方。

**受け入れ基準**
1. 未導入の ControlNet(Canny)を、SDXL / SD1.5 の variant から選んで取得でき、取得後に再起動なしで生成画面のモデル候補に現れる。
2. 取得中に進捗・速度が表示され、キャンセル・再開ができる。
3. ハッシュが一致しないファイルは配置されない。既存ファイルは上書きされない。
4. 「ダウンロード」を押さない限り、外部へ一切通信しない(ページを開いても通信しない)。
5. モデルフォルダが特定できない場合(リモートの ComfyUI など)は、取得ボタンが無効で、設定の案内が出る。
6. Windows / macOS の両方で動作する。

## 12. 決定してほしい点(07 B4 にも記載)

| ID | 論点 | 推奨 |
|---|---|---|
| Q-32 | カタログの更新方法 | **アプリ同梱の JSON、更新はアプリのリリースで**。実行時に外部からカタログを取らない(プライバシー・安全性を優先)。新しいモデルの追加はアプリ更新が必要になる |
| Q-33 | 任意 URL・Civitai・Hugging Face トークン(gated モデル)の対応 | **v0.4 では対応しない**(厳選カタログのみ)。需要があれば後続で、トークンの保管方法(11 の API キー同様に平文か資格情報ストアか)と合わせて設計する |
| Q-34 | 初期カタログの範囲 | **§4.2 のとおり**(SD1.5 / SDXL のベース、ControlNet 3 種 ×(SD1.5 / SDXL)、顔・手の検出モデル、**Flux(schnell / dev の GGUF、テキストエンコーダ、VAE)**)。LoRA・Flux 本体(safetensors)は含めない。Flux は「モデルがありません」の報告を受けて追加(2026-11-19、ユーザーが schnell と dev の両方を指定) |
| Q-35 | モデルフォルダの特定方法 | **設定の指定 > `/system_stats` の `argv` から自動検出**。リモート ComfyUI は取得機能の対象外 |
| Q-36 | 導入済みモデルの削除・検証・移動 | **削除・移動は v0.4 では作らない**(誤削除の影響が大きく、フォルダを直接操作できる)。**検証(ハッシュ照合)のみ提供**。同名ファイルがある場合は上書きしない |
| Q-37 | ライセンス同意 | **`requiresConsent` の variant だけ、初回に確認ダイアログ**。同意は variant ごとに保存 |
| Q-38 | 同時ダウンロード数・自動再開 | **同時 1 件**。アプリ終了後の自動再開はしない(`.part` は残し、手動で再開) |


## 追記(v0.9・M13): 拡散モデル本体の種別と、まとめて取得

- `ModelCategory` に `anima` / `qwen_image` / `krea2` を追加(取得ページのタブ)。`ModelFamily` に任意の `kind`(`flux_gguf | anima | qwen_image | krea2`。`unet/` に置く拡散モデル本体だけが持ち、`profile` が必須)と `requires`(一緒に必要なテキストエンコーダ・VAE への参照)を追加する。以前の「`unet/` に置くもの = Flux」という推定は `kind` に置き換える。配置先とファイル名と SHA-256 がすべて同じ variant は、複数のファミリーが共有できる(Anima と Krea 2 の VAE)。取得ページのカードに [必要なファイルをまとめて取得] を出す。詳細は [17-new-model-kinds.md](17-new-model-kinds.md) §5。

## 追記(v0.10・承認済み): モデルのプロフィール

- `ModelFamily` に必須の `profile`(用途・得意/不得意・プロンプトの書き方・推奨設定・必要メモリ・NSFW の表記)を追加し、`schemaVersion` を 2 にする。生成画面の説明パネルと AI のモデルガイドの単一の情報源とする。詳細は [18-model-profiles.md](18-model-profiles.md) §3。
