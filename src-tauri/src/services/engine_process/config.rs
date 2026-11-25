//! ComfyUI 起動管理の設定・検証・起動引数の組み立て(docs/15 §3・§6・§8)。
//!
//! 設定は WebView から渡るため、Rust 側で必ず再検証する。引数は検証済みの値だけで組み立て、
//! 任意の追加引数や環境変数は受け付けない。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const DEFAULT_PORT: u16 = 8188;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedEngineConfig {
    /// `false` なら従来どおり(別途起動した ComfyUI に接続)。
    pub enabled: bool,
    /// ComfyUI の `main.py`(絶対パス)。
    pub main_py: String,
    /// Python の実行ファイル(venv のもの。絶対パス)。
    pub python: String,
    /// `--base-directory`(絶対パス)。`None` なら指定しない。
    pub base_directory: Option<String>,
    pub port: u16,
    /// アプリ起動時に自動で起動する。
    pub auto_start: bool,
    /// 接続が 3 分以上切れたら自動で再起動し、キューを実行し直す(docs/47。既定 OFF)。
    pub auto_recover: bool,
}

impl Default for ManagedEngineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            main_py: String::new(),
            python: String::new(),
            base_directory: None,
            port: DEFAULT_PORT,
            auto_start: false,
            auto_recover: false,
        }
    }
}

/// 起動に使う具体的なコマンド。テストでは任意のプログラムに差し替える。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}

fn invalid(field: &str, message: &str) -> AppError {
    AppError::validation(field, message)
}

impl ManagedEngineConfig {
    /// 保存時の検証。有効にする場合は、起動に必要な項目がすべて有効であること。
    /// 無効のままなら、入力済みの項目の形式だけを確認する(途中保存を許す)。
    pub fn validate(&self) -> AppResult<()> {
        if self.port < 1024 {
            return Err(invalid("port", "ポートは 1024〜65535 で指定してください"));
        }
        check_file("mainPy", &self.main_py, self.enabled)?;
        check_file("python", &self.python, self.enabled)?;
        if let Some(dir) = self.base_directory.as_deref().filter(|d| !d.is_empty()) {
            let p = Path::new(dir);
            if !p.is_absolute() {
                return Err(invalid("baseDirectory", "絶対パスで指定してください"));
            }
            if self.enabled && !p.is_dir() {
                return Err(invalid("baseDirectory", "フォルダが見つかりません"));
            }
        }
        Ok(())
    }

    /// 起動管理が有効のとき、接続先となる URL(ローカル固定)。
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 検証済みの設定から起動コマンドを組み立てる(08 の `start` スクリプトと同じ引数)。
    pub fn launch_spec(&self) -> AppResult<LaunchSpec> {
        if !self.enabled {
            return Err(AppError::engine_proc(
                "ENGINE_NOT_MANAGED",
                "ComfyUI の起動管理が有効ではありません",
            ));
        }
        self.validate()?;
        let main = PathBuf::from(&self.main_py);
        let mut args: Vec<OsString> = vec![
            main.clone().into_os_string(),
            "--listen".into(),
            "127.0.0.1".into(),
            "--port".into(),
            self.port.to_string().into(),
        ];
        if let Some(dir) = self.base_directory.as_deref().filter(|d| !d.is_empty()) {
            args.push("--base-directory".into());
            args.push(dir.into());
        }
        if supports_disable_dynamic_vram(&main) {
            args.push(DISABLE_DYNAMIC_VRAM.into());
        }
        args.push("--disable-auto-launch".into());
        Ok(LaunchSpec {
            program: PathBuf::from(&self.python),
            args,
            cwd: main.parent().map(Path::to_path_buf),
        })
    }
}

/// 動的 VRAM ロードの無効化(docs/46 §14)。生成を繰り返すと ComfyUI が応答しなくなる不具合を避ける。
const DISABLE_DYNAMIC_VRAM: &str = "--disable-dynamic-vram";

/// この ComfyUI が `--disable-dynamic-vram` に対応しているか。`main.py` と同じ場所の
/// `comfy/cli_args.py` に引数名があるかを見る(プロセスは起動しない)。読めなければ付けない
/// (古い版は未知の引数で起動に失敗するため)。
fn supports_disable_dynamic_vram(main_py: &Path) -> bool {
    let Some(dir) = main_py.parent() else {
        return false;
    };
    std::fs::read_to_string(dir.join("comfy").join("cli_args.py"))
        .is_ok_and(|s| s.contains(DISABLE_DYNAMIC_VRAM))
}

