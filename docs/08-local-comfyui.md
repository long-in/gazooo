# 08. ローカル ComfyUI 開発環境

アプリの接続先となる ComfyUI を、このリポジトリ内で再現可能に用意するための仕組み。
**アプリ本体のランタイム要件ではなく、開発・検証用のツール**(01 の MVP 方針どおり、アプリは「起動済みの ComfyUI」に接続する)。
ユーザーが既に ComfyUI を持っている場合は、これを使わず設定画面で URL を指定すればよい。

> ステータス: 🟢 承認済み(2026-11-19)

## 1. 方針

- セットアップは **スクリプトで再現可能** にし、ComfyUI・カスタムノード・PyTorch はすべて **バージョン固定**(AGENTS.md「依存バージョンの固定」)。
- 固定値は `tools/comfyui/versions.env` に集約する。更新は単独の変更として行う。
- 生成物(ComfyUI 本体・venv・モデル・出力)は `comfyui/` 配下に置き、**git 管理しない**。
- OS 差異は OS 別スクリプト(macOS: bash / Windows: PowerShell)で吸収し、呼び出しは `npm run` スクリプトに統一する(06 の方針)。
- モデル(数 GB)・ライセンス同意が必要なものは、**このスクリプトでは自動ダウンロードしない**。置き場所と推奨モデルを案内する。なお、アプリ本体には v0.4 でモデル取得ページ(12。ユーザーの明示操作ごとに取得。レビュー待ち)を設ける予定で、スクリプトの方針は変わらない。

## 2. 構成

```
tools/comfyui/                 # git 管理(スクリプトと固定値)
  versions.env                 # 固定バージョン(ComfyUI タグ / GGUF コミット / Python / PyTorch)
  run.mjs                      # OS を判定して下記スクリプトを呼ぶディスパッチャ
  setup.sh   setup.ps1         # セットアップ(冪等)
  start.sh   start.ps1         # 起動(ComfyUI 対応時は --disable-dynamic-vram 付き。46 §14)
comfyui/                       # git 管理外(生成物)
  ComfyUI/                     # 固定タグの ComfyUI 本体(読み取り専用扱い)
  venv/                        # Python 仮想環境
  data/                        # --base-directory: models/ custom_nodes/ input/ output/ user/
```

`--base-directory` により、モデル・カスタムノード・出力は `comfyui/data/` に置かれ、ComfyUI 本体を汚さない(本体の更新はタグ変更だけで済む)。

## 3. 使い方

| コマンド | 内容 |
|---|---|
| `npm run comfyui:setup` | ComfyUI(固定タグ)と ComfyUI-GGUF(固定コミット)を取得し、venv と依存(PyTorch 含む)を導入 |
| `npm run comfyui:start` | `127.0.0.1:8188` で ComfyUI を起動(`COMFYUI_PORT` で変更可)。アプリの既定接続先と一致 |

起動後、アプリのサイドバーの接続バッジが緑になれば接続成功。

## 4. 前提と OS ごとの扱い

| | macOS (Apple Silicon) | Windows 10/11 (NVIDIA) |
|---|---|---|
| Python | 3.12(`brew install python@3.12`) | 3.12(`py -3.12` で起動できること) |
| 追加要件 | git | git、最新の NVIDIA ドライバ |
| PyTorch | PyPI 版(MPS 対応) | PyTorch 公式 CUDA 版(`versions.env` の `TORCH_CUDA_INDEX`、既定 cu130) |
| 起動時の設定 | `PYTORCH_ENABLE_MPS_FALLBACK=1` | – |

- Python は ComfyUI/PyTorch が十分に対応する 3.12 に固定する(既定の `python3` が新しすぎる場合に備え、バージョンを明示して探す)。
- Windows 版は macOS 上では実行検証できないため、**初回は実機での確認が必要**(CI の Windows ジョブでは構文確認まで)。代替として ComfyUI 公式の Windows ポータブル版を使い、URL を設定してもよい。
- AMD / Intel GPU は対象外(必要なら `TORCH_*` を調整)。

## 5. モデルの配置(手動)

`comfyui/data/models/` 以下に配置する。

