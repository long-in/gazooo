# 23. ComfyUI のアプリ内インストール(v0.15)

> ステータス: 🟢 承認済み(2026-11-20。07 Q-108〜115 は推奨どおり。「同梱ではなく、アプリが取得して導入する」方式で確定)・実装済み(§13)。08(開発用セットアップ)の手順を、**アプリ自身が実行できる形**にする。22(既定パスの保持)の Q-104「本体の導入は別設計」にあたる。

## 1. 目的とスコープ

インストーラーで入れたアプリを初めて起動した利用者が、**ComfyUI を自分で探して入れなくても**、アプリの画面から [インストール] を押すだけで生成できる状態になる。

- ComfyUI 本体・Python・PyTorch・必要なカスタムノードを、**アプリ専用のフォルダ 1 つの中にすべて**置く(システムの Python・git・PATH・レジストリには触れない)。
- 導入後は、15 の起動管理(管理モード)に **パスが設定済みで有効** の状態になる。利用者はパスを一切入力しない。
- 対象: Windows 10/11 x64(NVIDIA GPU)、macOS(Apple Silicon)。AMD / Intel GPU・Intel Mac・Linux は対象外(§4.1 の事前チェックで案内して止める)。
- 既に ComfyUI を持つ利用者は、従来どおり外部接続(`external`)か、22 の候補検出を使う。**アプリの導入は強制しない**。

スコープ外: モデル(数 GB)の同梱(取得は 12 の取得ページ)、ComfyUI の自動更新(§8 は利用者の操作のみ)、GPU ドライバの導入。

## 2. インストール先(§11 Q-108)

```
<app_data>/comfyui/                  # Tauri の app_data_dir 配下(Windows: %APPDATA%\dev.gazooo.desktop\comfyui)
  .gazooo-install.json               # 導入状態(マニフェストの版・導入済みコンポーネント・日時)
  python/                            # 単体配布の Python(python-build-standalone)
  venv/                              # 上記 Python から作る仮想環境(08 のレイアウトと同じ)
  ComfyUI/                           # 固定タグの ComfyUI 本体(読み取り専用扱い)
  data/                              # --base-directory: models/ custom_nodes/ input/ output/ user/
  downloads/                         # 取得中・取得済みアーカイブ(導入完了後に削除)
```

- 08 の `comfyui/{ComfyUI,venv,data}` と **同じ並び** にするので、15 §3.1 / 22 の候補検出・パス組み立てがそのまま使える。
- **アプリのインストール先(Program Files / `.app`)ではなく `app_data_dir` に置く**。インストール先はアプリの更新・アンインストールで消え、macOS の `.app` は書き込めないため。利用者には「アプリのデータフォルダ内」と表示し、[フォルダを開く] を置く。
- モデルの既定の保存先(12 §3)は、導入後は `<root>/data/models` になる(設定で変更した場合はその値を優先。06 §6.2 の `extra_model_paths.yaml` の仕組みも従来どおり)。
- すべてのパスは Rust の `std::path` / Tauri の path API で組み立てる(06)。**フォルダ名に空白・日本語を含むユーザー名でも動く**ことをテストする(`%APPDATA%` 配下に日本語のユーザー名が入る場合がある)。

## 3. 何を導入するか(固定マニフェスト)

バージョンは **アプリに内蔵するマニフェスト**(`src-tauri/src/services/engine_install/manifest.json`)に固定する。値は 08 の `tools/comfyui/versions.env` と同じ(ComfyUI タグ・各ノードのコミット・PyTorch)で、**二重管理を避けるため、`manifest.json` は `tools/gen-engine-manifest.mjs`(41)で `versions.env` から生成し、CI で `--check` する**(不一致なら CI が落ちる)。Rust 側の一致テストも安全網として残す。更新は依存更新と同様に単独の変更で行う(AGENTS.md §2)。