fn check_file(field: &str, value: &str, required: bool) -> AppResult<()> {
    if value.is_empty() {
        return if required {
            Err(invalid(field, "指定してください"))
        } else {
            Ok(())
        };
    }
    let p = Path::new(value);
    if !p.is_absolute() {
        return Err(invalid(field, "絶対パスで指定してください"));
    }
    if required && !p.is_file() {
        return Err(invalid(field, "ファイルが見つかりません"));
    }
    Ok(())
}

/// 画面に出す、パスの入力候補(確定はユーザー)。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_py: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_directory: Option<String>,
    /// 候補の出どころ(`running` / `app` / `manual` / `portable` / `desktop` / `dev`)。表示用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
}

/// 起動中の ComfyUI の `argv`(`/system_stats`)から候補を求める。
pub fn suggest(argv: &[String]) -> Suggestions {
    let mut out = Suggestions::default();
    let main = argv
        .first()
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf);
    let mut it = argv.iter().skip(1);
    while let Some(a) = it.next() {
        let v = if a == "--base-directory" {
            it.next().cloned()
        } else {
            a.strip_prefix("--base-directory=").map(str::to_string)
        };
        if let Some(v) = v.filter(|v| Path::new(v).is_absolute()) {
            out.base_directory = Some(v);
        }
    }
    if let Some(main) = main {
        // 08 のレイアウト: <root>/ComfyUI/main.py と <root>/venv
        let py = main.parent().and_then(Path::parent).map(|root| {
            if cfg!(windows) {
                root.join("venv").join("Scripts").join("python.exe")
            } else {
                root.join("venv").join("bin").join("python")
            }
        });
        out.python = py
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned());
        out.main_py = Some(main.to_string_lossy().into_owned());
        out.source = Some("running");
    }
    out
}

/// docs/08 のレイアウト(`<root>/ComfyUI/main.py`、`<root>/venv`、`<root>/data`)から候補を求める。
fn suggest_from_root(root: &Path, main: &Path) -> Suggestions {
    let py = if cfg!(windows) {
        root.join("venv").join("Scripts").join("python.exe")
    } else {
        root.join("venv").join("bin").join("python")
    };
    let data = root.join("data");
    Suggestions {
        main_py: Some(main.to_string_lossy().into_owned()),
        python: py.is_file().then(|| py.to_string_lossy().into_owned()),
        base_directory: data.is_dir().then(|| data.to_string_lossy().into_owned()),
        source: Some("dev"),
    }
}

