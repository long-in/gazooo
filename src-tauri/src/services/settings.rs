//! 設定の読み書き(docs/03-data-model.md「設定キー」)。`settings` テーブルの KV を型付きで扱う。

use rusqlite::Connection;
use serde::{Deserialize, Deserializer, Serialize};

use crate::db::scopes::{self, Layer, Scope};
use crate::error::{AppError, AppResult};

pub const DEFAULT_COMFYUI_URL: &str = "http://127.0.0.1:8188";

const KEY_COMFYUI_URL: &str = "engine.comfyui.url";
const KEY_IMAGES_DIR: &str = "storage.images_dir";
const KEY_THEME: &str = "ui.theme";
const KEY_UI_STATE: &str = "ui.state";
const KEY_LANGUAGE: &str = "ui.language";
const KEY_GENERATION_DEFAULTS: &str = "generation.default_params";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    #[default]
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Ja,
    En,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub comfyui_url: String,
    /// 画像の保存先の個別設定。`None` は全体設定を使う。
    pub images_dir: Option<String>,
    /// 画像の保存先の全体設定。`None` はアプリのデータ内。
    pub images_root: Option<String>,
    /// 以前の版の画像の場所を使い続けるユーザー(docs/52 §5)。
    #[serde(skip)]
    pub legacy_images_user: Option<String>,
    pub theme: Theme,
    /// `None` は OS のロケールに従う。
    pub language: Option<Language>,
    /// 生成フォームの下書き(`GenerationParams` の JSON)。`None` は組み込みの既定値。
    /// 形式の検証は `job_enqueue` で行うため、ここではオブジェクトであることだけを確認する。
    pub generation_defaults: Option<serde_json::Value>,
    /// 画面の状態(パネルの開閉・幅・表示の密度。docs/29 §6)の JSON。`None` は既定。オブジェクトであることだけを確認する。
    pub ui_state: Option<serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            comfyui_url: DEFAULT_COMFYUI_URL.to_string(),
            images_dir: None,
            images_root: None,
            legacy_images_user: None,
            theme: Theme::default(),
            language: None,
            generation_defaults: None,
            ui_state: None,
        }
    }
}

/// 部分更新。`scope` は、上書きできる項目(テーマ・言語)の書き込み先。`imagesDir` / `language` は「未指定(変更しない)」と「null(既定に戻す)」を区別する。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    #[serde(default)]
    pub scope: Scope,
    pub comfyui_url: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub images_dir: Option<Option<String>>,
    pub theme: Option<Theme>,
    #[serde(default, deserialize_with = "double_option")]
    pub language: Option<Option<Language>>,
    #[serde(default, deserialize_with = "double_option")]
    pub generation_defaults: Option<Option<serde_json::Value>>,
    #[serde(default, deserialize_with = "double_option")]
    pub ui_state: Option<Option<serde_json::Value>>,
}

fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn get_raw(conn: &Connection, key: &str) -> AppResult<Option<serde_json::Value>> {
    scopes::get_json(conn, Layer::Effective, key)
}

fn set_raw(conn: &Connection, key: &str, value: &serde_json::Value) -> AppResult<()> {
    scopes::write(conn, Scope::User, key, Some(value))
}

fn delete_raw(conn: &Connection, key: &str) -> AppResult<()> {
    scopes::write(conn, Scope::User, key, None)
}

fn parse<T: serde::de::DeserializeOwned>(v: Option<serde_json::Value>) -> Option<T> {
    // 壊れた値は既定値扱い(起動不能にしない)
    v.and_then(|v| serde_json::from_value(v).ok())
}

/// ComfyUI の接続先(端末共通の設定。未ログインでも読める)。
pub fn get_comfyui_url(conn: &Connection) -> AppResult<String> {
    Ok(parse(get_raw(conn, KEY_COMFYUI_URL)?).unwrap_or_else(|| DEFAULT_COMFYUI_URL.to_string()))
}

pub fn load(conn: &Connection) -> AppResult<Settings> {
    load_in(conn, Layer::Effective)
}

