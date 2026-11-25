//! 導入するソフトウェアの固定マニフェスト(docs/23 §3)。
//!
//! バージョン・コミット・SHA-256 はここに固定し、`tools/comfyui/versions.env`(08)と一致することを
//! テストで検証する。取得元は許可ホストの https のみ。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::models::download::DownloadPolicy;

const EMBEDDED: &str = include_str!("manifest.json");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonDist {
    pub version: String,
    #[serde(rename = "windows-x64")]
    pub windows_x64: Source,
    #[serde(rename = "macos-arm64")]
    pub macos_arm64: Source,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComfyuiPin {
    pub tag: String,
    /// 記録用(versions.env との一致と、URL への固定をテストで確認する)。
    #[allow(dead_code)]
    pub commit: String,
    pub license: String,
    pub source: Source,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorchPin {
    pub version: String,
    pub torchvision: String,
    pub torchaudio: String,
    pub windows_index: String,
    pub license: String,
    pub download_mb: u64,
    pub disk_mb: u64,
}

/// 親と一緒に導入する、同じ選択に属する追加のノード(顔補正の Subpack)。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraNode {
    pub name: String,
    pub repo: String,
    #[allow(dead_code)]
    pub commit: String,
    pub expect_node: String,
    pub source: Source,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: String,
    pub name: String,
    pub repo: String,
    #[allow(dead_code)]
    pub commit: String,
    pub license: String,
    /// 導入後に `/object_info` に現れるべきノード名(検証に使う)。
    pub expect_node: String,
    pub download_mb: u64,
    pub disk_mb: u64,
    pub source: Source,
    #[serde(default)]
    pub extra: Option<ExtraNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub manifest_version: String,
    pub allowed_hosts: Vec<String>,
    pub python: PythonDist,
    pub comfyui: ComfyuiPin,
    pub torch: TorchPin,
    pub components: Vec<Component>,
}

/// 対応するプラットフォーム。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Platform {
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "macos-arm64")]
    MacosArm64,
}

impl Platform {
    /// このビルドの対象。非対応(Linux・Intel Mac など)は `None`。
    pub fn current() -> Option<Self> {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Some(Self::WindowsX64)
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(Self::MacosArm64)
        } else {
            None
        }
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::internal(format!("engine manifest: {}", message.into()))
}

impl Manifest {
    pub fn embedded() -> AppResult<Self> {
        let m: Self = serde_json::from_str(EMBEDDED)?;
        m.validate()?;
        Ok(m)
    }

    pub fn policy(&self) -> DownloadPolicy {
        DownloadPolicy::with_hosts(self.allowed_hosts.clone())
    }