| 部品 | 取得元(許可ホスト) | 固定の仕方 | 検証 |
|---|---|---|---|
| Python 3.12(単体配布) | `github.com/astral-sh/python-build-standalone` の Release | リリースタグ+ファイル名 | **SHA-256 をマニフェストに記載し照合** |
| ComfyUI 本体 | `github.com/comfyanonymous/ComfyUI` のタグ `v0.45.6` のソース zip | タグ(`COMFYUI_TAG`) | SHA-256 を記載し照合 |
| PyTorch 一式 | Windows: `download.pytorch.org/whl/cu130`、macOS: `pypi.org` | `==` で完全固定(`TORCH_VERSION` 等) | pip の `--require-hashes` は使わず、固定バージョン+公式インデックスのみ(§9 リスク) |
| ComfyUI 本体の依存 | `pypi.org` / `files.pythonhosted.org` | ComfyUI の `requirements.txt`+アプリ内蔵の制約ファイル(`constraints.txt`) | 同上 |
| ComfyUI-GGUF | `github.com/city96/ComfyUI-GGUF` | コミット(`GGUF_NODE_COMMIT`)の zip | SHA-256 を記載し照合 |
| Impact Pack / Subpack | `github.com/ltdrdata/...` | コミットの zip | 同上 |
| controlnet_aux | `github.com/Fannovel16/comfyui_controlnet_aux` | コミットの zip | 同上 |
| ComfyUI-WD14-Tagger(v0.34・レビュー待ち。39) | `github.com/pythongosssss/ComfyUI-WD14-Tagger` | コミット(`WD14_TAGGER_COMMIT`)の zip + `onnxruntime`(CPU 版)の固定 | SHA-256 を記載し照合 |

- git・システムの Python は **不要**(ソースは zip で取得して展開する。08 のスクリプトが使う `git` を使わない)。
- 展開は Rust 側(`zip` / `tar` + 圧縮解除のクレート。採用時に固定バージョン・ライセンス・メンテ状況を確認して 02 を更新)。**パストラバーサル(`..`・絶対パス・シンボリックリンク)を含むアーカイブは拒否**する。
- 追加の pip 依存の衝突は、08 §6.1 で確認済みの組み合わせを制約ファイルに固定して再現する。

### 3.1 コンポーネントの選択(§11 Q-109)

| コンポーネント | 内容 | 既定 |
|---|---|---|
| **基本**(必須) | Python・venv・ComfyUI・PyTorch。SD 1.5 / SDXL / Anima / Qwen-Image / Krea 2 | 必須 |
| Flux(GGUF) | ComfyUI-GGUF | オン |
| 顔補正 | Impact Pack・Impact Subpack(ultralytics を含む) | オン |
| ControlNet の前処理 | controlnet_aux(Depth / OpenPose) | オン |
| 画像のタグ付け(v0.34・レビュー待ち。39) | ComfyUI-WD14-Tagger(onnxruntime を含む)。解析モデルは初回の解析時にノードが取得 | オン |

選択は導入後に [追加で導入] / [削除] で変更できる(§8)。各項目に、ダウンロードサイズとディスク使用量の目安を表示する。

## 4. 導入の流れ(Rust の `EngineInstaller`)

状態: `NotInstalled → Preflight → Installing(step) → Verifying → Installed`、失敗は `Failed(step, 理由)`、利用者の操作で `Cancelled`。同時に実行できるのは 1 つだけ。**実行中のジョブがある・エンジンが管理モードで起動中のときは開始できない**(`ENGINE_BUSY`。15 と同じ)。

### 4.1 事前チェック(Preflight。何もダウンロードしない)

- OS・CPU: Windows x64 / macOS arm64 以外は不可(理由を表示)。
- 空き容量: **基本だけで 12GB 以上**(選択に応じて加算。一覧に必要量を表示)。足りなければ開始しない。
- Windows: NVIDIA GPU の存在(`nvidia-smi` の存在 / `where`)。**無ければ「GPU が見つかりません。CPU では実用的な速度が出ません」と警告し、続行は利用者の確認が必要**(CPU 版は導入しない)。ドライバが古い場合の案内(cu130 は新しいドライバが必要)を出す。
- 書き込み: インストール先に書き込めるか。既に導入済み(`.gazooo-install.json` あり)なら「修復 / 更新 / 削除」を出す。
- 他の ComfyUI がポート 8188 を使用中でも導入はできる(起動時に 15 のポート検査が働く)。