/// 全体の画像の保存先と、以前の版の場所を使うユーザー。
pub fn global_images(conn: &Connection) -> AppResult<(Option<String>, Option<String>)> {
    Ok((
        parse(get_raw(conn, scopes::KEY_IMAGES_ROOT)?),
        parse(get_raw(conn, scopes::KEY_LEGACY_IMAGES_USER)?),
    ))
}

/// `Layer::Global` は全体設定の値(ログイン前でも読める。個人の項目は空)。
pub fn load_in(conn: &Connection, layer: Layer) -> AppResult<Settings> {
    let d = Settings::default();
    let get = |key: &str| scopes::get_json(conn, layer, key);
    let personal = |key: &str| match layer {
        Layer::Effective => get_raw(conn, key),
        Layer::Global => Ok(None),
    };
    let (images_root, legacy_images_user) = global_images(conn)?;
    Ok(Settings {
        comfyui_url: parse(get(KEY_COMFYUI_URL)?).unwrap_or(d.comfyui_url),
        images_dir: parse(personal(KEY_IMAGES_DIR)?),
        images_root,
        legacy_images_user,
        theme: parse(get(KEY_THEME)?).unwrap_or(d.theme),
        language: parse(get(KEY_LANGUAGE)?),
        generation_defaults: personal(KEY_GENERATION_DEFAULTS)?.filter(|v| v.is_object()),
        ui_state: personal(KEY_UI_STATE)?.filter(|v| v.is_object()),
    })
}

// モデル取得(docs/12 §3・§9)。`Settings` には含めず、専用コマンドで検証つきに変更する。
const KEY_MODELS_DIR: &str = "engine.comfyui.models_dir";
const KEY_MODEL_CONSENTS: &str = "models.consents";

/// ユーザーが指定した ComfyUI の `models` フォルダ。`None` は自動検出。
pub fn get_models_dir(conn: &Connection) -> AppResult<Option<String>> {
    Ok(parse(get_raw(conn, KEY_MODELS_DIR)?))
}

pub fn set_models_dir(conn: &Connection, dir: Option<&str>) -> AppResult<()> {
    match dir {
        Some(d) => set_raw(conn, KEY_MODELS_DIR, &serde_json::json!(d)),
        None => delete_raw(conn, KEY_MODELS_DIR),
    }
}

/// ライセンスに同意済みのモデル(`familyId/variantId`)。
pub fn get_model_consents(conn: &Connection) -> AppResult<Vec<String>> {
    Ok(parse(get_raw(conn, KEY_MODEL_CONSENTS)?).unwrap_or_default())
}

pub fn add_model_consent(conn: &Connection, key: &str) -> AppResult<()> {
    let mut all = get_model_consents(conn)?;
    if !all.iter().any(|k| k == key) {
        all.push(key.to_string());
        set_raw(conn, KEY_MODEL_CONSENTS, &serde_json::json!(all))?;
    }
    Ok(())
}

// ComfyUI の起動管理(docs/15 §3)。`Settings` には含めず、専用コマンドで検証つきに変更する。
const KEY_MANAGED_ENGINE: &str = "engine.comfyui.managed";

pub fn get_managed_engine(
    conn: &Connection,
) -> AppResult<crate::services::engine_process::config::ManagedEngineConfig> {
    Ok(parse(get_raw(conn, KEY_MANAGED_ENGINE)?).unwrap_or_default())
}

pub fn set_managed_engine(
    conn: &Connection,
    cfg: &crate::services::engine_process::config::ManagedEngineConfig,
) -> AppResult<()> {
    set_raw(conn, KEY_MANAGED_ENGINE, &serde_json::to_value(cfg)?)
}

// 調査用の詳細ログ(docs/31)。ログレベルの切り替えを伴うため、`Settings` には含めず専用コマンドで変更する。
const KEY_DEBUG_ENABLED: &str = "debug.enabled";

pub fn get_debug_enabled(conn: &Connection) -> AppResult<bool> {
    Ok(parse(get_raw(conn, KEY_DEBUG_ENABLED)?).unwrap_or(false))
}

pub fn set_debug_enabled(conn: &Connection, enabled: bool) -> AppResult<()> {
    set_raw(conn, KEY_DEBUG_ENABLED, &serde_json::json!(enabled))
}

