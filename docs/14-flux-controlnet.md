# 14. Flux 系の ControlNet(v0.6)

> ステータス: 🟡 レビュー待ち。承認までは実装しない(AGENTS.md §3)。
> 経緯: Flux を選ぶと ControlNet が無効になる(10 §7「SD 1.5 / SDXL のみ」)。ユーザーの要望で、Flux 系でも ControlNet を使えるようにする。

## 1. 目的とスコープ

**やること(v0.6)**
- Flux 系(`flux` / `flux_gguf`)で、既存の「ControlNet」セクションを使えるようにする(F-12 の拡張)。
- 対応モデルは **Union 型 1 種類**: `Shakker-Labs/FLUX.1-dev-ControlNet-Union-Pro-2.0`(1 つのモデルで Canny・Depth・OpenPose などを切り替える)。
- 1 ユニットまで。前処理は既存と同じ(Canny / Depth / OpenPose / なし)。
- モデル取得ページ(12)に、このモデルを追加する。

**やらないこと(v0.6)**
- XLabs・BFL 公式(Flux.1 Canny / Depth の別モデル・LoRA)など、Union 以外の Flux ControlNet(カスタムノードや別ワークフローが必要なため)。
- 複数ユニット、Flux 系での顔補正・Inpaint(別設計)。
- Flux schnell での動作保証(§5 のリスク)。

## 2. 方針: コアノードだけで組む

実機の ComfyUI 0.45.6 に、必要なノードがコアにあることを確認した(`SetUnionControlNetType` は `auto / openpose / depth / hed・pidi・scribble・ted / canny・lineart・anime_lineart・mlsd / normal / segment / tile / repaint` を持つ。`ControlNetApplyAdvanced` は任意の `vae` 入力を持つ)。追加のカスタムノードは不要。

```
ControlNetLoader(Union モデル) ─ SetUnionControlNetType(type) ─┐
参照画像 ─ ImageScale ─ 前処理 ──────────────────────────────→ ControlNetApplyAdvanced
FluxGuidance(positive) / ConditioningZeroOut(negative) ──────→   (vae 入力に Flux の VAE)
                                         └─ positive / negative → KSampler
```

- SD 系の ControlNet(10 §5)と同じ構造に、(1) `SetUnionControlNetType`、(2) `vae` 入力を加えたもの。
- 前処理の種類から Union の `type` へ対応づける: Canny → `canny/lineart/anime_lineart/mlsd`、Depth → `depth`、OpenPose → `openpose`、なし → `auto`。
- 強さの既定は **0.6〜0.8**(Union Pro 2.0 の推奨。SD 系の既定 1.0 とは異なる)。モデル種別を Flux に替えたとき、未変更なら 0.7 に切り替える。

## 3. 変更点

| 領域 | 内容 |
|---|---|
| `GenerationParams` | 形は変更なし(`controlnet.units[]`)。検証(`validate_controlnet`)の「Flux 系は不可」を外す。**ControlNet モデルが Union 型であること**を、モデル名に `union` を含むかで確認し、含まなければ `VALIDATION`(`controlnet.units[0].model`。Flux 用でないモデルを誤って選ぶのを防ぐ) |
| ワークフロー | Flux のワークフロー(`build_flux`)に ControlNet を追加する。上記の構成。ノード ID は SD 系と同じ割り当てを使う |
| 機能検出 | `EngineFeatures` に `unionControlnet: boolean`(`SetUnionControlNetType` の有無)を追加。無い場合は Flux の ControlNet スイッチを無効にし、ComfyUI の更新を案内する |
| UI | Flux 選択中も ControlNet を使える。モデル一覧は「union」を含むものだけを表示する(Flux 時)。SD 系のモデル(`control_v11…` 等)は Flux 時に出さない。逆に SD 系選択中は Union 型を出さない |
| モデル取得(12) | カタログに ControlNet「Flux Union Pro 2.0」を追加(§4) |
| AI 機能カタログ(13 §4) | 「Flux でも ControlNet を使える(Union 型のみ)」へ更新。「Flux の Inpaint・顔補正・ControlNet」の記述から ControlNet を外す |
| 既存の記述 | 10 §7、05、09 の「Flux 系では使えない」を更新する |