### 4.2 手順(各手順は冪等。完了した手順は印(`.gazooo-install.json`)を残して再実行時に飛ばす)

1. `python/` に Python を取得・展開(SHA-256 照合)。
2. `venv/` を作成(`python -m venv`)。
3. `ComfyUI/` に本体を取得・展開。
4. PyTorch を導入(`pip install torch==… torchvision==… torchaudio==… --index-url <固定のインデックス>`)。
5. ComfyUI の依存を導入(`pip install -r requirements.txt -c constraints.txt`)。
6. 選択したカスタムノードを `data/custom_nodes/` に取得・展開し、その依存を導入。
7. `data/` の骨組み(`models/*` 等)を作る。
8. **検証(Verifying)**: 管理モード(15)で ComfyUI を一時起動し、`/system_stats` が応答し、`/object_info` に期待するノード(選択したコンポーネント分。10 §3 の検出と同じ)があることを確認して停止する。失敗なら `Failed(Verifying)` とログを表示。
9. 成功したら、`ManagedEngineConfig` を **自動で設定**(`enabled: true`、`mainPy` / `python` / `baseDirectory` は導入先、ポート 8188、`autoStart: false`)。モードを管理にし、画面に [起動] を出す(§11 Q-110。15 Q-45「勝手に有効にしない」は、**利用者が導入を明示的に押した場合は適用しない**)。

- pip は **`python -m pip` を Rust から子プロセスとして起動**する(`command-group` で後始末。15 Q-50)。引数は固定の形で組み立て、任意の追加引数・環境変数は受け付けない。環境変数は `PIP_DISABLE_PIP_VERSION_CHECK=1`、`PYTHONNOUSERSITE=1`(利用者のサイトパッケージを読ませない)、`PIP_REQUIRE_VIRTUALENV=1` を固定して付ける。
- 進捗は、ダウンロードはバイト数、pip は出力行(最新 500 行を保持、ANSI 除去。15 と同じ)から表示する。手順ごとの進行は「手順 4/8」のように示す。
- **キャンセル**: ダウンロードは `.part` を残して中断(再開可。12 §6)、pip は子プロセスを終了する。中途半端な手順は次回再実行で再開(冪等)。
- **失敗時**: ログ全文のコピー、[再試行](失敗した手順から)、[削除してやり直す]。ネットワーク断・容量不足・ハッシュ不一致・pip 失敗を区別して理由を表示する(`ENGINE_INSTALL_*`、04 に追記)。

## 5. 通信とプライバシー(§11 Q-111)

01 の「通信先は設定されたエンジンのみ」に対する **3 つ目の例外**(AI サポート 11・モデル取得 12 に続く)。

- **利用者が [インストール] を押し、同意ダイアログで承認したときだけ**通信する。起動時・バックグラウンド・自動更新では通信しない。
- 同意ダイアログに、**接続するホストの一覧・合計ダウンロードサイズ・ディスク使用量・導入されるソフトのライセンス**を表示する。
- 許可ホスト: `github.com`、`codeload.github.com`、`objects.githubusercontent.com`(リリース・アーカイブのリダイレクト先)、`download.pytorch.org`(Windows のみ)、`pypi.org`、`files.pythonhosted.org`。それ以外へのリダイレクトは中止。https のみ。リダイレクトは最大 5 回(12 §6 と同じ基盤)。
- ユーザー情報・プロンプト・画像・API キーは送らない。User-Agent は `gazooo` +バージョンのみ(12 §10)。pip には `--index-url` で許可インデックスのみを渡し、追加の `--extra-index-url` は使わない。
- ダウンロード基盤は **12 §6 の実装(`.part`・Range 再開・SHA-256 照合・許可リスト)を再利用**する。新しい独自の取得処理は作らない。

## 6. ライセンスと同意(§11 Q-112)