// 画像の形式変換(docs/43 §8)。ユーザーごとの設定。`Settings` には含めず、専用コマンドで検証つきに変更する。
const KEY_CONVERT: &str = "convert.settings";

pub fn get_convert(conn: &Connection) -> AppResult<crate::services::convert::ConvertSettings> {
    get_convert_in(conn, Layer::Effective)
}

pub fn get_convert_in(
    conn: &Connection,
    layer: Layer,
) -> AppResult<crate::services::convert::ConvertSettings> {
    Ok(parse(scopes::get_json(conn, layer, KEY_CONVERT)?).unwrap_or_default())
}

pub fn set_convert(
    conn: &Connection,
    scope: Scope,
    cfg: &crate::services::convert::ConvertSettings,
) -> AppResult<()> {
    scopes::write(conn, scope, KEY_CONVERT, Some(&serde_json::to_value(cfg)?))
}

const KEY_EXPORT_DIR: &str = "gallery.export_dir";

/// ギャラリーの一括ダウンロードで前回選んだ保存先(ダイアログの初期位置。docs/44 §2)。
pub fn get_export_dir(conn: &Connection) -> AppResult<Option<String>> {
    Ok(parse(get_raw(conn, KEY_EXPORT_DIR)?))
}

pub fn set_export_dir(conn: &Connection, dir: &str) -> AppResult<()> {
    set_raw(
        conn,
        KEY_EXPORT_DIR,
        &serde_json::Value::String(dir.to_owned()),
    )
}

fn validate_url(url: &str) -> AppResult<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| {
            AppError::validation(
                "comfyuiUrl",
                "URL は http:// または https:// で始めてください",
            )
        })?;
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Err(AppError::validation(
            "comfyuiUrl",
            "URL の形式が正しくありません",
        ));
    }
    Ok(url.to_string())
}

