//! モデルカタログ(docs/12-model-downloader.md §4)。
//!
//! アプリに同梱した JSON(`resources/models/catalog.json`)を読み込み、**構造と安全性を検証**する。
//! 取得元・ハッシュ・配置先・ファイル名はすべてここで固定され、実行時に外部から差し替えられない。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::download::DownloadPolicy;
use crate::error::{AppError, AppResult};

const EMBEDDED: &str = include_str!("../../../resources/models/catalog.json");
const SCHEMA_VERSION: u32 = 2;
/// 配置を許すファイルの拡張子(ComfyUI のモデルとして読まれる形式)。
const ALLOWED_EXTENSIONS: &[&str] = &["safetensors", "gguf", "pt", "ckpt", "pth"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    pub ja: String,
    pub en: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Checkpoint,
    Controlnet,
    Detector,
    /// Flux の本体(GGUF)・テキストエンコーダ・VAE。
    Flux,
    /// Anima の本体・テキストエンコーダ・VAE(docs/17)。
    Anima,
    /// Qwen-Image 2.1 の本体・テキストエンコーダ・VAE。
    #[serde(rename = "qwen_image")]
    QwenImage,
    /// Krea 2 の本体・テキストエンコーダ・VAE。
    Krea2,
}

/// 生成画面で選べる拡散モデル本体(`unet/` に置くもの)の種別(docs/17 §5)。
/// 以前の「`unet/` に置くもの = Flux」という推定の置き換え。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyKind {
    FluxGguf,
    Anima,
    QwenImage,
    Krea2,
}

/// 「まとめて取得」で一緒に取得する、必要なファイル(別ファミリーの variant)への参照(docs/17 §5、Q-70)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub family_id: String,
    pub variant_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaseModel {
    Sd15,
    Sdxl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    Fp8,
    Fp16,
    Fp32,
}

/// `models/` 直下からの配置先。列挙にして、カタログから任意のパスを指定できないようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DestDir {
    #[serde(rename = "checkpoints")]
    Checkpoints,
    #[serde(rename = "controlnet")]
    Controlnet,
    #[serde(rename = "ultralytics/bbox")]
    UltralyticsBbox,
    /// Flux の拡散モデル(GGUF)。ComfyUI-GGUF が `models/unet` から読む。
    #[serde(rename = "unet")]
    Unet,
    /// テキストエンコーダ(CLIP / T5)。
    #[serde(rename = "clip")]
    Clip,
    #[serde(rename = "vae")]
    Vae,
    /// LoRA(顔の入れ替え専用の 1 件。docs/42 §7)。
    #[serde(rename = "loras")]
    Loras,
}