- ComfyUI 本体と Impact Pack は GPL-3.0、ultralytics(Impact Subpack が使う)は AGPL-3.0、PyTorch は BSD 系、ほか各依存はそれぞれのライセンス。**アプリは再配布せず、利用者の PC 上で利用者の操作によって取得・導入し、別プロセスとして起動する**(アプリ本体に組み込まない)。
- 同意ダイアログに「導入されるソフトウェアと、それぞれのライセンス」の一覧(名前・バージョン・ライセンス名・配布元)を出し、承認を必須にする。承認日時は `.gazooo-install.json` に記録する。
- 顔補正(ultralytics / AGPL)は選択式にして、利用者が外せるようにする(§3.1)。

## 7. UI

- **設定 > ComfyUI の起動管理**(15 §8)の先頭に「ComfyUI のインストール」セクションを追加:
  - 未導入: 説明と [インストール…]。既存の ComfyUI を使う場合は [既存の ComfyUI を使う](従来の画面へ)。
  - 導入済み: バージョン・導入先・コンポーネント・使用量、[追加で導入] [修復] [更新] [フォルダを開く] [削除]。
- **インストールウィザード**(ダイアログ): ① 事前チェック結果 → ② コンポーネント選択(サイズ表示)→ ③ 同意(ホスト・ライセンス)→ ④ 進捗(手順・バー・ログ・[キャンセル])→ ⑤ 完了([ComfyUI を起動] / [モデルを取得])。
- **初回案内**: エンジンが未接続で、管理モードも未設定のとき、生成ページと接続バッジのツールチップに「ComfyUI がありません。[インストール]」を出す(ダイアログは自動で開かない)。
- 完了後の導線: 「次はモデルを取得」(12 の取得ページへ)。

## 8. 更新・修復・削除

- **追加で導入**(v0.34 で実装): 導入済みの環境に、記録(`.gazooo-install.json` の `components`)に無い部品があるとき、設定 > エンジン > 「ComfyUI のインストール」に **[追加で導入]** が出る(足りない部品の名前も表示)。押すと同じウィザードが「部品を追加で導入」として開き、**導入済みの部品はオンのまま変更できず(「導入済み」と表示)**、足りない部品だけを選べる。ダウンロード量・容量の合計と同意画面(ライセンス)は、**新しく入れる部品だけ**を表示する(基本は出さない)。新しい部品が 1 つも選ばれていないと [次へ] は押せない。バックエンドは通常の導入と同じ手順で、完了済みの手順は印で飛ばすため、追加した部品だけが取得・導入される。ComfyUI の起動中・ジョブ実行中は `ENGINE_BUSY`。
- **修復**: 手順 1〜9 を、印の有無に関わらず検証から再実行(壊れたファイルは再取得)。
- **更新**: アプリの更新でマニフェストのバージョンが変わったとき、導入済みの版と比較して設定画面に「更新があります」と出す(**自動では更新しない**)。更新は、変わった部品だけ差し替えて 8(検証)まで実行。`data/`(モデル・出力・ユーザー設定)は触らない。
- **削除**: エンジン停止後(ジョブ実行中は不可)、確認ダイアログで **「モデルと生成物(`data/`)も削除するか」を選ばせる**(既定は残す)。残す場合は `data/` を `comfyui/` 以外へ移さず、その場に残す(再導入で再利用できる)。管理モードは `external` に戻す。
- **アンインストール**: アプリのアンインストーラーは `app_data_dir` を消さないため、数 GB が残る。アンインストール前に設定画面の [削除] を使うよう、09 と README に明記する(NSIS のフック等での自動削除は、誤削除の危険があるため行わない)。

## 9. リスク