    pub fn python_source(&self, platform: Platform) -> &Source {
        match platform {
            Platform::WindowsX64 => &self.python.windows_x64,
            Platform::MacosArm64 => &self.python.macos_arm64,
        }
    }

    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    /// 取得するすべてのソース(検証用)。
    fn sources(&self) -> Vec<(&str, &Source)> {
        let mut v = vec![
            ("python windows", &self.python.windows_x64),
            ("python macos", &self.python.macos_arm64),
            ("comfyui", &self.comfyui.source),
        ];
        for c in &self.components {
            v.push((c.id.as_str(), &c.source));
            if let Some(e) = &c.extra {
                v.push((e.name.as_str(), &e.source));
            }
        }
        v
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != 1 {
            return Err(invalid("未対応のスキーマ版です"));
        }
        let policy = self.policy();
        for (name, s) in self.sources() {
            let url =
                reqwest::Url::parse(&s.url).map_err(|_| invalid(format!("{name}: URL が不正")))?;
            if url.scheme() != "https" || !policy.url_allowed(&url) {
                return Err(invalid(format!("{name}: 許可されていない取得元です")));
            }
            let hex_ok = s.sha256.len() == 64
                && s.sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !hex_ok {
                return Err(invalid(format!("{name}: SHA-256 は小文字 16 進 64 桁")));
            }
            if s.size_bytes == 0 {
                return Err(invalid(format!("{name}: サイズが 0")));
            }
        }
        let url = reqwest::Url::parse(&self.torch.windows_index)
            .map_err(|_| invalid("PyTorch のインデックス URL が不正"))?;
        if url.scheme() != "https" || !policy.url_allowed(&url) {
            return Err(invalid("PyTorch のインデックスが許可されていません"));
        }
        let mut ids: Vec<&str> = self.components.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != self.components.len() {
            return Err(invalid("コンポーネント ID が重複しています"));
        }
        // ディレクトリ名に使うので、安全な文字だけ
        if self.components.iter().any(|c| {
            !c.id
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        }) {
            return Err(invalid("コンポーネント ID に使えない文字があります"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tools/comfyui/versions.env` の `KEY=value` を読む。
    fn versions_env() -> std::collections::HashMap<String, String> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/comfyui/versions.env");
        std::fs::read_to_string(path)
            .expect("versions.env")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    #[test]
    fn embedded_manifest_is_valid() {
        Manifest::embedded().unwrap();
    }

    /// 生成物(docs/41)の `_generated` を含んでいても読める。将来 `deny_unknown_fields` を付けて壊さないための確認。
    #[test]
    fn generated_note_is_accepted() {
        let raw: serde_json::Value = serde_json::from_str(EMBEDDED).unwrap();
        assert!(raw.get("_generated").is_some_and(|v| v.is_string()));
        Manifest::embedded().unwrap();
    }

    #[test]
    fn matches_versions_env() {
        let m = Manifest::embedded().unwrap();
        let v = versions_env();
        assert_eq!(m.comfyui.tag, v["COMFYUI_TAG"]);
        assert_eq!(m.torch.version, v["TORCH_VERSION"]);
        assert_eq!(m.torch.torchvision, v["TORCHVISION_VERSION"]);
        assert_eq!(m.torch.torchaudio, v["TORCHAUDIO_VERSION"]);
        assert_eq!(m.torch.windows_index, v["TORCH_CUDA_INDEX"]);
        assert!(
            m.python
                .version
                .starts_with(&format!("{}.", v["PYTHON_VERSION"]))
        );
        assert_eq!(m.component("gguf").unwrap().commit, v["GGUF_NODE_COMMIT"]);
        let face = m.component("face").unwrap();
        assert_eq!(face.commit, v["IMPACT_PACK_COMMIT"]);
        assert_eq!(
            face.extra.as_ref().unwrap().commit,
            v["IMPACT_SUBPACK_COMMIT"]
        );
        assert_eq!(
            m.component("controlnet").unwrap().commit,
            v["CONTROLNET_AUX_COMMIT"]
        );
        assert_eq!(
            m.component("tagger").unwrap().commit,
            v["WD14_TAGGER_COMMIT"]
        );
    }

    #[test]
    fn source_urls_are_pinned_to_the_commit() {
        let m = Manifest::embedded().unwrap();
        assert!(m.comfyui.source.url.contains(&m.comfyui.commit));
        for c in &m.components {
            assert!(c.source.url.contains(&c.commit), "{}", c.id);
            if let Some(e) = &c.extra {
                assert!(e.source.url.contains(&e.commit), "{}", e.name);
            }
        }
    }

    #[test]
    fn rejects_disallowed_host_and_bad_hash() {
        let mut m = Manifest::embedded().unwrap();
        m.comfyui.source.url = "https://evil.example.com/x.zip".into();
        assert!(m.validate().is_err());
        let mut m = Manifest::embedded().unwrap();
        m.comfyui.source.url = m.comfyui.source.url.replace("https://", "http://");
        assert!(m.validate().is_err());
        let mut m = Manifest::embedded().unwrap();
        m.comfyui.source.sha256 = "ABC".into();
        assert!(m.validate().is_err());
    }
}