| 種類 | 置き場所 | 用途 |
|---|---|---|
| チェックポイント | `checkpoints/` | SD1.5 / SDXL |
| Flux 拡散モデル(GGUF) | `unet/` | Flux.1 GGUF(量子化推奨: Mac 16GB は Q4 前後) |
| テキストエンコーダ | `clip/` | `clip_l`、`t5xxl`(fp8 または GGUF) |
| VAE | `vae/` | Flux 用 `ae.safetensors` など |
| LoRA | `loras/` | (v0.2 以降) |

Flux.1 dev は非商用ライセンス、schnell は Apache 2.0(01 の注意事項を参照)。

開発中の動作確認には SD 1.5(`stable-diffusion-v1-5/stable-diffusion-v1-5` の `v1-5-pruned-emaonly.safetensors`、約 4.27GB、CreativeML OpenRAIL-M、認証不要)を使用した(09「開発中の実機検証ログ」)。配布元が公表する SHA-256(`6ce01616…fa2fa`)と照合してから使うこと。

## 6. 実機スモークテスト

起動中の ComfyUI の実際の応答を、アプリの Rust 側パーサで解釈できるか確認するテスト(通常の `cargo test` では実行されない)。

```
npm run comfyui:start                               # 別ターミナル
cd src-tauri && cargo test -- --ignored real_comfyui --nocapture
```

ComfyUI のバージョンを上げたとき(§7)に、応答形式の変化を検出するために実行する。接続先は環境変数 `COMFYUI_URL` で変更できる。

## 6.1 生成補助機能用のカスタムノード(v0.2、10 参照。承認済み)

顔補正(F-11)と ControlNet(F-12)の開発・検証用に、次を **コミット固定** で `setup` に追加する(実装時に `versions.env` へ値を追記)。

| 項目 | 固定する値(案) | 用途 |
|---|---|---|
| ComfyUI-Impact-Pack | `IMPACT_PACK_COMMIT` | `FaceDetailer` |
| ComfyUI-Impact-Subpack | `IMPACT_SUBPACK_COMMIT` | `UltralyticsDetectorProvider`(検出モデルの読み込み) |
| comfyui_controlnet_aux | `CONTROLNET_AUX_COMMIT` | Depth / OpenPose の前処理 |

- 追加の pip 依存(ultralytics 等)が PyTorch 固定(§1)と衝突しないことを、導入時に確認する(M6 で確認済み: PyTorch は固定どおり)。ただし **ultralytics 本体と、その推移的依存はバージョンを固定していない**(Subpack の `requirements.txt` が無指定のため)。再現性が必要になったら、導入結果を制約ファイルとして固定する。
- M6 で `versions.env` に追加した値: `IMPACT_PACK_COMMIT`(リリースタグが無いため、検証したコミット)、`IMPACT_SUBPACK_COMMIT`(タグ 1.3.4 のコミット)。M7 で `CONTROLNET_AUX_COMMIT`(comfyui_controlnet_aux。リリースタグが無いため検証したコミット)を追加した。controlnet_aux は opencv-contrib-python など多数の依存を入れ、既存の opencv-python と並存する(動作は確認済み)。
- モデルは自動ダウンロードしない(§1 の方針)。置き場所と推奨を案内する。

| 種類 | 置き場所 | 用途 |
|---|---|---|
| 顔検出モデル | `ultralytics/bbox/`(例: `face_yolov8m.pt`) | 顔補正(手は `hand_yolov8s.pt` など) |
| ControlNet モデル | `controlnet/`(SD1.5 用 / SDXL 用はベースモデルに合わせる) | ControlNet |

- 検出モデル(`.pt`)は pickle 形式のため、信頼できる配布元のものだけを使うこと。

### 6.1.1 画像のタグ付け用のカスタムノード(v0.34・承認済み。39 参照)

F-20 のために、**ComfyUI-WD14-Tagger**(`pythongosssss/ComfyUI-WD14-Tagger`)を **コミット固定**で `setup` に追加する(`versions.env` に `WD14_TAGGER_COMMIT`。リリースタグが無い場合は検証したコミット)。`onnxruntime`(CPU 版)はバージョンを固定し、PyTorch 固定(§1)と衝突しないことを導入時に確認する。解析モデルは **ノードが初回に自動取得する**(アプリでは配置しない。39 §8)。

## 6.2 モデルの保存先を変更したとき(v0.6 追記・🟢 承認済み)