## 4. モデル取得カタログ(12 に追加する内容)

| 項目 | 内容 |
|---|---|
| ファミリー | `controlnet-flux-union` |
| バリアント | Union Pro 2.0(fp16 相当。約 4GB)/ コミュニティの fp8 版(VRAM が足りない場合。fp8 は公式ではないため注記する) |
| 置き場所 | `models/controlnet/` |
| ライセンス | FLUX.1 [dev] 系の **非商用ライセンス**(`commercialUse: false`、同意ダイアログあり。12 §6 の `requiresConsent`) |
| 注記 | 「FLUX.1 dev を前提に学習されています。schnell では効果が弱い・不安定な場合があります」 |
| ハッシュ・URL | 実装時に、リビジョンを固定した URL と SHA-256 を実際のファイルから取得して記載する(12 §4 の方針。推測で書かない) |

## 5. リスク・要検証

- **VRAM**: Flux GGUF(Q4)に ControlNet(約 4GB)を足すと、12GB の GPU でも厳しい可能性がある。実機(RTX 2060 12GB)で、解像度 768 / 1024 と fp16 / fp8 の可否を計測し、結果を 09 に記録する。足りない場合の案内(fp8 版・解像度を下げる)を UI に出す。
- **schnell との相性**: Union Pro 2.0 は dev 向け。schnell(蒸留・4 step)での動作は未確認。実機で試し、効かない場合は schnell 選択時に注意文を出す(禁止はしない)。
- **GGUF 版 Flux との組み合わせ**: dev GGUF + Union の実績はある(ワークフロー例あり)が、本アプリのノード構成での実機確認が必要。
- **ノードの挙動差**: `ControlNetApplyAdvanced` の `vae` 入力が Flux の Union で必要かは、実機で確認する(不要な場合は付けない)。
- **前処理**: Depth / OpenPose は controlnet_aux が必要(既存と同じ。10 §3)。

## 6. テスト方針

- Rust: Flux の ControlNet ワークフロー(ノード構成・`type` の対応・`vae` 入力・リンクの整合)、検証(Union 以外のモデルの拒否・Flux でのその他の補助機能の拒否が維持されること)、機能検出。ComfyUI 実機のテストは `#[ignore]`(Canny 参照で生成し、ControlNet の有無で結果が変わること。既存の SD 系テストと同形)。
- フロント(Vitest): モデル一覧の絞り込み(Flux 時は union のみ / SD 系時は union を除外)、種別切替時の強さの既定、スイッチの有効/無効条件。
- 手動(09 に追加): Flux で Canny 参照の生成、VRAM・時間の記録、schnell の挙動、モデル取得ページからの取得。

## 7. 実装の分割(マイルストーン M11)

1. 実機で、`ControlNetLoader` → `SetUnionControlNetType` → `ControlNetApplyAdvanced` の Flux ワークフローを試し、構成(`vae` の要否・強さ)と VRAM を確定する(**設計の確認を兼ねる。結果は本書へ反映**)。
2. ワークフロー・検証・機能検出(Rust)。
3. UI(モデル絞り込み・強さの既定)・AI 機能カタログの更新。
4. モデル取得カタログへの追加(実ファイルからハッシュ取得)。
5. 09 のチェックリストと検証ログ。

## 8. 未決事項(07 に追記)

- **Q-42**: 対応モデルを Union Pro 2.0 の 1 種類に絞ってよいか。**推奨: 絞る**(カスタムノード不要・1 モデルで複数の前処理に対応。XLabs・BFL 公式は需要があれば別設計)。
- **Q-43**: fp8 のコミュニティ版を取得ページに載せてよいか。**推奨: 載せる**(公式でない旨を注記。VRAM が厳しい環境向け。信頼できる配布元とハッシュ固定が前提)。
- **Q-44**: schnell で効果が弱い場合の扱い。**推奨: 禁止せず注意文のみ**。