- **サプライチェーン**: pip と GitHub のソースから第三者のコードを取得して実行する。→ 取得元の許可リスト、固定バージョン・コミット、zip の SHA-256 照合、https のみ。pip の依存は完全なハッシュ固定をしていない(08 §6.1 と同じ弱点)。**制約ファイルを検証済みの組み合わせで固定**し、将来 `--require-hashes` に進める余地を残す。
- GitHub のソース zip は、**ハッシュが将来変わる可能性がある**(GitHub は固定を保証しない)。→ 不一致のときは自動で続行せず `ENGINE_INSTALL_HASH_MISMATCH` で止め、マニフェスト更新(単独の変更)で対応する。これを防ぐため、可能な部品はリリースアセット(ハッシュが固定される)を優先する。
- **巨大な導入(Windows で 6〜10GB)・長時間**。→ 空き容量チェック、手順ごとの再開、キャンセル、進捗表示。途中でアプリを閉じても、次回再実行で続きから。
- **Windows の長いパス・ウイルス対策ソフトの干渉**(大量の小さいファイルの展開)。→ 展開先を短いパス(`app_data_dir/comfyui`)に保つ。失敗時はログに原因を残し、再試行で復旧できるようにする。実機で要確認。
- **CUDA / ドライバ**: cu130 は新しい NVIDIA ドライバを要する。→ 事前チェックで案内。古い場合に別の CUDA 版へ切り替える選択肢は後続(Q-113)。
- Python の単体配布(python-build-standalone)の venv が、Windows で想定どおり動かない可能性。→ 実装前に実機で確認する(動かなければ設計を見直して再提示する)。
- macOS の Gatekeeper / 署名: 単体配布の Python 実行ファイルが隔離属性で止まる場合がある。→ アプリがダウンロードしたファイルには隔離属性が付かないが、実機で確認する(09)。
- ComfyUI のバージョンを上げたときの互換(08 §7 と同じ)。マニフェスト更新と実機スモークテストで確認する。

## 10. テスト

- Rust(ネットワークに出ない。ローカルのモックサーバと一時ディレクトリを使用): マニフェストのスキーマ・許可ホスト・固定値の検証、`versions.env` との一致、アーカイブ展開の安全性(`..`・絶対パス・シンボリックリンクの拒否)、手順の冪等・再開、キャンセル、ハッシュ不一致、容量不足の事前チェック、`.gazooo-install.json` の読み書き、削除(`data/` を残す/消す)、日本語・空白を含むパス、ジョブ実行中の開始拒否。pip の起動は、テスト用の偽の Python で引数と環境変数が固定の形であることを検証する。
- Vitest: ウィザードの状態遷移・サイズ合計・コンポーネント選択のロジック。
- 実機(`cargo test -- --ignored real_engine_install`): クリーンな環境で、最小構成(基本のみ)を導入 → 検証 → 起動 → 小さなモデルで生成まで。Windows と macOS で、それぞれ所要時間・使用量を 09 の検証ログに記録する。
- 手動(09 に追加): ウィザードの全手順、途中キャンセルと再開、ネットワーク切断からの再試行、導入後にパスの入力なしで起動・生成できること、[削除](`data/` を残す/消す)、アプリ更新後の「更新があります」。

## 11. 未決事項(07 B13 に追記)

| ID | 論点 | 推奨 |
|---|---|---|
| Q-108 | 「アプリフォルダ」の場所 | **`app_data_dir/comfyui`**(アプリのインストール先ではなく、データ用フォルダ)。インストール先は更新・アンインストールで消え、macOS の `.app` は書き込めないため。アプリのインストール先そのものに置きたい場合は、理由と併せて指示してほしい |
| Q-109 | コンポーネントの選択 | **基本(必須)+ Flux(GGUF)・顔補正・ControlNet 前処理を選択式(既定はすべてオン)**。顔補正は ultralytics(AGPL)を含むため外せるようにする |
| Q-110 | 導入後の管理モード | **導入を明示操作したので、管理モードを自動で有効にしパスを設定する(自動起動はオフ)** |
| Q-111 | 通信の例外 | **3 つ目の例外として、導入ボタン+同意ダイアログのときだけ許可リストのホストへ接続**。自動更新はしない |
| Q-112 | ライセンスの扱い | **再配布せず、利用者の操作で取得・別プロセスとして起動。同意ダイアログでライセンス一覧を提示し承認を必須にする** |
| Q-113 | AMD / Intel GPU・CPU のみ・古いドライバ | **対象外(事前チェックで案内して止める。CPU 版は導入しない)**。別の CUDA 版や ROCm は後続 |
| Q-114 | 導入方式 | **python-build-standalone + ComfyUI のソース zip + pip(OS 共通の 1 経路)**。Windows 版ポータブル(公式 7z、約 2GB)を使う案は、導入が単純だが macOS に使えず、カスタムノードの固定も別経路になるため採用しない |
| Q-115 | アンインストール時のデータ | **自動では消さない**(設定画面の [削除] を案内)。誤削除の危険があるため |