impl DestDir {
    /// `models/` からのパス要素(区切り文字を直書きしない)。
    pub fn components(self) -> &'static [&'static str] {
        match self {
            Self::Checkpoints => &["checkpoints"],
            Self::Controlnet => &["controlnet"],
            Self::UltralyticsBbox => &["ultralytics", "bbox"],
            Self::Unet => &["unet"],
            Self::Clip => &["clip"],
            Self::Vae => &["vae"],
            Self::Loras => &["loras"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommercialUse {
    Flag(bool),
    /// `"see-license"`(ライセンス本文を確認する)。
    Note(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
    pub name: String,
    pub url: String,
    pub commercial_use: CommercialUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub id: String,
    pub label: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model: Option<BaseModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<Precision>,
    #[serde(default)]
    pub recommended: bool,
    pub source: Source,
    pub size_bytes: u64,
    pub sha256: String,
    pub dest_dir: DestDir,
    pub file_name: String,
    pub license: License,
    #[serde(default)]
    pub requires_consent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Localized>,
}

/// 取得ページの用途フィルタ用のタグ(docs/16 §3.1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tag {
    Anime,
}

/// 絵柄の傾向(docs/18 §3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Anime,
    Realistic,
    Illustration,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptKind {
    Tags,
    Natural,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speed {
    Fast,
    Medium,
    Slow,
}

/// 規制の強さ。配布元の記載で確認できたものだけ段階を付け、確認できなければ `unknown`(docs/18 §10)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModerationLevel {
    Strict,
    Moderate,
    Loose,
    Unknown,
}

/// 規制の強さと、その根拠の 1 行(docs/18 §10.2)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moderation {
    pub level: ModerationLevel,
    pub note: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedList {
    pub ja: Vec<String>,
    pub en: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStyle {
    pub kind: PromptKind,
    /// プロンプトの推奨言語(docs/11 §14)。未指定は英語(安全側)。
    #[serde(default)]
    pub language: PromptLanguage,
    pub hint: Localized,
}

/// プロンプトに使う言語。`En` は英語推奨(日本語は効きにくい)、`Multilingual` は日本語でもよい。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptLanguage {
    #[default]
    En,
    Multilingual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// 「推奨設定を適用」の値(docs/18 §5)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommended {
    pub steps: u32,
    pub cfg: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<String>,
    pub size: Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    /// 必要な VRAM の最小の目安(GB)。
    pub vram_gb: u32,
    pub speed: Speed,
}

/// モデルの説明(docs/18 §3)。生成画面の説明パネル・取得ページ・AI のモデルガイドの単一の情報源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub summary: Localized,
    pub style: Vec<Style>,
    pub best_for: LocalizedList,
    pub weak_at: LocalizedList,
    pub prompt_style: PromptStyle,
    pub recommended: Recommended,
    pub resources: Resources,
    pub moderation: Moderation,
    pub source_note: Localized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Family {
    pub id: String,
    pub category: Category,
    pub name: Localized,
    pub description: Localized,
    #[serde(default)]
    pub used_by: Vec<String>,
    /// 用途フィルタ用(docs/16 §3.1)。
    #[serde(default)]
    pub tags: Vec<Tag>,
    /// 拡散モデル本体(`unet/`)のファミリーの種別。生成画面で選べるモデルで、`profile` が必須。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FamilyKind>,
    /// 一緒に取得が必要なファイル(テキストエンコーダ・VAE)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// モデルの説明。生成画面で選べるモデル(チェックポイント・Flux 本体)では必須(docs/18 §2)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    pub variants: Vec<Variant>,
}

impl Family {
    /// 生成画面のモデル選択に出るファミリー(チェックポイント、拡散モデル本体)。
    pub fn is_selectable_model(&self) -> bool {
        self.category == Category::Checkpoint || self.kind.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub families: Vec<Family>,
}

impl Catalog {
    /// 同梱カタログを読み込んで検証する。
    pub fn embedded(policy: &DownloadPolicy) -> AppResult<Self> {
        Self::parse(EMBEDDED, policy)
    }

    pub fn parse(json: &str, policy: &DownloadPolicy) -> AppResult<Self> {
        let catalog: Catalog = serde_json::from_str(json)
            .map_err(|e| AppError::internal(format!("モデルカタログを読めません: {e}")))?;
        catalog.validate(policy)?;
        Ok(catalog)
    }

    pub fn find(&self, family_id: &str, variant_id: &str) -> Option<(&Family, &Variant)> {
        let family = self.families.iter().find(|f| f.id == family_id)?;
        let variant = family.variants.iter().find(|v| v.id == variant_id)?;
        Some((family, variant))
    }

    fn validate(&self, policy: &DownloadPolicy) -> AppResult<()> {
        let bad = |msg: String| {
            Err(AppError::internal(format!(
                "モデルカタログが不正です: {msg}"
            )))
        };
        if self.schema_version != SCHEMA_VERSION {
            return bad(format!(
                "schemaVersion {} は未対応です",
                self.schema_version
            ));
        }
        let mut family_ids = HashSet::new();
        // 配置先ごとの SHA-256。同じファイルを複数のファミリーが共有するのは許す(docs/17 §5)
        let mut targets: HashMap<(DestDir, String), &str> = HashMap::new();
        for f in &self.families {
            if f.id.is_empty() || !family_ids.insert(f.id.as_str()) {
                return bad(format!("ファミリー ID が空か重複しています: {}", f.id));
            }
            if f.variants.is_empty() {
                return bad(format!("{}: variant がありません", f.id));
            }
            if let Err(why) = check_kind(f) {
                return bad(format!("{}: {why}", f.id));
            }
            if let Err(why) = check_profile(f) {
                return bad(format!("{}: {why}", f.id));
            }
            let mut variant_ids = HashSet::new();
            let mut recommended = HashSet::new();
            for v in &f.variants {
                let at = format!("{}/{}", f.id, v.id);
                if v.id.is_empty() || !variant_ids.insert(v.id.as_str()) {
                    return bad(format!("{at}: variant ID が空か重複しています"));
                }
                if v.recommended && !recommended.insert(v.base_model) {
                    return bad(format!("{at}: 同じベースモデルに推奨が複数あります"));
                }
                if v.size_bytes == 0 {
                    return bad(format!("{at}: sizeBytes が 0 です"));
                }
                if !is_sha256(&v.sha256) {
                    return bad(format!(
                        "{at}: sha256 は小文字 16 進 64 桁で指定してください"
                    ));
                }
                if let Err(why) = check_file_name(&v.file_name) {
                    return bad(format!("{at}: fileName が不正です({why})"));
                }
                let slot = (v.dest_dir, v.file_name.to_ascii_lowercase());
                match targets.get(&slot) {
                    Some(sha) if *sha != v.sha256 => {
                        return bad(format!(
                            "{at}: 配置先のファイルが、内容(SHA-256)の違う別の variant と重複しています"
                        ));
                    }
                    Some(_) => {}
                    None => {
                        targets.insert(slot, v.sha256.as_str());
                    }
                }
                match reqwest::Url::parse(&v.source.url) {
                    Ok(url) if policy.url_allowed(&url) => {}
                    _ => return bad(format!("{at}: 取得元 URL が許可されていません")),
                }
                if let CommercialUse::Note(n) = &v.license.commercial_use
                    && n != "see-license"
                {
                    return bad(format!("{at}: commercialUse が不正です"));
                }
            }
        }
        // 「まとめて取得」の参照先が存在すること
        for f in &self.families {
            for r in &f.requires {
                if r.family_id == f.id {
                    return bad(format!("{}: requires が自分自身を指しています", f.id));
                }
                if self.find(&r.family_id, &r.variant_id).is_none() {
                    return bad(format!(
                        "{}: requires の参照先がありません({}/{})",
                        f.id, r.family_id, r.variant_id
                    ));
                }
            }
        }
        Ok(())
    }
}

/// `kind` と配置先・カテゴリの整合性(docs/17 §5)。拡散モデル本体(`unet/`)だけが `kind` を持つ。
fn check_kind(f: &Family) -> Result<(), String> {
    let has_unet = f.variants.iter().any(|v| v.dest_dir == DestDir::Unet);
    let all_unet = f.variants.iter().all(|v| v.dest_dir == DestDir::Unet);
    match f.kind {
        Some(kind) => {
            if !all_unet {
                return Err(
                    "kind を持つファミリーの variant は、すべて unet/ に置くものにしてください"
                        .into(),
                );
            }
            let expected = match kind {
                FamilyKind::FluxGguf => Category::Flux,
                FamilyKind::Anima => Category::Anima,
                FamilyKind::QwenImage => Category::QwenImage,
                FamilyKind::Krea2 => Category::Krea2,
            };
            if f.category != expected {
                return Err("kind とカテゴリが一致しません".into());
            }
        }
        None if has_unet => {
            return Err("unet/ に置く拡散モデルには kind が必要です".into());
        }
        None => {}
    }
    if f.kind.is_none() && !f.requires.is_empty() {
        return Err("requires は拡散モデル本体(kind を持つファミリー)だけに付けられます".into());
    }
    Ok(())
}

/// プロフィールとタグの整合性(docs/18 §2〜3)。
fn check_profile(f: &Family) -> Result<(), String> {
    let Some(p) = &f.profile else {
        if f.is_selectable_model() {
            return Err("profile がありません(選べるモデルには必須です)".into());
        }
        return Ok(());
    };
    // 規制の強さの根拠(日英)は、段階が確定しているときだけ必須
    if p.moderation.level != ModerationLevel::Unknown
        && (p.moderation.note.ja.trim().is_empty() || p.moderation.note.en.trim().is_empty())
    {
        return Err("profile.moderation.note(日英)が空です".into());
    }
    if f.tags.contains(&Tag::Anime) != p.style.contains(&Style::Anime) {
        return Err("anime タグと profile.style(anime)が一致しません".into());
    }
    let text_ok = |l: &Localized| !l.ja.trim().is_empty() && !l.en.trim().is_empty();
    if !text_ok(&p.summary) || !text_ok(&p.prompt_style.hint) || !text_ok(&p.source_note) {
        return Err("profile の説明文(日英)が空です".into());
    }
    if p.style.is_empty() {
        return Err("profile.style が空です".into());
    }
    for (name, l) in [("bestFor", &p.best_for), ("weakAt", &p.weak_at)] {
        let ok =
            |v: &Vec<String>| (1..=4).contains(&v.len()) && v.iter().all(|s| !s.trim().is_empty());
        if !ok(&l.ja) || !ok(&l.en) || l.ja.len() != l.en.len() {
            return Err(format!(
                "profile.{name} は日英とも 1〜4 件(同数)で指定してください"
            ));
        }
    }
    let r = &p.recommended;
    let dim_ok = |n: u32| (256..=2048).contains(&n) && n.is_multiple_of(64);
    if !(1..=150).contains(&r.steps)
        || !(0.0..=30.0).contains(&r.cfg)
        || !dim_ok(r.size.width)
        || !dim_ok(r.size.height)
    {
        return Err("profile.recommended の値が範囲外です".into());
    }
    if p.resources.vram_gb == 0 {
        return Err("profile.resources.vramGb が 0 です".into());
    }
    Ok(())
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// 配置するファイル名の安全性。区切り文字・`..`・Windows の代替データストリーム(`:`)などを拒否する。
fn check_file_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("空");
    }
    if name.starts_with('.') || name.contains("..") {
        return Err("先頭のドットまたは ..");
    }
    if name.chars().any(|c| {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
    }) {
        return Err("使えない文字");
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext {
        Some(e) if ALLOWED_EXTENSIONS.contains(&e.as_str()) => Ok(()),
        _ => Err("未対応の拡張子"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> DownloadPolicy {
        DownloadPolicy::production()
    }

    #[test]
    fn embedded_catalog_is_valid() {
        let c = Catalog::embedded(&policy()).unwrap();
        assert!(c.families.len() >= 6);
        // 初期カタログの要点(docs/12 §4.2)
        let (_, v) = c.find("controlnet-canny", "sdxl-xinsir").unwrap();
        assert_eq!(v.base_model, Some(BaseModel::Sdxl));
        assert!(v.file_name.contains("canny"));
        let (_, face) = c.find("detector-face", "yolov9c").unwrap();
        assert_eq!(face.dest_dir, DestDir::UltralyticsBbox);
        assert!(face.note.is_some(), "pickle 形式の注意書きがある");
    }

    #[test]
    fn flux_set_has_everything_the_generate_screen_needs() {
        // Flux(GGUF)の生成には、本体・テキストエンコーダ 2 種・VAE が要る(docs/12 §4.2)
        let c = Catalog::embedded(&policy()).unwrap();
        let place = |family: &str| -> Vec<(DestDir, String, bool, bool)> {
            c.families
                .iter()
                .find(|f| f.id == family && f.category == Category::Flux)
                .unwrap_or_else(|| panic!("{family} がカタログにない"))
                .variants
                .iter()
                .map(|v| {
                    (
                        v.dest_dir,
                        v.file_name.clone(),
                        v.requires_consent,
                        v.recommended,
                    )
                })
                .collect()
        };

        for (family, quant_prefix) in [
            ("flux-schnell-gguf", "flux1-schnell-"),
            ("flux-dev-gguf", "flux1-dev-"),
        ] {
            let v = place(family);
            assert!(v.iter().all(|(d, f, _, _)| *d == DestDir::Unet
                && f.starts_with(quant_prefix)
                && f.ends_with(".gguf")));
            assert_eq!(v.iter().filter(|x| x.3).count(), 1, "推奨はちょうど 1 つ");
        }
        // schnell(Apache-2.0)は同意不要、dev(非商用)は同意が必要
        assert!(place("flux-schnell-gguf").iter().all(|x| !x.2));
        assert!(place("flux-dev-gguf").iter().all(|x| x.2));

        assert_eq!(place("flux-clip-l")[0].0, DestDir::Clip);
        assert!(
            place("flux-t5xxl")
                .iter()
                .all(|x| x.0 == DestDir::Clip && x.1.starts_with("t5xxl"))
        );
        let vae = place("flux-vae");
        assert_eq!(
            (vae[0].0, vae[0].1.as_str()),
            (DestDir::Vae, "ae.safetensors")
        );
    }

    #[test]
    fn flux_vae_matches_the_official_black_forest_labs_file() {
        // 公式(gated)の ae.safetensors と同一の SHA-256。認証不要のミラーから取得する根拠(docs/12 §4.2)
        let c = Catalog::embedded(&policy()).unwrap();
        let (_, v) = c.find("flux-vae", "ae").unwrap();
        assert_eq!(
            v.sha256,
            "afc8e28272cd15db3919bacdb6918ce9c1ed22e96cb12c4d5ed0fba823529e38"
        );
        assert_eq!(v.size_bytes, 335_304_388);
    }

    #[test]
    fn controlnet_file_names_carry_the_preprocessor_for_auto_switch() {
        // 前処理に応じた ControlNet の自動切り替え(docs/10 §8)が、ファイル名の語に依存する
        let c = Catalog::embedded(&policy()).unwrap();
        for f in c
            .families
            .iter()
            .filter(|f| f.category == Category::Controlnet)
        {
            let word = f.id.trim_start_matches("controlnet-");
            for v in &f.variants {
                assert!(v.file_name.contains(word), "{}/{}", f.id, v.id);
            }
        }
    }

    #[test]
    fn embedded_catalog_pins_revisions_and_every_checkpoint_has_a_base_model() {
        let c = Catalog::embedded(&policy()).unwrap();
        for f in &c.families {
            for v in &f.variants {
                assert!(v.source.url.contains("/resolve/"), "{}", v.id);
                assert!(
                    !v.source.url.contains("/resolve/main/"),
                    "リビジョンを固定する: {}",
                    v.id
                );
                // SD1.5 / SDXL 用のもの(チェックポイント・ControlNet)は、ベースモデルを必ず持つ。
                // 検出モデルと Flux 一式は、ベースモデルの区別がない
                if matches!(f.category, Category::Checkpoint | Category::Controlnet) {
                    assert!(v.base_model.is_some(), "{}/{}", f.id, v.id);
                }
            }
        }
    }

    #[test]
    fn every_selectable_model_has_a_profile() {
        // docs/18 §2: 生成画面で選べるモデル(チェックポイント・Flux 本体)には説明が必須
        let c = Catalog::embedded(&policy()).unwrap();
        let selectable: Vec<_> = c
            .families
            .iter()
            .filter(|f| f.is_selectable_model())
            .collect();
        assert!(
            selectable.len() >= 6,
            "SD1.5 / SDXL / Flux 2 種 / NoobAI-XL / Animagine"
        );
        for f in selectable {
            assert!(f.profile.is_some(), "{} に profile がない", f.id);
        }
    }

    #[test]
    fn anime_models_are_in_the_catalog_with_tags() {
        // docs/16 §3.2・§3.3: NoobAI-XL と Animagine XL 4.0(どちらも SDXL 派生・アニメ)
        let c = Catalog::embedded(&policy()).unwrap();
        for id in ["checkpoint-noobai-xl", "checkpoint-animagine-xl-4"] {
            let f = c
                .families
                .iter()
                .find(|f| f.id == id)
                .unwrap_or_else(|| panic!("{id}"));
            assert_eq!(f.category, Category::Checkpoint);
            assert!(f.tags.contains(&Tag::Anime), "{id}");
            let p = f.profile.as_ref().unwrap();
            assert_ne!(p.moderation.level, ModerationLevel::Unknown, "{id}");
            assert_eq!(p.prompt_style.kind, PromptKind::Tags);
            for v in &f.variants {
                // 既存の sdxl 種別でそのまま生成できる(docs/16 §2)ので、ベースは SDXL
                assert_eq!(v.base_model, Some(BaseModel::Sdxl), "{id}/{}", v.id);
                assert!(v.requires_consent, "{id}/{}: 同意ダイアログの対象", v.id);
                assert_eq!(v.dest_dir, DestDir::Checkpoints);
            }
            assert_eq!(f.variants.iter().filter(|v| v.recommended).count(), 1);
        }
        // 取得元は Hugging Face の公開リポジトリのリビジョン固定(認証不要を 2026-11-19 に確認)
        let (_, noob) = c.find("checkpoint-noobai-xl", "noobai-xl-v1.1").unwrap();
        assert_eq!(noob.size_bytes, 7_105_349_958);
        let (_, opt) = c
            .find("checkpoint-animagine-xl-4", "animagine-xl-4.0-opt")
            .unwrap();
        assert_eq!(opt.size_bytes, 6_938_350_040);
    }

    #[test]
    fn new_model_kinds_are_in_the_catalog_with_everything_they_need() {
        // docs/17 §2・§5: Anima / Qwen-Image 2.1 / Krea 2 は、本体・テキストエンコーダ・VAE を取得できる
        let c = Catalog::embedded(&policy()).unwrap();
        let cases = [
            (
                "anima-base",
                FamilyKind::Anima,
                Category::Anima,
                "anima-base-v1.0",
                4_182_218_328u64,
            ),
            (
                "qwen-image-2-1",
                FamilyKind::QwenImage,
                Category::QwenImage,
                "int8-convrot",
                7_256_783_064,
            ),
            (
                "krea2-turbo",
                FamilyKind::Krea2,
                Category::Krea2,
                "fp8-scaled",
                13_141_730_784,
            ),
        ];
        for (id, kind, category, recommended, size) in cases {
            let f = c
                .families
                .iter()
                .find(|f| f.id == id)
                .unwrap_or_else(|| panic!("{id}"));
            assert_eq!((f.kind, f.category), (Some(kind), category), "{id}");
            assert!(f.is_selectable_model() && f.profile.is_some(), "{id}");
            // 本体は unet/ に置き、同意の対象
            assert!(
                f.variants
                    .iter()
                    .all(|v| v.dest_dir == DestDir::Unet && v.requires_consent),
                "{id}"
            );
            let rec = f.variants.iter().find(|v| v.recommended).unwrap();
            assert_eq!(
                (rec.id.as_str(), rec.size_bytes),
                (recommended, size),
                "{id}"
            );
            // 必要なファイル: テキストエンコーダ(clip/)と VAE(vae/)がちょうど 1 つずつ
            assert_eq!(f.requires.len(), 2, "{id}");
            let dests: Vec<_> = f
                .requires
                .iter()
                .map(|r| c.find(&r.family_id, &r.variant_id).unwrap().1.dest_dir)
                .collect();
            assert_eq!(dests, vec![DestDir::Clip, DestDir::Vae], "{id}");
            // 取得元は Hugging Face のリビジョン固定
            for v in &f.variants {
                assert!(
                    v.source.url.contains("/resolve/") && !v.source.url.contains("/resolve/main/"),
                    "{id}"
                );
            }
        }
        // 本体以外(エンコーダ・VAE)は、生成画面で選べるモデルではない
        for id in [
            "anima-text-encoder",
            "anima-vae",
            "qwen-image-2-1-text-encoder",
            "qwen-image-2-1-vae",
            "krea2-text-encoder",
            "krea2-vae",
        ] {
            let f = c.families.iter().find(|f| f.id == id).unwrap();
            assert!(
                !f.is_selectable_model() && f.kind.is_none() && f.profile.is_none(),
                "{id}"
            );
        }
    }

    #[test]
    fn anima_and_krea2_share_the_identical_qwen_image_vae() {
        // docs/17 §2: 同一ファイル(SHA-256・サイズが一致)なので、別ファミリーに置ける
        let c = Catalog::embedded(&policy()).unwrap();
        let (_, a) = c.find("anima-vae", "qwen-image-vae").unwrap();
        let (_, k) = c.find("krea2-vae", "qwen-image-vae").unwrap();
        assert_eq!(a.file_name, k.file_name);
        assert_eq!(
            (a.sha256.as_str(), a.size_bytes),
            (k.sha256.as_str(), k.size_bytes)
        );
        assert_eq!(
            a.sha256,
            "a70580f0213e67967ee9c95f05bb400e8fb08307e017a924bf3441223e023d1f"
        );
        assert_ne!(a.source.repo, k.source.repo, "取得元は各配布元のもの");
    }

    #[test]
    fn moderation_levels_follow_the_confirmed_sources() {
        // docs/18 §10.3。Anima は安全タグ(safe 〜 explicit)で制御(README)、Krea 2 はモデルカードに
        // 学習データの選別と追加調整の記載あり。Qwen-Image 2.1 は公式の記載を確認できず unknown
        let c = Catalog::embedded(&policy()).unwrap();
        let level = |id: &str| {
            c.families
                .iter()
                .find(|f| f.id == id)
                .unwrap()
                .profile
                .as_ref()
                .unwrap()
                .moderation
                .level
        };
        assert_eq!(level("anima-base"), ModerationLevel::Loose);
        assert_eq!(level("krea2-turbo"), ModerationLevel::Strict);
        assert_eq!(level("qwen-image-2-1"), ModerationLevel::Unknown);
        let tags = |id: &str| c.families.iter().find(|f| f.id == id).unwrap().tags.clone();
        assert_eq!(tags("anima-base"), vec![Tag::Anime]);
        assert!(tags("qwen-image-2-1").is_empty() && tags("krea2-turbo").is_empty());
    }

    #[test]
    fn existing_flux_models_carry_the_flux_gguf_kind() {
        let c = Catalog::embedded(&policy()).unwrap();
        for id in ["flux-schnell-gguf", "flux-dev-gguf"] {
            assert_eq!(
                c.families.iter().find(|f| f.id == id).unwrap().kind,
                Some(FamilyKind::FluxGguf)
            );
        }
    }

    #[test]
    fn rejects_inconsistent_kind_and_requires() {
        let id = "anima-base";
        assert!(mutated(id, |_| {}).is_ok());
        // unet/ に置くのに kind が無い
        assert!(
            mutated(id, |f| {
                f.as_object_mut().unwrap().remove("kind");
            })
            .is_err()
        );
        // kind とカテゴリの不一致
        assert!(mutated(id, |f| f["kind"] = "krea2".into()).is_err());
        // requires の参照先が無い・自分自身
        assert!(mutated(id, |f| f["requires"][0]["familyId"] = "nope".into()).is_err());
        assert!(mutated(id, |f| f["requires"][0]["variantId"] = "nope".into()).is_err());
        assert!(mutated(id, |f| f["requires"][0]["familyId"] = id.into()).is_err());
        // エンコーダ(kind なし)に requires は付けられない
        assert!(
            mutated("anima-text-encoder", |f| {
                f["requires"] =
                    serde_json::json!([{ "familyId": "anima-vae", "variantId": "qwen-image-vae" }]);
            })
            .is_err()
        );
        // 内容(SHA-256)の違う同名ファイルの共有は拒否
        assert!(
            mutated("krea2-vae", |f| f["variants"][0]["sha256"] =
                "1".repeat(64).into())
            .is_err()
        );
    }

    #[test]
    fn every_selectable_family_has_a_moderation_note_when_rated() {
        let c = Catalog::embedded(&policy()).unwrap();
        for f in c.families.iter().filter(|f| f.is_selectable_model()) {
            let m = &f.profile.as_ref().unwrap().moderation;
            if m.level != ModerationLevel::Unknown {
                assert!(!m.note.ja.is_empty() && !m.note.en.is_empty(), "{}", f.id);
            }
        }
    }

    /// 組み込みカタログの 1 ファミリーを書き換えて検証する(検証が拒否することを確かめる)。
    fn mutated(id: &str, edit: impl FnOnce(&mut serde_json::Value)) -> AppResult<Catalog> {
        let mut v: serde_json::Value = serde_json::from_str(EMBEDDED).unwrap();
        let fam = v["families"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|f| f["id"] == id)
            .unwrap();
        edit(fam);
        Catalog::parse(&v.to_string(), &policy())
    }

    #[test]
    fn rejects_inconsistent_profiles_and_tags() {
        let id = "checkpoint-noobai-xl";
        assert!(mutated(id, |_| {}).is_ok());
        // profile を外す(選べるモデルには必須)
        assert!(
            mutated(id, |f| {
                f.as_object_mut().unwrap().remove("profile");
            })
            .is_err()
        );
        // 段階が確定しているのに根拠が空
        assert!(mutated(id, |f| f["profile"]["moderation"]["note"]["en"] = "".into()).is_err());
        // anime タグと profile.style の不一致
        assert!(mutated(id, |f| f["tags"] = serde_json::json!([])).is_err());
        // 廃止した nsfw タグは読み込めない
        assert!(mutated(id, |f| f["tags"] = serde_json::json!(["anime", "nsfw"])).is_err());
        // 件数・値の範囲
        assert!(
            mutated(id, |f| f["profile"]["bestFor"]["en"] =
                serde_json::json!([]))
            .is_err()
        );
        assert!(
            mutated(id, |f| {
                f["profile"]["bestFor"]["ja"] = serde_json::json!(["a", "b", "c", "d", "e"]);
                f["profile"]["bestFor"]["en"] = serde_json::json!(["a", "b", "c", "d", "e"]);
            })
            .is_err()
        );
        assert!(mutated(id, |f| f["profile"]["recommended"]["steps"] = 0.into()).is_err());
        assert!(
            mutated(id, |f| f["profile"]["recommended"]["size"]["width"] =
                1000.into())
            .is_err()
        );
        assert!(mutated(id, |f| f["profile"]["resources"]["vramGb"] = 0.into()).is_err());
        assert!(mutated(id, |f| f["profile"]["summary"]["en"] = "".into()).is_err());
        // 未知のタグは読み込めない
        assert!(
            mutated(id, |f| f["tags"] =
                serde_json::json!(["anime", "nsfw", "gore"]))
            .is_err()
        );
    }

    #[test]
    fn rejects_an_old_schema_version() {
        let json = EMBEDDED.replacen("\"schemaVersion\": 2", "\"schemaVersion\": 1", 1);
        assert!(Catalog::parse(&json, &policy()).is_err());
    }

    fn catalog_with(file_name: &str, url: &str, sha: &str) -> String {
        format!(
            r#"{{"schemaVersion":2,"families":[{{"id":"f","category":"detector",
            "name":{{"ja":"a","en":"a"}},"description":{{"ja":"a","en":"a"}},
            "variants":[{{"id":"v","label":{{"ja":"a","en":"a"}},"source":{{"url":"{url}","repo":"r"}},
            "sizeBytes":10,"sha256":"{sha}","destDir":"checkpoints","fileName":"{file_name}",
            "license":{{"name":"l","url":"https://x.test","commercialUse":true}}}}]}}]}}"#
        )
    }

    const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const OK_URL: &str = "https://huggingface.co/a/b/resolve/abc/m.safetensors";

    #[test]
    fn accepts_a_minimal_valid_catalog() {
        Catalog::parse(&catalog_with("m.safetensors", OK_URL, SHA), &policy()).unwrap();
    }

    #[test]
    fn rejects_unsafe_file_names() {
        for name in [
            "../m.safetensors",
            "sub/m.safetensors",
            "sub\\\\m.safetensors",
            "m.safetensors:evil",
            ".hidden.safetensors",
            "m.exe",
            "m",
            "",
        ] {
            let json = catalog_with(name, OK_URL, SHA);
            assert!(
                Catalog::parse(&json, &policy()).is_err(),
                "{name:?} は拒否される"
            );
        }
    }

    #[test]
    fn rejects_disallowed_hosts_and_schemes() {
        for url in [
            "https://evil.example.com/m.safetensors",
            "http://huggingface.co/m.safetensors",
            "https://huggingface.co.evil.com/m.safetensors",
            "https://nothuggingface.co/m.safetensors",
            "file:///etc/passwd",
        ] {
            let json = catalog_with("m.safetensors", url, SHA);
            assert!(
                Catalog::parse(&json, &policy()).is_err(),
                "{url} は拒否される"
            );
        }
    }

    #[test]
    fn rejects_malformed_hashes() {
        for sha in ["", "abc", &"G".repeat(64), &"A".repeat(64)] {
            let json = catalog_with("m.safetensors", OK_URL, sha);
            assert!(
                Catalog::parse(&json, &policy()).is_err(),
                "{sha:?} は拒否される"
            );
        }
    }

    #[test]
    fn rejects_duplicate_targets() {
        let one = catalog_with("m.safetensors", OK_URL, SHA);
        let mut v: serde_json::Value = serde_json::from_str(&one).unwrap();
        // 同じファミリー ID の重複は拒否
        let fam = v["families"][0].clone();
        v["families"].as_array_mut().unwrap().push(fam.clone());
        assert!(Catalog::parse(&v.to_string(), &policy()).is_err());

        // 別ファミリーで、配置先・ファイル名が同じで内容(SHA-256)が違うものは拒否
        let mut other = fam.clone();
        other["id"] = "g".into();
        other["variants"][0]["sha256"] = "1".repeat(64).into();
        let mut v: serde_json::Value = serde_json::from_str(&one).unwrap();
        v["families"].as_array_mut().unwrap().push(other);
        assert!(Catalog::parse(&v.to_string(), &policy()).is_err());
    }

    #[test]
    fn allows_two_families_to_share_the_very_same_file() {
        // docs/17 §5: Anima と Krea 2 は、同一の qwen_image_vae.safetensors を共有する
        let one = catalog_with("m.safetensors", OK_URL, SHA);
        let mut v: serde_json::Value = serde_json::from_str(&one).unwrap();
        let mut other = v["families"][0].clone();
        other["id"] = "g".into();
        v["families"].as_array_mut().unwrap().push(other);
        Catalog::parse(&v.to_string(), &policy()).unwrap();
    }
}