/// 起動中でないときの初期値用に、既知の場所から ComfyUI を探す(docs/15 §9)。
/// `starts` の各パスとその親ディレクトリ(最大 8 階層)で、`comfyui/ComfyUI/main.py` と
/// `ComfyUI/main.py` を探す。最初に見つかったものを返す。
pub fn detect(starts: &[PathBuf]) -> Suggestions {
    for start in starts {
        for dir in start.ancestors().take(8) {
            for root in [dir.join("comfyui"), dir.to_path_buf()] {
                let main = root.join("ComfyUI").join("main.py");
                if main.is_file() {
                    return suggest_from_root(&root, &main);
                }
            }
        }
    }
    Suggestions::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gazooo-cfg-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn valid() -> (ManagedEngineConfig, PathBuf) {
        let d = tmp("ok");
        let main = d.join("main.py");
        let py = d.join("python.exe");
        std::fs::write(&main, "").unwrap();
        std::fs::write(&py, "").unwrap();
        let cfg = ManagedEngineConfig {
            enabled: true,
            main_py: main.to_string_lossy().into_owned(),
            python: py.to_string_lossy().into_owned(),
            base_directory: Some(d.to_string_lossy().into_owned()),
            port: 8189,
            auto_start: false,
            auto_recover: false,
        };
        (cfg, d)
    }

    #[test]
    fn auto_recover_defaults_to_off_for_existing_settings() {
        let old = r#"{"enabled":true,"mainPy":"","python":"","baseDirectory":null,"port":8188,"autoStart":true}"#;
        let cfg: ManagedEngineConfig = serde_json::from_str(old).unwrap();
        assert!(cfg.auto_start && !cfg.auto_recover);
    }

    #[test]
    fn disabled_config_allows_partial_input_but_not_bad_shapes() {
        assert!(ManagedEngineConfig::default().validate().is_ok());
        let mut c = ManagedEngineConfig {
            main_py: "relative/main.py".into(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
        c.main_py.clear();
        c.port = 80;
        assert!(c.validate().is_err());
    }

    #[test]
    fn enabled_config_requires_existing_files() {
        let (cfg, d) = valid();
        assert!(cfg.validate().is_ok());

        let mut c = cfg.clone();
        c.python = d.join("missing.exe").to_string_lossy().into_owned();
        assert_eq!(c.validate().unwrap_err().code(), "VALIDATION");

        let mut c = cfg.clone();
        c.main_py.clear();
        assert!(c.validate().is_err());

        let mut c = cfg.clone();
        c.base_directory = Some(d.join("nodir").to_string_lossy().into_owned());
        assert!(c.validate().is_err());

        let mut c = cfg;
        c.base_directory = Some("rel".into());
        assert!(c.validate().is_err());
    }

    #[test]
    fn launch_args_are_fixed_and_local_only() {
        let (cfg, d) = valid();
        let spec = cfg.launch_spec().unwrap();
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[1..5], ["--listen", "127.0.0.1", "--port", "8189"]);
        assert!(args.contains(&"--base-directory".to_string()));
        assert_eq!(args.last().unwrap(), "--disable-auto-launch");
        assert_eq!(spec.program, PathBuf::from(&cfg.python));
        assert_eq!(spec.cwd.as_deref(), Some(d.as_path()));
        assert_eq!(cfg.url(), "http://127.0.0.1:8189");

        // base-directory なしなら引数に含めない
        let mut c = cfg;
        c.base_directory = None;
        let spec = c.launch_spec().unwrap();
        assert!(!spec.args.iter().any(|a| a == "--base-directory"));

        // 無効のときは起動できない
        c.enabled = false;
        assert_eq!(c.launch_spec().unwrap_err().code(), "ENGINE_NOT_MANAGED");
    }

    #[test]
    fn disable_dynamic_vram_is_added_only_when_comfyui_supports_it() {
        let (cfg, d) = valid();
        let has = |cfg: &ManagedEngineConfig| {
            cfg.launch_spec()
                .unwrap()
                .args
                .iter()
                .any(|a| a == DISABLE_DYNAMIC_VRAM)
        };
        // cli_args.py が無い・引数名が無い → 付けない(古い版は未知の引数で起動に失敗する)
        assert!(!has(&cfg));
        std::fs::create_dir_all(d.join("comfy")).unwrap();
        std::fs::write(
            d.join("comfy").join("cli_args.py"),
            "--disable-smart-memory",
        )
        .unwrap();
        assert!(!has(&cfg));
        // 対応あり → 付く。--disable-auto-launch の前に入り、他の引数は変わらない
        std::fs::write(
            d.join("comfy").join("cli_args.py"),
            r#"parser.add_argument("--disable-dynamic-vram", action="store_true")"#,
        )
        .unwrap();
        assert!(has(&cfg));
        let spec = cfg.launch_spec().unwrap();
        assert_eq!(spec.args.last().unwrap(), "--disable-auto-launch");
    }

    #[test]
    fn suggestions_come_from_argv_and_the_08_layout() {
        let root = tmp("layout");
        let main = root.join("ComfyUI").join("main.py");
        let py = if cfg!(windows) {
            root.join("venv").join("Scripts").join("python.exe")
        } else {
            root.join("venv").join("bin").join("python")
        };
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();
        let data = root.join("data");
        let argv = vec![
            main.to_string_lossy().into_owned(),
            "--base-directory".to_string(),
            data.to_string_lossy().into_owned(),
        ];
        let s = suggest(&argv);
        assert_eq!(s.main_py.as_deref(), Some(main.to_string_lossy().as_ref()));
        assert_eq!(s.python.as_deref(), Some(py.to_string_lossy().as_ref()));
        assert_eq!(
            s.base_directory.as_deref(),
            Some(data.to_string_lossy().as_ref())
        );

        // 相対パス・不明なものは候補にしない
        let s = suggest(&["main.py".to_string(), "--base-directory=data".to_string()]);
        assert_eq!(s, Suggestions::default());
        assert_eq!(suggest(&[]), Suggestions::default());
    }

    #[test]
    fn detect_finds_the_08_layout_from_a_nested_start() {
        let root = tmp("detect");
        let main = root.join("comfyui").join("ComfyUI").join("main.py");
        std::fs::create_dir_all(main.parent().unwrap()).unwrap();
        std::fs::write(&main, "").unwrap();
        std::fs::create_dir_all(root.join("comfyui").join("data")).unwrap();
        let start = root
            .join("src-tauri")
            .join("target")
            .join("release")
            .join("gazooo.exe");
        let s = detect(&[start]);
        assert_eq!(s.main_py.as_deref(), Some(main.to_string_lossy().as_ref()));
        assert!(s.base_directory.is_some());
        assert_eq!(detect(&[tmp("detect-empty")]), Suggestions::default());
    }
}
