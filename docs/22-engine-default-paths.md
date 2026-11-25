# 22. ComfyUI の既定パスの保持(v0.14、M18)

> ステータス: 🟢 承認済み(2026-11-20。07 Q-104〜107 は推奨どおり)・実装済み(`services/engine_process/known.rs`)。15 §3.1 / §12「初期値の自動入力」の拡張。

## 1. 目的と現状の問題

インストーラーで入れたアプリを初めて起動すると、設定 > ComfyUI の起動管理の `main.py` / Python / base-directory が **すべて空欄** になり、利用者が自分で探して入力する必要がある。

原因: 現在の自動入力(15 §12)は、(1) 起動中の ComfyUI の `argv`、(2) **アプリの実行ファイルとカレントディレクトリから親方向 8 階層** の `comfyui/ComfyUI/main.py` を探すだけで、後者は開発時のフォルダ構成(08)でしか見つからない。インストール版では (2) は常に空振りする。

目的: **アプリが「ComfyUI がよくある場所」の一覧を内蔵し、初回から候補を初期値として入れる**。利用者は候補を確認して保存するだけにする。

スコープ外: ComfyUI 本体のダウンロード・インストールをアプリが行うこと(**[23-engine-installer.md](23-engine-installer.md) で別に設計**。Q-104)。モードの既定(`external`、15 Q-45)は変えない(オプトインのまま)。

## 2. 方針

- 候補の探索は **Rust 側**(`services/engine_process/config.rs` の `detect` を拡張)。探索対象は **アプリが内蔵する既定の場所の一覧** と、従来の「実行ファイル周辺」。ユーザーの設定・ファイルは変更しない(読み取りのみ)。
- 見つかった最初の 1 件を `suggestions` として返す(15 §7 の戻り値は変えない)。**確定はユーザー**(有効化して [保存] したときだけ設定に書く)。勝手に有効化・起動しない。
- 探索の優先順位: ① 保存済みの設定(あればそのまま。候補は使わない)→ ② 起動中の ComfyUI の `argv` → ③ 既定の場所の一覧(下記)→ ④ 従来の実行ファイル周辺(開発用)→ ⑤ 空欄。
- パスは `std::path` / Tauri path API(`home_dir`・`document_dir`・`desktop_dir`・`download_dir`・`local_data_dir`)で組み立てる。区切り文字を直書きしない(06)。

## 3. 内蔵する既定の場所(一覧)

各ルート `<R>` で `<R>/main.py`(ComfyUI 直下)を探す。Python は §3.1、base-directory は §3.2 に従う。実在確認は `is_file()` / `is_dir()` のみ(深い再帰探索はしない。各ルートは 1 階層)。

| 種類 | OS | ルート `<R>`(の候補。上から探す) |
|---|---|---|
| **アプリが導入した ComfyUI(23)** | 共通 | `<app_data>/comfyui/ComfyUI`(最優先。Python は `<app_data>/comfyui/venv`、base-directory は `<app_data>/comfyui/data`) |
| 手動インストール(08 のレイアウト) | 共通 | `<home>/ComfyUI`、`<home>/comfyui/ComfyUI`、`<documents>/ComfyUI`、`<desktop>/ComfyUI` |
| Windows 版ポータブル | Windows | `<home>/ComfyUI_windows_portable/ComfyUI`、`<desktop>/ComfyUI_windows_portable/ComfyUI`、`<downloads>/ComfyUI_windows_portable/ComfyUI`、`C:/ComfyUI_windows_portable/ComfyUI`、`C:/ComfyUI` |
| ComfyUI Desktop(公式デスクトップ版) | Windows | `<local_data>/Programs/ComfyUI/resources/ComfyUI`(実機で要確認。§7 リスク) |
| ComfyUI Desktop | macOS | `/Applications/ComfyUI.app/Contents/Resources/ComfyUI`(実機で要確認) |

一覧は **コード内の定数**(1 か所に集約し、ユニットテストで固定)。利用者が追加できる設定は作らない(必要なら §6 Q-105)。

### 3.1 Python の候補

`<R>` の親と `<R>` を基準に、最初に実在するものを入れる(Windows は `Scripts/python.exe`、macOS は `bin/python`):