## 12. 実装の分割(マイルストーン M19)

1. マニフェスト(`manifest.json`。41 の生成スクリプトで `versions.env` から生成)と `versions.env` との一致テスト。
2. 取得・展開の基盤(12 §6 の再利用+アーカイブ展開の安全化)。
3. `EngineInstaller`(事前チェック・手順・冪等・検証・キャンセル)と IPC・イベント(04 に追記)。
4. 導入後の `ManagedEngineConfig` の自動設定、削除・修復・更新。
5. ウィザード UI・設定画面・初回案内(ja / en)。
6. 実機検証(Windows・macOS)と 09 / README の追記。

## 13. 実装時の判断(設計との差異)・検証(実装済み)

- **実装の場所**: `services/engine_install/`(`manifest.rs` / `manifest.json` / `archive.rs` / `mod.rs`)、`commands/engine_install.rs`、`services/engine_process/known.rs`(22)。フロントは `lib/tauri/engine-install.ts`・`stores/engine-install-store.ts`・`components/settings/engine-install-section.tsx`(ウィザード)。追加クレートは `zip`(=9.7.3、deflate のみ)・`tar`(=0.4.46)・`flate2`(=1.1.11)。
- **取得の固定**: ソースはコミット固定の `archive/<commit>.zip` を使い、SHA-256 をマニフェストに記録した(タグ名の URL は展開後のフォルダ名が変わりハッシュも変わるため使わない)。同じ URL を繰り返し取得して、ハッシュが一致することを確認した。Python は GitHub Release のダイジェストと同じ値を記載。
- **取得基盤**: 12 §6 の `download` をそのまま再利用し、許可ホストだけを差し替えられる `DownloadPolicy::with_hosts` を追加した。エラーコードは `MODELS_*` を `ENGINE_INSTALL_*` に対応づけて返す。
- **完了印**: 各手順の完了を、入力の識別子(ハッシュやバージョン)つきで `.gazooo-install.json` に記録する。マニフェストの版が変わると、印をすべて無効にして取得し直す(「更新」)。[修復] は、印が揃っていてもファイルが無い手順だけをやり直し、最後に検証を実行する。
- **検証(手順 8)**: 共有の `EngineProcess`(15)は使わず、空きポートで ComfyUI を **`--cpu` で一時起動**し、`/object_info` に選択したコンポーネントのノードがあることを確認して停止する(最大 300 秒。起動の途中で終了した場合は即失敗)。GPU の状態に左右されない。
- **pip の環境**: `PYTHONNOUSERSITE=1` などを固定し、`PIP_INDEX_URL` / `PIP_EXTRA_INDEX_URL` / `PIP_FIND_LINKS` / `PIP_CONFIG_FILE` / `PYTHONHOME` / `PYTHONPATH` を除去する(利用者の環境変数で導入先・取得元が変わらないように)。`PIP_REQUIRE_VIRTUALENV` も除去(venv の pip を直接呼ぶため)。
- **削除**: `keepData` なら `python / venv / ComfyUI / downloads` と印だけを消し、`data/` は触らない。管理設定が削除した導入を指していれば無効(既定値)に戻す。
- **初回案内**: 未接続のバナーに、「ComfyUI がありませんか?」の 1 行と [設定でインストール] を出す(ダイアログは自動で開かない)。
- **実機検証(2026-11-20、Windows 11・RTX 2060・日本語のユーザー名)**: クリーンなフォルダに「基本 + Flux(GGUF)」を導入(`real_engine_install`、ignore)。**346 秒で完了**(Python 取得 → venv → ComfyUI → PyTorch 2.14.1(cu130)→ 依存 → ComfyUI-GGUF → 検証で `UnetLoaderGGUF` を確認)。導入後の使用量は **約 4.6GB**(`downloads` は完了後に削除)。同じフォルダでの再実行は **15 秒**(完了済みの手順を飛ばし、検証だけ実行)。顔補正・ControlNet 前処理(ultralytics・opencv)の導入と、macOS は未検証。
