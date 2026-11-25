# 17. 新モデル種別: Anima / Qwen-Image 2.1 / Krea 2(v0.9)

> ステータス: 🟢 承認済み(2026-11-19。Q-68〜73 は推奨どおり)。§4・§5 は、承認どおり「公式テンプレートで値を確定してから実装」(Q-71)した結果を反映した(2026-11-19)。承認時の案からの差異は §10 にまとめた。
> **経緯**: ユーザーが「NoobAI-XL・Animagine XL 4.0・Anima・Qwen-Image 2.1・Krea 2 に対応してほしい」と指定(2026-11-19)。前 2 つは既存の `sdxl` 種別で動くため 16(M12)で扱い、**後 3 つは新しいモデル種別として本資料(M13)で扱う**。個人利用のため、ライセンスの商用可否は選定条件にしない(取得時はライセンス名とリンクを表示するのみ)。

## 1. 目的とスコープ

Anima・Qwen-Image 2.1・Krea 2 を、生成画面で選んで txt2img できるようにし、取得ページから必要なファイルを取得できるようにする。

**やること**: 3 種別の追加(`anima` / `qwen_image` / `krea2`)、ワークフロー生成、必要ファイルの取得(カタログ)、生成フォームの出し分け、推奨パラメータ、モデルの説明(18)。
**やらないこと(v0.9)**: 画像編集(Qwen-Image 2.1 の参照画像入力は **v0.16 の [24](24-qwen-reference-image.md) で別途設計**。Krea 2 の LoRA による参照は対象外のまま)、顔補正・ControlNet・Inpaint(いずれも `sd15` / `sdxl` のみのまま。10・13)、LoRA、Turbo 系の LoRA、プロンプトの自動書き換え(Qwen-Image 2.1 の `TextGenerate`・Krea 2 のプロンプト拡張)、Civitai からの取得(07 Q-65)。

## 2. 調査結果(2026-11-19。ComfyUI 0.45.6 同梱の公式テンプレートと、Hugging Face の API による)

3 種別とも、**ComfyUI 0.45.6 のコアノードだけ** で動く(カスタムノード不要)。同梱の公式テンプレート(`image_anima_base_v1.json`・`image_qwen_image_2_1_t2i.json`・`image_krea2_turbo_t2i.json`)のノード構成・値を、そのまま採用する。取得元(Hugging Face)はいずれも **認証不要(gated でない)**。

| 種別 | 拡散モデル(`UNETLoader`) | テキストエンコーダ(`CLIPLoader` の `type`) | VAE(`VAELoader`) |
|---|---|---|---|
| **Anima**(2B。NVIDIA Cosmos-Predict2-2B 派生) | `anima-base-v1.0.safetensors` 4.18GB | `qwen_3_06b_base.safetensors` 1.19GB(`stable_diffusion`) | `qwen_image_vae.safetensors` 254MB |
| **Qwen-Image 2.1**(7B 級) | `qwen_image_2.1_int8_convrot.safetensors` 7.26GB(推奨)/ `qwen_image_2.1_bf16.safetensors` 14.2GB | `qwen3vl_8b_int8_convrot.safetensors` 9.35GB(推奨)/ `qwen3vl_8b_bf16.safetensors` 17.5GB(`qwen_image`) | `qwen_image_2.1_vae_bf16.safetensors` 676MB(2.1 専用) |
| **Krea 2**(12.9B。Turbo) | `krea2_turbo_fp8_scaled.safetensors` 13.1GB | `qwen3vl_4b_fp8_scaled.safetensors` 5.24GB(`krea2`) | `qwen_image_vae.safetensors` 254MB |

