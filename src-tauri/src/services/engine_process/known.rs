//! アプリが内蔵する「ComfyUI がよくある場所」の一覧からの候補検出(docs/22)。
//!
//! 起動中の ComfyUI がないとき、`main.py`・Python・base-directory の初期値を求める。ファイルを読むだけで、
//! 何も変更しない。深い再帰探索はせず、各場所は 1 つずつ確認する。基準のフォルダは引数で受け取る(テストのため)。

use std::path::{Path, PathBuf};

use super::config::Suggestions;

/// 基準となるフォルダ(Tauri の path API で求めたもの)。
#[derive(Debug, Clone, Default)]
pub struct KnownDirs {
    /// アプリが導入した ComfyUI のルート(`<app_data>/comfyui`。docs/23)。
    pub app_comfyui: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub desktop: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    pub local_data: Option<PathBuf>,
}

/// 見つけた ComfyUI の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    App,
    Manual,
    Portable,
    Desktop,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Manual => "manual",
            Self::Portable => "portable",
            Self::Desktop => "desktop",
        }
    }
}

fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

fn first_file(candidates: impl IntoIterator<Item = PathBuf>) -> Option<String> {
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

fn first_dir(candidates: impl IntoIterator<Item = PathBuf>) -> Option<String> {
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
}

/// 探す場所(上から順)。`comfy` は `main.py` があるフォルダ。
struct Spot {
    kind: Kind,
    comfy: PathBuf,
    python: Vec<PathBuf>,
    base: Vec<PathBuf>,
}

fn manual_spot(comfy: PathBuf) -> Spot {
    let parent = comfy.parent().map(Path::to_path_buf).unwrap_or_default();
    Spot {
        kind: Kind::Manual,
        python: vec![
            venv_python(&parent.join("venv")),
            venv_python(&parent.join(".venv")),
            venv_python(&comfy.join("venv")),
            venv_python(&comfy.join(".venv")),
        ],
        base: vec![parent.join("data")],
        comfy,
    }
}

fn spots(d: &KnownDirs) -> Vec<Spot> {
    let mut v = Vec::new();
    // アプリが導入したもの(最優先。08 と同じ並び)
    if let Some(root) = &d.app_comfyui {
        v.push(Spot {
            kind: Kind::App,
            comfy: root.join("ComfyUI"),
            python: vec![venv_python(&root.join("venv"))],
            base: vec![root.join("data")],
        });
    }
    // 手動インストール
    for (base, sub) in [
        (&d.home, vec!["ComfyUI"]),
        (&d.home, vec!["comfyui", "ComfyUI"]),
        (&d.documents, vec!["ComfyUI"]),
        (&d.desktop, vec!["ComfyUI"]),
    ] {
        if let Some(b) = base {
            v.push(manual_spot(sub.iter().fold(b.clone(), |p, s| p.join(s))));
        }
    }
    if cfg!(windows) {
        // Windows 版ポータブル(ComfyUI_windows_portable/ComfyUI と、隣の python_embeded)
        let mut portable_bases: Vec<PathBuf> = [&d.home, &d.desktop, &d.downloads, &d.documents]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        portable_bases.push(PathBuf::from("C:\\"));
        for b in portable_bases {
            let root = b.join("ComfyUI_windows_portable");
            v.push(Spot {
                kind: Kind::Portable,
                python: vec![root.join("python_embeded").join("python.exe")],
                base: vec![],
                comfy: root.join("ComfyUI"),
            });
        }
        v.push(manual_spot(PathBuf::from("C:\\ComfyUI")));
        // ComfyUI Desktop(アプリ本体は resources 配下、Python の環境と作業フォルダは Documents/ComfyUI)
        if let Some(local) = &d.local_data {
            v.push(desktop_spot(
                local
                    .join("Programs")
                    .join("ComfyUI")
                    .join("resources")
                    .join("ComfyUI"),
                d.documents.as_deref(),
            ));
        }
    } else if cfg!(target_os = "macos") {
        v.push(desktop_spot(
            PathBuf::from("/Applications/ComfyUI.app/Contents/Resources/ComfyUI"),
            d.documents.as_deref(),
        ));
    }
    v
}

fn desktop_spot(comfy: PathBuf, documents: Option<&Path>) -> Spot {
    let work = documents.map(|d| d.join("ComfyUI"));
    Spot {
        kind: Kind::Desktop,
        python: work.iter().map(|w| venv_python(&w.join(".venv"))).collect(),
        base: work.into_iter().collect(),
        comfy,
    }
}

/// 既知の場所から、最初に見つかった ComfyUI の候補を返す。見つからなければ空。
pub fn detect_known(dirs: &KnownDirs) -> Suggestions {
    for s in spots(dirs) {
        let main = s.comfy.join("main.py");
        if main.is_file() {
            return Suggestions {
                main_py: Some(main.to_string_lossy().into_owned()),
                python: first_file(s.python),
                base_directory: first_dir(s.base),
                source: Some(s.kind.label()),
            };
        }
    }
    Suggestions::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gazooo-known-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    fn dirs(root: &Path) -> KnownDirs {
        KnownDirs {
            app_comfyui: Some(root.join("appdata").join("comfyui")),
            home: Some(root.join("home")),
            documents: Some(root.join("home").join("Documents")),
            desktop: Some(root.join("home").join("Desktop")),
            downloads: Some(root.join("home").join("Downloads")),
            local_data: Some(root.join("local")),
        }
    }

    #[test]
    fn nothing_found_returns_empty() {
        let root = tmp("none");
        assert_eq!(detect_known(&dirs(&root)), Suggestions::default());
    }

    #[test]
    fn app_installed_comes_first_with_venv_and_data() {
        let root = tmp("app");
        let d = dirs(&root);
        let app = d.app_comfyui.clone().unwrap();
        touch(&app.join("ComfyUI").join("main.py"));
        touch(&venv_python(&app.join("venv")));
        std::fs::create_dir_all(app.join("data")).unwrap();
        // 手動インストールも存在するが、アプリの導入が優先される
        touch(&root.join("home").join("ComfyUI").join("main.py"));
        let s = detect_known(&d);
        assert_eq!(s.source, Some("app"));
        assert!(Path::new(s.main_py.as_deref().unwrap()).starts_with(&app));
        assert!(Path::new(s.python.as_deref().unwrap()).starts_with(&app));
        assert!(Path::new(s.base_directory.as_deref().unwrap()).starts_with(&app));
    }

    #[test]
    fn manual_install_finds_sibling_venv_and_data() {
        let root = tmp("manual");
        let d = dirs(&root);
        let base = root.join("home").join("comfyui");
        touch(&base.join("ComfyUI").join("main.py"));
        touch(&venv_python(&base.join("venv")));
        std::fs::create_dir_all(base.join("data")).unwrap();
        let s = detect_known(&d);
        assert_eq!(s.source, Some("manual"));
        assert!(Path::new(s.python.as_deref().unwrap()).starts_with(&base));
        assert!(s.base_directory.is_some());
    }

    #[test]
    fn python_and_data_are_left_empty_when_missing() {
        let root = tmp("nopy");
        let d = dirs(&root);
        touch(
            &root
                .join("home")
                .join("Documents")
                .join("ComfyUI")
                .join("main.py"),
        );
        let s = detect_known(&d);
        assert!(s.main_py.is_some());
        assert_eq!(s.python, None, "システムの python を勝手に選ばない");
        assert_eq!(s.base_directory, None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_portable_uses_python_embeded() {
        let root = tmp("portable");
        let d = dirs(&root);
        let p = root
            .join("home")
            .join("Desktop")
            .join("ComfyUI_windows_portable");
        touch(&p.join("ComfyUI").join("main.py"));
        touch(&p.join("python_embeded").join("python.exe"));
        let s = detect_known(&d);
        assert_eq!(s.source, Some("portable"));
        assert!(Path::new(s.python.as_deref().unwrap()).ends_with("python_embeded/python.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_desktop_app_uses_documents_venv() {
        let root = tmp("desktop");
        let d = dirs(&root);
        let comfy = root
            .join("local")
            .join("Programs")
            .join("ComfyUI")
            .join("resources")
            .join("ComfyUI");
        touch(&comfy.join("main.py"));
        let work = root.join("home").join("Documents").join("ComfyUI");
        touch(&venv_python(&work.join(".venv")));
        let s = detect_known(&d);
        assert_eq!(s.source, Some("desktop"));
        assert!(Path::new(s.python.as_deref().unwrap()).starts_with(&work));
        assert_eq!(
            s.base_directory.as_deref(),
            Some(work.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn japanese_and_spaces_in_paths_are_fine() {
        let root = tmp("jp").join("ユーザー 名");
        let d = dirs(&root);
        touch(&root.join("home").join("ComfyUI").join("main.py"));
        assert!(detect_known(&d).main_py.is_some());
    }
}