1. `<R>/../venv`、`<R>/../.venv`、`<R>/venv`、`<R>/.venv`
2. ポータブル版: `<R>/../python_embeded/python.exe`
3. ComfyUI Desktop: `<documents>/ComfyUI/.venv`(Desktop は仮想環境を利用者の ComfyUI フォルダに作る。実機で要確認)

見つからなければ空欄のまま(従来どおり手入力)。システムの `python` を PATH から勝手に選ぶことはしない(依存関係が揃っていない可能性が高く、誤起動の原因になる)。

### 3.2 base-directory の候補

`<R>/../data`(08 のレイアウト)があれば入れる。ComfyUI Desktop は `<documents>/ComfyUI` があれば入れる。無ければ空欄(ComfyUI 既定の `<R>` 配下を使う)。

## 4. UI

- 設定 > ComfyUI の起動管理: 空欄の項目は候補を **薄い色で初期値として表示**し、「候補を入れました。確認して [保存] してください」の注記を出す(従来の「候補を入れる」ボタンは残す)。候補が無い項目には「見つかりませんでした。[参照…] で選んでください」と表示する。
- 見つかった場所の種類(例: 「ComfyUI Desktop が見つかりました」)を 1 行で示す(i18n、`engine.process.found.<種類>`)。
- 初回起動時に、設定が未入力で候補がある場合だけ、設定画面への案内(バッジのツールチップ。ダイアログは出さない)を出す。

## 5. IPC / 保存

- IPC の追加・変更は **なし**(`engine_managed_get` の `suggestions` に、より多くの場合で値が入るだけ)。`suggestions` に `source`(`running` / `portable` / `desktop` / `manual` / `dev`)の任意フィールドを追加する(UI の 1 行表示用。後方互換: 無くても動く)。04 の該当箇所に追記する。
- DB・設定スキーマの変更なし。保存済みの設定は常に候補より優先する。

## 6. 未決事項(07 B13 に追記)

| ID | 論点 | 推奨 |
|---|---|---|
| Q-104 | ComfyUI 本体をアプリが導入するか | **導入する。ただし 22 とは別設計(23)**。22 は既存の導入を探して初期値に入れる部分だけ |
| Q-105 | 探索場所を利用者が追加できるようにするか | **しない**(内蔵の一覧のみ。見つからなければ [参照…] で指定)。要望があれば後続で設定に追加 |
| Q-106 | 初期値の入れ方 | **候補を画面の初期値として表示するだけ**(保存・有効化は利用者の操作)。起動管理を自動で有効にはしない(15 Q-45 を維持) |
| Q-107 | システムの `python` の利用 | **使わない**(PATH からの自動選択はしない) |

## 7. リスク

- ComfyUI Desktop・ポータブル版の実際のフォルダ構成が、バージョンで変わる。→ 一覧は定数に集約して直しやすくし、**見つからなければ空欄で従来どおり**(害はない)。Desktop の構成は実機で確認してから 09 に記録する。
- 候補が別のバージョンの ComfyUI を指す。→ 確定は利用者。保存時の検証(15 §3.1)は従来どおり。

## 8. テスト

- Rust: 一時ディレクトリに各レイアウト(手動・ポータブル・Desktop 相当)を作り、`home` / `documents` などの基準ディレクトリを **引数で注入できる形** にして `detect` を検証する(優先順位・Python / base-directory の選択・見つからない場合の空)。OS 依存の分岐は `cfg!(windows)` で分け、パスの組み立てに区切り文字を直書きしない。
- 手動(09 に追加): 新規インストールの Windows で、ポータブル版 / Desktop / 手動インストールのそれぞれを置いて、設定画面に候補が入ること。何も無い環境で空欄・案内が出ること。

## 9. 実装の分割(マイルストーン M18)

1. `detect` を、基準ディレクトリを注入できる形に変更し、§3 の一覧と優先順位を実装(Rust テスト付き)。
2. `suggestions.source` の追加と、設定画面の候補表示・注記(ja / en)。
3. 09 のチェックリストと、実機での構成確認の記録。