- 取得元: Anima = [circlestone-labs/Anima](https://huggingface.co/circlestone-labs/Anima)(`split_files/`)、Qwen-Image 2.1 = [Comfy-Org/Qwen-Image-2.1](https://huggingface.co/Comfy-Org/Qwen-Image-2.1)、Krea 2 = [Comfy-Org/Krea-2](https://huggingface.co/Comfy-Org/Krea-2)。ファイルごとの SHA-256・サイズは、実装時にカタログへ(リビジョン固定の URL で)記入し、テストで固定する(12 §4.1)。
- Anima と Krea 2 の `qwen_image_vae.safetensors` は **同一のファイル**(SHA-256 `a70580f0…3d1f`、253,806,246 バイト)。
- 配置先は既存の `unet/`(拡散モデル)・`clip/`(テキストエンコーダ)・`vae/` を使う(ComfyUI が `diffusion_models` / `text_encoders` と同じ場所として読む。`folder_paths.py` で確認)。`DestDir` の追加は不要。
- **必要メモリ**: Anima は軽い(合計 5.6GB)。Qwen-Image 2.1 は INT8 で本体とエンコーダ合わせて約 17GB、Krea 2 は FP8 で約 18.6GB。開発機(RTX 2060 12GB / メモリ 48GB)で動くかは、実機で確認する(§8)。

## 3. データモデル・IPC(03・04 への変更)

- `ModelKind` を `"sd15" | "sdxl" | "flux" | "flux_gguf" | "anima" | "qwen_image" | "krea2"` に拡張する(04 §型、03 `model.kind`)。`params_json` の `schemaVersion` は据え置き(任意項目の追加のみ。マイグレーション不要)。
- 3 種別とも `model.name` = 拡散モデル、`model.clip` = **テキストエンコーダ 1 つ**(`["…"]`)、`model.vae` = VAE(必須)。Flux は `clip` が 2 つ(T5 と CLIP-L)のままで、3 種別は **1 つ** にする。
- ネガティブプロンプト・CFG: **Anima** はネガティブを使う(CFG あり、既定 4)。**Qwen-Image 2.1** は公式の既定が CFG 1(ネガティブは空)だが、ノードがネガティブを受けるので使えるようにする(CFG > 1 のとき効く)。**Krea 2** はネガティブを使わない(`ConditioningZeroOut`)。`guidance` は 3 種別とも使わない。
- `faceDetailer` / `controlnet` / `inpaint` は 3 種別で指定されたら `VALIDATION`(10 §7 の条件に追加)。
- エンジンの一覧(`EngineModels`)は、拡散モデルを `unets`(`UNETLoader`)、テキストエンコーダを `clips`(`CLIPLoader`)、VAE を `vaes` から選ぶ(新しい項目は不要)。

## 4. ワークフロー生成(`engine/comfyui/workflow/`)— 公式テンプレートの値

`workflow.rs` に種別ごとのビルダーを追加する(Flux と同じ、Rust コードで組み立てる)。共通の骨格は `UNETLoader`(`weight_dtype` ではなく `["…", "default"]`)→ `CLIPLoader` → `VAELoader` → `EmptyLatentImage` → `KSampler` → `VAEDecode` → `SaveImage`。

| 種別 | 構成(公式テンプレートと同じ) | 既定値 |
|---|---|---|
| **Anima** | `UNETLoader`(weight_dtype `default`)、`CLIPLoader`(`type`=`stable_diffusion`、`device`=`default`)、`VAELoader`。`CLIPTextEncode` ×2(正・負)、`EmptyLatentImage`(SD3 用ではない)、`KSampler`(model は直結。`ModelSampling*` ノードは不要) | 1024×1024、steps 30、CFG 4、`euler` / `simple`、ネガティブ `worst quality, low quality, score_1, score_2, score_3, blurry, jpeg artifacts, sepia` |
| **Qwen-Image 2.1** | `UNETLoader`(`default`)、`CLIPLoader`(`type`=`qwen_image`)、`VAELoader`、**`TextEncodeQwenImage21`**(`clip`・`prompt`・`negative_prompt`、`resolution` 1024。出力は positive / negative)、`EmptyLatentImage`、`KSampler` | 1024×1024、steps 25、CFG 1、`euler` / `simple` |
| **Krea 2(Turbo)** | `UNETLoader`(`default`)、`CLIPLoader`(`type`=`krea2`)、`VAELoader`、`CLIPTextEncode`(正)、**`ConditioningZeroOut`**(負)、`EmptyLatentImage`、`KSampler` | 1024×1024、steps 8、CFG 1、`euler` / `simple` |

- 実機のバリデータで構造を検証する(09)。ノードの有無は `/object_info` で検出し、無ければ **ComfyUI の更新を案内**(`ENGINE_NODE_MISSING`)。Qwen-Image 2.1 は `TextEncodeQwenImage21`、Krea 2 / Anima は `CLIPLoader` の `type` の選択肢に `krea2` / `qwen_image` / `stable_diffusion` があること(= 新しい ComfyUI)を確認対象にする。
- 公式テンプレートにある `QwenImage21Cache`(KV キャッシュの設定。編集用)・`TextGenerate`(プロンプトの書き換え)・`LoraLoaderModelOnly`(Turbo / スタイル LoRA)・`ComfySwitchNode` は、t2i の基本には不要なので **使わない**。

## 5. カタログ(12 §4.2 に追加)

- `ModelCategory` に `"anima" | "qwen_image" | "krea2"` を追加(取得ページに 3 つのタブ)。`ModelFamily` に任意の **`kind`**(`"flux_gguf" | "anima" | "qwen_image" | "krea2"`)を追加する。`kind` を持つファミリー(`unet/` に置く拡散モデル本体)が、生成画面で選べるモデルで、`profile` が必須(18)。以前の「`unet/` に置くもの = Flux」という推定は、この `kind` に置き換える(既存の Flux GGUF 2 ファミリーにも `kind: "flux_gguf"` を付ける)。
- **まとめて取得**(Q-70): `ModelFamily` に任意の **`requires`**(`[{ familyId, variantId }]`)を追加し、拡散モデルのファミリーから、テキストエンコーダ・VAE を必要なものとして列挙する。取得ページのカードに [必要なファイルをまとめて取得] を出し、導入済みのものは飛ばして順に取得する(同意が要るものは、その都度ダイアログ)。
- **同じファイルの共有**: Anima と Krea 2 の `qwen_image_vae.safetensors` のように、**配置先とファイル名が同じで SHA-256 も同じ** variant は、複数のファミリーに置ける(ハッシュが違う重複は従来どおり拒否する)。

| ファミリー(ID) | variant |
|---|---|
| Anima(`anima-base`)+ テキストエンコーダ(`anima-text-encoder`)+ VAE(`anima-vae`) | `anima-base-v1.0`(推奨)/ `qwen_3_06b_base` / `qwen_image_vae` |
| Qwen-Image 2.1(`qwen-image-2-1`)+ エンコーダ(`qwen-image-2-1-text-encoder`)+ VAE(`qwen-image-2-1-vae`) | 本体: INT8 convrot(推奨)/ BF16。エンコーダ: INT8 convrot(推奨)/ BF16。VAE: bf16 |
| Krea 2 Turbo(`krea2-turbo`)+ エンコーダ(`krea2-text-encoder`)+ VAE(`krea2-vae`) | 本体: FP8 scaled(推奨)。エンコーダ: FP8 scaled。VAE |

- (2026-11-24: `nsfw` タグは 18 §10 で廃止。以下は当時の記録)タグ: Anima に `anime` と `nsfw`(記事の評価と、配布元の README で **確認できたものだけ**。16 §3.3)。Qwen-Image 2.1 は記事で「NSFW を出せる」とされるが、公式の記載で確認できなければ `nsfw` を付けない。Krea 2 は付けない。
- 同意ダイアログ: 拡散モデル本体の variant だけ `requiresConsent`(ライセンス名とリンクを表示。商用可否の注意書きはしない)。

## 6. UI(05)

- 生成画面のモデル種別に **Anima / Qwen-Image 2.1 / Krea 2** を追加する。種別を選ぶと、拡散モデル・テキストエンコーダ 1 つ・VAE を選ぶ欄と、推奨値(§4 の既定)が入る。
- 必要ファイルが足りないとき、既存の導線(「モデル取得ページを開く」)で該当タブへ遷移する(`/models?category=anima` など)。
- 顔補正・ControlNet・Inpaint のセクションは、3 種別では **無効化し「SD1.5 / SDXL のみ」と表示**。ネガティブの欄は Krea 2 で非表示。

## 7. AI サポート(11・13 §6・18)

モデルガイド(18 §6.1)は、カタログのプロフィールから自動生成されるので、3 種別の `profile` を追加すれば反映される。対応機能の記述(Flux 系のみ「使えない」)は、種別ごとに計算する形に直す。

## 8. テスト・検証

- Rust: 各種別のワークフロー JSON のリンク整合性(ノード間の参照)と、公式テンプレートの値との一致(`type`・既定値)をユニットテストで検証。種別ごとの `validate`(`clip` が 1 つ、VAE 必須、未対応機能の拒否)。カタログのスキーマ・`requires` の参照整合・ハッシュ・同一ファイル共有の規則。
- フロント(Vitest): 種別の切り替えでフォームの項目・既定値が変わること、まとめて取得の対象の計算。
- 実機(2026-11-19 に確認): **Anima は 57 秒、Krea 2 Turbo(FP8)は 59 秒、Qwen-Image 2.1(INT8)は 104 秒で、開発機(RTX 2060 12GB)で生成できた**(09 の検証ログ)。以下は検証の方針(当初の想定)。**Anima は開発機で生成まで検証する(軽量)**。Qwen-Image 2.1 と Krea 2 は、メモリ(VRAM 12GB)や INT8 / FP8 の対応で生成できない可能性があるため、**構造がバリデータに受理される(`/prompt` が通る)ところまで** を最低限とし、生成できたかを 09 に記録する。

**マイルストーン M13**(実装順): ① 共通基盤(`ModelKind` 拡張・`kind` / `requires`・カタログのカテゴリ・UI の種別追加)+ Anima、② Qwen-Image 2.1、③ Krea 2。各ステップで、ワークフローを実機のバリデータで確認してから次へ進む。

## 9. 未決事項(07 B8 を参照)

| ID | 論点 | 推奨 |
|---|---|---|
| Q-68 | 対象 3 種別と順序 | **Anima → Qwen-Image 2.1 → Krea 2**(承認済み) |
| Q-69 | 3 種別の機能範囲 | **txt2img のみ**(承認済み) |
| Q-70 | 必要ファイルのまとめ取得 | **ファミリー単位でまとめて取得**(承認済み。§5 の `requires`) |
| Q-71 | ワークフローの確定方法 | **公式テンプレートで確定してから実装**(承認済み。§4 に反映) |
| Q-72 | Krea 2 の GGUF・量子化の第三者再配布 | **含めない**(承認済み。公式の Comfy-Org 配布のみ) |
| Q-73 | メモリ不足で実機検証できない種別 | **構造検証まで行い、09 に未検証と明記**(承認済み) |

## 10. 承認時の案からの差異(§4・§5 の確定で変わった点)

1. **Krea 2 は Turbo のみ**(Raw は除く)。公式テンプレートがあるのは Turbo で、Raw の推奨値(ステップ・CFG)は第三者の記述のみで確認できないため。Raw は、値を確認できてから追加する。
2. **精度**: Krea 2 は FP8 scaled のみ(BF16 は 26GB で現実的でない)。Qwen-Image 2.1 は INT8 convrot(推奨)と BF16。
3. **テキストエンコーダは 1 つ**(Flux のような 2 つではない)。
4. 配置先は新設せず、既存の `unet/` `clip/` `vae/` を使う。
5. カタログに `kind` と `requires` を追加(上記)。`kind` は、従来の「`unet/` = Flux」という推定の置き換え。
6. **Anima は Base のみ**(配布元は Turbo・Aesthetic も出し、README は「Turbo から始める」を勧めるが、Turbo は CFG 1・8〜12 ステップと設定が違う)。公式テンプレートがあるのは Base なので、まず Base を入れる。Turbo は、推奨値を別ファミリーとして持たせる形で、あとから追加できる。
7. **Qwen-Image 2.1 は標準版のみ**(Turbo 版・プロンプトの書き換え用エンコーダは含めない)。