pub fn update(conn: &mut Connection, patch: SettingsPatch) -> AppResult<Settings> {
    // 先に検証してからトランザクションで書き込む
    let url = patch.comfyui_url.as_deref().map(validate_url).transpose()?;
    if let Some(Some(dir)) = &patch.images_dir
        && !std::path::Path::new(dir).is_absolute()
    {
        return Err(AppError::validation(
            "imagesDir",
            "保存先は絶対パスで指定してください",
        ));
    }

    if let Some(Some(v)) = &patch.generation_defaults
        && !v.is_object()
    {
        return Err(AppError::validation(
            "generationDefaults",
            "生成設定の形式が正しくありません",
        ));
    }

    if let Some(Some(v)) = &patch.ui_state
        && !v.is_object()
    {
        return Err(AppError::validation(
            "uiState",
            "画面の状態の形式が正しくありません",
        ));
    }

    let scope = patch.scope;
    let tx = conn.transaction()?;
    if let Some(url) = url {
        set_raw(&tx, KEY_COMFYUI_URL, &serde_json::json!(url))?;
    }
    match patch.images_dir {
        Some(Some(dir)) => set_raw(&tx, KEY_IMAGES_DIR, &serde_json::json!(dir))?,
        Some(None) => delete_raw(&tx, KEY_IMAGES_DIR)?,
        None => {}
    }
    if let Some(theme) = patch.theme {
        let value = serde_json::to_value(theme)?;
        scopes::write(&tx, scope, KEY_THEME, Some(&value))?;
    }
    if let Some(language) = patch.language {
        let value = language.map(serde_json::to_value).transpose()?;
        scopes::write(&tx, scope, KEY_LANGUAGE, value.as_ref())?;
    }
    match patch.generation_defaults {
        Some(Some(v)) => set_raw(&tx, KEY_GENERATION_DEFAULTS, &v)?,
        Some(None) => delete_raw(&tx, KEY_GENERATION_DEFAULTS)?,
        None => {}
    }
    match patch.ui_state {
        Some(Some(v)) => set_raw(&tx, KEY_UI_STATE, &v)?,
        Some(None) => delete_raw(&tx, KEY_UI_STATE)?,
        None => {}
    }
    tx.commit()?;
    load(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[tokio::test]
    async fn defaults_when_empty() {
        let db = Db::open_in_memory().unwrap();
        let s = db.call(|c| load(c)).await.unwrap();
        assert_eq!(s, Settings::default());
    }

    #[tokio::test]
    async fn debug_flag_defaults_off_and_roundtrips() {
        let db = Db::open_in_memory().unwrap();
        assert!(!db.call(|c| get_debug_enabled(c)).await.unwrap());
        db.call(|c| set_debug_enabled(c, true)).await.unwrap();
        assert!(db.call(|c| get_debug_enabled(c)).await.unwrap());
        db.call(|c| set_debug_enabled(c, false)).await.unwrap();
        assert!(!db.call(|c| get_debug_enabled(c)).await.unwrap());
    }

    #[tokio::test]
    async fn partial_update_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let patch: SettingsPatch = serde_json::from_str(
            r#"{"comfyuiUrl":"http://localhost:8188/","theme":"dark","language":"en"}"#,
        )
        .unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert_eq!(s.comfyui_url, "http://localhost:8188");
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.language, Some(Language::En));

        // 未指定は変更されず、null は既定に戻る
        let patch: SettingsPatch = serde_json::from_str(r#"{"language":null}"#).unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.language, None);
    }

    #[tokio::test]
    async fn rejects_invalid_url_without_writing() {
        let db = Db::open_in_memory().unwrap();
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"comfyuiUrl":"ftp://x","theme":"light"}"#).unwrap();
        let err = db.call(move |c| update(c, patch)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
        let s = db.call(|c| load(c)).await.unwrap();
        assert_eq!(
            s.theme,
            Theme::default(),
            "検証エラー時は一切書き込まれない"
        );
    }

    #[tokio::test]
    async fn images_dir_must_be_absolute() {
        let db = Db::open_in_memory().unwrap();
        let patch: SettingsPatch = serde_json::from_str(r#"{"imagesDir":"relative/dir"}"#).unwrap();
        let err = db.call(move |c| update(c, patch)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[tokio::test]
    async fn corrupt_value_falls_back_to_default() {
        let db = Db::open_in_memory().unwrap();
        let s = db
            .call(|c| {
                set_raw(c, KEY_THEME, &serde_json::json!("neon"))?;
                load(c)
            })
            .await
            .unwrap();
        assert_eq!(s.theme, Theme::default());
    }

    #[tokio::test]
    async fn generation_defaults_roundtrip_and_clear() {
        let db = Db::open_in_memory().unwrap();
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"generationDefaults":{"prompt":"a cat","steps":20}}"#)
                .unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert_eq!(s.generation_defaults.unwrap()["prompt"], "a cat");

        // 他の項目の更新では変わらない
        let patch: SettingsPatch = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert!(s.generation_defaults.is_some());

        // オブジェクト以外は拒否、null で既定に戻る
        let bad: SettingsPatch = serde_json::from_str(r#"{"generationDefaults":"x"}"#).unwrap();
        let err = db.call(move |c| update(c, bad)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
        let clear: SettingsPatch = serde_json::from_str(r#"{"generationDefaults":null}"#).unwrap();
        let s = db.call(move |c| update(c, clear)).await.unwrap();
        assert!(s.generation_defaults.is_none());
    }

    #[tokio::test]
    async fn ui_state_roundtrip_and_clear() {
        let db = Db::open_in_memory().unwrap();
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"uiState":{"density":"compact","panels":{"a":false}}}"#)
                .unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert_eq!(s.ui_state.unwrap()["density"], "compact");

        // 他の項目の更新では変わらない
        let patch: SettingsPatch = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        let s = db.call(move |c| update(c, patch)).await.unwrap();
        assert!(s.ui_state.is_some());

        // オブジェクト以外は拒否、null で既定に戻る
        let bad: SettingsPatch = serde_json::from_str(r#"{"uiState":"x"}"#).unwrap();
        let err = db.call(move |c| update(c, bad)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
        let clear: SettingsPatch = serde_json::from_str(r#"{"uiState":null}"#).unwrap();
        let s = db.call(move |c| update(c, clear)).await.unwrap();
        assert!(s.ui_state.is_none());
    }
}