アプリの「モデルの保存先」設定(12 §3)は、これまで **アプリのダウンロード先と導入状況の確認にだけ効き、ComfyUI のモデル検索先は変えなかった**。保存先を別の場所にすると、取得したモデルが ComfyUI に見えず生成に失敗する(`models/checkpoints` が無い等)。これを、**画面上の操作で ComfyUI に反映できる**ようにする。

### 仕組み

ComfyUI は起動時に、`main.py` と同じフォルダの `extra_model_paths.yaml` を自動で読み、そこに書かれたフォルダもモデルの検索先に加える(`--base-directory` 側の `models` も引き続き検索される)。アプリはこのファイルを書く。

### 画面(モデルページの「保存先」欄)

- 保存先設定が、ComfyUI の起動引数から検出した `models`(12 §3 の自動検出と同じ方法)と異なる場合、保存先の下に注意を出す。
  > 「この保存先は ComfyUI が参照しているフォルダと異なります。[ComfyUI に反映] を押し、ComfyUI を再起動してください。」
- **[ComfyUI に反映]** ボタン: `extra_model_paths.yaml` を `main.py` と同じフォルダに書く(IPC `models_apply_dir_to_engine`、04 に追記)。成功後、「ComfyUI を再起動すると有効になります」と表示する。アプリは ComfyUI を起動・停止しない(01 の MVP 方針)。
- **[反映を解除]** ボタン(反映済みのときだけ): アプリが書いたファイルを削除する(既定の検索先に戻る)。
- 反映済みかどうかは、ファイルの存在とアプリの印(先頭コメント `# managed by gazooo`)で判定する。アプリ名変更前の旧印 `# managed by gazoo` も「アプリが書いたもの」として扱う(07 Q-52b)。

### 書き込みの安全策

- 対象は **接続先がこの PC の ComfyUI で、`main.py` の場所が特定でき、そのフォルダに書き込める場合だけ**(それ以外はボタンを無効にし、理由を表示。リモート等は 12 §3 と同じ扱い)。
- 既に `extra_model_paths.yaml` があり、先頭に `# managed by gazooo` が **無い** ときは上書きしない(ユーザー自身の設定を壊さない)。エラー `MODELS_EXTRA_PATHS_FOREIGN` で理由を表示し、手動で追記する YAML の例を見せる。
- 内容は `gazooo:` セクションに `base_path: <保存先>` と、12 §4.2 の取得先サブフォルダ(`checkpoints` / `unet` / `clip` / `vae` / `loras` / `controlnet` / `ultralytics` …)の対応。パスは YAML 用に引用符で囲み、Windows の `\` を正しくエスケープする。保存先は絶対パスで、存在するフォルダであること。
- 書き込みは一時ファイル → リネーム。ComfyUI 本体以外のファイルには触れない。

### 受け入れ基準と検証

- (a) 保存先を `D:\models` に変更 → 注意と [ComfyUI に反映] が出る。(b) 反映 → ComfyUI を再起動 → `engine_list_models` に `D:\models\checkpoints` のモデルが現れ、生成できる。(c) [反映を解除] で元に戻る。(d) 既存のユーザー設定ファイルは上書きされない。(e) YAML の生成(Windows パス・空白・引用符を含むパス)・外部ファイル保護・解除は Rust のユニットテストで検証する。
- 実装前に、固定している ComfyUI のタグで、`--base-directory` 指定時も `main.py` 隣の `extra_model_paths.yaml` が読まれることを実機で確認する(読まれない場合は設計を見直して再提示する)。

## 7. 更新手順

1. `versions.env` の値を変更する(ComfyUI タグ、GGUF コミット、PyTorch)。
2. **`npm run engine-manifest` を実行して、アプリ内インストーラのマニフェスト(`manifest.json`)を再生成する(41)。差分(`commit` / `url` / `sha256` / `sizeBytes`)をレビューする。** 手で編集しない。CI の `--check` が不一致を検出する。
3. `npm run comfyui:setup` を再実行(冪等。タグ/コミットを取り直し、依存を再インストール)。
4. アプリと結合して動作確認(接続・モデル一覧・生成)。
5. ComfyUI の API 変更でアダプタに影響が出る場合は 02(R-01)に従い、ワークフローテンプレートを更新する。
