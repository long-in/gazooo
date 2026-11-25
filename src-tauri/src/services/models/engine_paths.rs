//! 保存先を ComfyUI に反映する(docs/08-local-comfyui.md §6.2)。
//!
//! ComfyUI は `main.py` と同じフォルダの `extra_model_paths.yaml` を起動時に読む。
//! アプリが書いたファイルにだけ印(先頭コメント)を付け、ユーザー自身の設定は上書きしない。

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub const FILE_NAME: &str = "extra_model_paths.yaml";
/// アプリが書いたファイルの印(先頭行)。
pub const MARKER: &str = "# managed by gazooo";
/// アプリ名変更前(gazoo)の印。旧印のファイルも「アプリが書いたもの」として扱う(docs/07 Q-52b)。
const LEGACY_MARKER: &str = "# managed by gazoo";

/// ComfyUI のフォルダ名 → `base_path` からの相対位置(12 §4.2 の取得先と一致させる)。
const SECTIONS: &[(&str, &str)] = &[
    ("checkpoints", "checkpoints"),
    ("clip", "clip"),
    ("text_encoders", "clip"),
    ("vae", "vae"),
    ("unet", "unet"),
    ("diffusion_models", "unet"),
    ("loras", "loras"),
    ("controlnet", "controlnet"),
    ("ultralytics", "ultralytics"),
    ("ultralytics_bbox", "ultralytics/bbox"),
];

/// 起動引数の先頭(`main.py`)と同じフォルダの設定ファイル。絶対パスでなければ特定できない。
pub fn file_for_argv(argv: &[String]) -> Option<PathBuf> {
    let main = Path::new(argv.first()?);
    let file = main.parent()?.join(FILE_NAME);
    file.is_absolute().then_some(file)
}

