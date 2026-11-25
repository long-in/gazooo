//! 画像の形式変換(docs/43 §8)。パスはフロントエンドから受け取らない(ダイアログ・ドロップは Rust 側)。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::convert::{
    ConvertEntry, ConvertEvents, ConvertOptions, ConvertService, ConvertSettings,
    ConvertSettingsPatch, ConvertSettingsView, Finished, ItemResult, Progress, default_output_dir,
};
use crate::services::settings;

type Svc<'a> = State<'a, Arc<ConvertService>>;

pub struct TauriConvertEvents(pub AppHandle);

impl ConvertEvents for TauriConvertEvents {
    fn added(&self, entries: &[ConvertEntry]) {
        let _ = self.0.emit("convert://added", entries);
    }
    fn progress(&self, p: &Progress) {
        let _ = self.0.emit("convert://progress", p);
    }
    fn item(&self, r: &ItemResult) {
        let _ = self.0.emit("convert://item", r);
    }
    fn finished(&self, f: &Finished) {
        let _ = self.0.emit("convert://finished", f);
    }
}

/// ドロップされたパスを取り込む(ウィンドウイベントから呼ぶ。変換ページが開いているときだけ)。
pub fn handle_drop(app: &AppHandle, paths: Vec<PathBuf>) {
    let Some(svc) = app.try_state::<Arc<ConvertService>>() else {
        return;
    };
    if !svc.accepting() {
        return;
    }
    let svc = Arc::clone(&svc);
    let events = TauriConvertEvents(app.clone());
    tauri::async_runtime::spawn(async move {
        let added = tokio::task::spawn_blocking(move || svc.add_paths(paths)).await;
        if let Ok(Ok(entries)) = added
            && !entries.is_empty()
        {
            events.added(&entries);
        }
    });
}

/// ドラッグ中の表示(ドロップ領域の強調)用。
pub fn handle_drag_state(app: &AppHandle, over: bool) {
    if let Some(svc) = app.try_state::<Arc<ConvertService>>()
        && svc.accepting()
    {
        let _ = app.emit("convert://drag", serde_json::json!({ "over": over }));
    }
}

fn view(app: &AppHandle, s: ConvertSettings) -> ConvertSettingsView {
    let dir = s
        .output_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_output_dir(app.path().picture_dir().ok()));
    ConvertSettingsView {
        output_dir: dir.to_string_lossy().into_owned(),
        settings: s,
    }
}

#[tauri::command]
pub async fn convert_settings_get(
    app: AppHandle,
    db: State<'_, Db>,
) -> AppResult<ConvertSettingsView> {
    let s = db.call(|c| settings::get_convert(c)).await?;
    Ok(view(&app, s))
}

#[tauri::command]
pub async fn convert_settings_update(
    app: AppHandle,
    db: State<'_, Db>,
    patch: ConvertSettingsPatch,
) -> AppResult<ConvertSettingsView> {
    let s = db
        .call(move |c| {
            let mut s = settings::get_convert(c)?;
            s.apply(patch)?;
            settings::set_convert(c, &s)?;
            Ok(s)
        })
        .await?;
    Ok(view(&app, s))
}

/// 出力先フォルダを選び、検証して保存する。キャンセルなら `None`。
#[tauri::command]
pub async fn convert_pick_output_dir(
    app: AppHandle,
    db: State<'_, Db>,
) -> AppResult<Option<ConvertSettingsView>> {
    let handle = app.clone();
    let picked = tokio::task::spawn_blocking(move || handle.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(dir) = picked else {
        return Ok(None);
    };
    let dir = dir
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(AppError::validation(
            "outputDir",
            "出力先のフォルダが不正です",
        ));
    }
    let d = dir.to_string_lossy().into_owned();
    let s = db
        .call(move |c| {
            let mut s = settings::get_convert(c)?;
            s.output_dir = Some(d);
            settings::set_convert(c, &s)?;
            Ok(s)
        })
        .await?;
    Ok(Some(view(&app, s)))
}

#[tauri::command]
pub async fn convert_pick_files(app: AppHandle, svc: Svc<'_>) -> AppResult<Vec<ConvertEntry>> {
    let handle = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .add_filter(
                "Image",
                &[
                    "webp", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "heic", "heif",
                ],
            )
            .blocking_pick_files()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let mut paths = Vec::new();
    for f in picked.unwrap_or_default() {
        paths.push(
            f.into_path()
                .map_err(|e| AppError::Storage(e.to_string()))?,
        );
    }
    add(svc, paths).await
}

#[tauri::command]
pub async fn convert_pick_folder(app: AppHandle, svc: Svc<'_>) -> AppResult<Vec<ConvertEntry>> {
    let handle = app.clone();
    let picked = tokio::task::spawn_blocking(move || handle.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(dir) = picked else {
        return Ok(Vec::new());
    };
    let dir = dir
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    add(svc, vec![dir]).await
}

async fn add(svc: Svc<'_>, paths: Vec<PathBuf>) -> AppResult<Vec<ConvertEntry>> {
    let svc = Arc::clone(&svc);
    tokio::task::spawn_blocking(move || svc.add_paths(paths))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

/// 現在の一覧(ページを開き直したときの復元用)。
#[tauri::command]
pub async fn convert_list(svc: Svc<'_>) -> AppResult<Vec<ConvertEntry>> {
    svc.list()
}

#[tauri::command]
pub async fn convert_remove(svc: Svc<'_>, ids: Vec<String>) -> AppResult<()> {
    svc.remove(&ids)
}

#[tauri::command]
pub async fn convert_clear(svc: Svc<'_>) -> AppResult<()> {
    svc.clear()
}

/// 変換ページが開いている間だけ、ドロップを受け付ける。
#[tauri::command]
pub async fn convert_set_accepting(svc: Svc<'_>, on: bool) -> AppResult<()> {
    svc.set_accepting(on);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub ids: Vec<String>,
    #[serde(flatten)]
    pub options: ConvertOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStarted {
    pub run_id: String,
}

#[tauri::command]
pub async fn convert_run(
    app: AppHandle,
    db: State<'_, Db>,
    svc: Svc<'_>,
    request: RunRequest,
) -> AppResult<RunStarted> {
    let s = db.call(|c| settings::get_convert(c)).await?;
    let out_dir = PathBuf::from(view(&app, s).output_dir);
    let run_id = svc.run(
        request.ids,
        request.options,
        out_dir,
        Arc::new(TauriConvertEvents(app.clone())),
    )?;
    Ok(RunStarted { run_id })
}

#[tauri::command]
pub async fn convert_cancel(svc: Svc<'_>, run_id: String) -> AppResult<()> {
    svc.cancel(&run_id);
    Ok(())
}

#[tauri::command]
pub async fn convert_open_output_dir(app: AppHandle, db: State<'_, Db>) -> AppResult<()> {
    let s = db.call(|c| settings::get_convert(c)).await?;
    let dir = view(&app, s).output_dir;
    if !std::path::Path::new(&dir).is_dir() {
        return Err(AppError::validation(
            "outputDir",
            "出力先のフォルダがまだありません",
        ));
    }
    tauri_plugin_opener::open_path(dir, None::<&str>).map_err(|e| AppError::Storage(e.to_string()))
}