/// YAML の二重引用符つき文字列にする(`\` と `"` をエスケープ。制御文字は不可)。
fn quote(s: &str) -> AppResult<String> {
    if s.chars().any(char::is_control) {
        return Err(AppError::validation("path", "使えない文字が含まれています"));
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if matches!(c, '\\' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

/// ファイルの内容。`models_dir` は絶対パス。
pub fn render(models_dir: &Path) -> AppResult<String> {
    let base = quote(&models_dir.to_string_lossy())?;
    let mut out = format!(
        "{MARKER}\n# 設定の「ComfyUI に反映」で作成されました。手で編集しても構いませんが、印の行を消すと上書きされなくなります。\ngazooo:\n  base_path: {base}\n"
    );
    for (name, rel) in SECTIONS {
        out.push_str(&format!("  {name}: {rel}/\n"));
    }
    Ok(out)
}

/// アプリが書いたファイルか(先頭行が印)。ファイルが無ければ `None`。
pub fn is_managed(file: &Path) -> AppResult<Option<bool>> {
    match std::fs::read_to_string(file) {
        Ok(s) => Ok(Some(s.lines().next().is_some_and(|l| {
            let l = l.trim();
            l == MARKER || l == LEGACY_MARKER
        }))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        // 読めない(UTF-8 でない等)ものは、ユーザーのファイルとして扱う
        Err(_) => Ok(Some(false)),
    }
}

fn foreign() -> AppError {
    AppError::models(
        "MODELS_EXTRA_PATHS_FOREIGN",
        format!("{FILE_NAME} は別の設定として既に存在するため、上書きしません"),
    )
}

fn write_failed(e: &std::io::Error) -> AppError {
    AppError::models(
        "MODELS_DIR_UNKNOWN",
        format!("{FILE_NAME} を書き込めません: {e}"),
    )
}

/// 反映済みか(アプリが書いた内容が、現在の保存先と一致している)。
pub fn is_applied(file: &Path, models_dir: &Path) -> bool {
    matches!(is_managed(file), Ok(Some(true)))
        && render(models_dir)
            .ok()
            .zip(std::fs::read_to_string(file).ok())
            .is_some_and(|(want, have)| want.replace("\r\n", "\n") == have.replace("\r\n", "\n"))
}

/// 設定ファイルを書く。ユーザー自身のファイルがあれば上書きしない。
pub fn apply(file: &Path, models_dir: &Path) -> AppResult<()> {
    if !models_dir.is_absolute() || !models_dir.is_dir() {
        return Err(AppError::validation("path", "フォルダが見つかりません"));
    }
    if is_managed(file)? == Some(false) {
        return Err(foreign());
    }
    let body = render(models_dir)?;
    let tmp = file.with_file_name(format!("{FILE_NAME}.gazooo-tmp"));
    std::fs::write(&tmp, body).map_err(|e| write_failed(&e))?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        write_failed(&e)
    })
}

/// アプリが書いたファイルを削除する。無ければ何もしない。
pub fn clear(file: &Path) -> AppResult<()> {
    match is_managed(file)? {
        None => Ok(()),
        Some(false) => Err(foreign()),
        Some(true) => std::fs::remove_file(file).map_err(|e| write_failed(&e)),
    }
}

/// フォルダの同一判定(区切り・末尾の区切り・Windows での大小文字を無視)。
pub fn same_dir(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/');
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    }
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gazooo-ep-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_is_next_to_main_py() {
        let a = vec!["C:\\w\\ComfyUI\\main.py".to_string()];
        let f = file_for_argv(&a);
        if cfg!(windows) {
            assert_eq!(f, Some(PathBuf::from("C:\\w\\ComfyUI").join(FILE_NAME)));
        }
        assert_eq!(file_for_argv(&["main.py".to_string()]), None);
        assert_eq!(file_for_argv(&[]), None);
    }

    #[test]
    fn render_quotes_windows_and_special_paths() {
        let y = render(Path::new("D:\\my \"models\"\\x")).unwrap();
        assert!(y.starts_with(MARKER));
        assert!(y.contains("base_path: \"D:\\\\my \\\"models\\\"\\\\x\"\n"));
        assert!(y.contains("  checkpoints: checkpoints/\n"));
        assert!(y.contains("  ultralytics_bbox: ultralytics/bbox/\n"));
        assert!(render(Path::new("D:\\a\nb")).is_err());
    }

    #[test]
    fn apply_status_and_clear() {
        let engine = tmpdir("engine");
        let models = tmpdir("models");
        let file = engine.join(FILE_NAME);

        assert_eq!(is_managed(&file).unwrap(), None);
        assert!(!is_applied(&file, &models));
        clear(&file).unwrap(); // 無ければ何もしない

        apply(&file, &models).unwrap();
        assert_eq!(is_managed(&file).unwrap(), Some(true));
        assert!(is_applied(&file, &models));
        assert!(!is_applied(&file, &engine)); // 別の保存先とは一致しない
        apply(&file, &engine).unwrap(); // 自分のファイルは更新できる
        assert!(is_applied(&file, &engine));
        assert!(!engine.join(format!("{FILE_NAME}.gazooo-tmp")).exists());

        clear(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn user_files_are_never_overwritten_or_removed() {
        let engine = tmpdir("foreign");
        let models = tmpdir("foreign-models");
        let file = engine.join(FILE_NAME);
        std::fs::write(&file, "a111:\n  base_path: x\n").unwrap();

        let err = apply(&file, &models).unwrap_err();
        assert!(format!("{err:?}").contains("MODELS_EXTRA_PATHS_FOREIGN"));
        assert!(clear(&file).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "a111:\n  base_path: x\n"
        );
    }

    #[test]
    fn apply_rejects_bad_dirs() {
        let engine = tmpdir("bad");
        let file = engine.join(FILE_NAME);
        assert!(apply(&file, Path::new("relative/dir")).is_err());
        assert!(apply(&file, &engine.join("missing")).is_err());
        assert!(!file.exists());
    }

    #[test]
    fn same_dir_ignores_separators() {
        assert!(same_dir(Path::new("C:\\a\\b\\"), Path::new("C:/a/b")));
        assert!(!same_dir(Path::new("C:\\a\\b"), Path::new("C:\\a\\c")));
    }

    #[test]
    fn legacy_marker_is_still_recognised_as_managed() {
        let d = std::env::temp_dir().join(format!("gazooo-ep-legacy-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        let old = d.join("old.yaml");
        std::fs::write(
            &old,
            "# managed by gazoo
gazoo:
  base_path: x
",
        )
        .unwrap();
        assert_eq!(is_managed(&old).unwrap(), Some(true));
        let other = d.join("other.yaml");
        std::fs::write(
            &other,
            "# my own settings
",
        )
        .unwrap();
        assert_eq!(is_managed(&other).unwrap(), Some(false));
        assert_eq!(is_managed(&d.join("none.yaml")).unwrap(), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
