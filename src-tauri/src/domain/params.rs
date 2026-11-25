//! 生成パラメータ(docs/03-data-model.md「params_json」)。エンジン非依存。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// JS の `Number` で安全に扱える最大の整数(2^53 - 1)。シードの上限。
pub const MAX_SEED: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Sd15,
    Sdxl,
    Flux,
    FluxGguf,
    /// docs/17: 拡散モデル・テキストエンコーダ 1 つ・VAE を別々に読む種別。
    Anima,
    QwenImage,
    #[serde(rename = "krea2")]
    Krea2,
}

impl ModelKind {
    pub fn is_flux(self) -> bool {
        matches!(self, Self::Flux | Self::FluxGguf)
    }

    /// 拡散モデル・テキストエンコーダ(1 つ)・VAE を別々に読む種別(Anima / Qwen-Image 2.1 / Krea 2)。
    pub fn is_split_model(self) -> bool {
        matches!(self, Self::Anima | Self::QwenImage | Self::Krea2)
    }

    /// 顔補正・ControlNet・Inpaint に対応するのは SD 1.5 / SDXL のみ(docs/10 §7、17 §3)。
    pub fn supports_extras(self) -> bool {
        matches!(self, Self::Sd15 | Self::Sdxl)
    }

    /// ネガティブプロンプトを使う種別。Flux は使わず、Krea 2 は `ConditioningZeroOut` で代える。
    pub fn uses_negative(self) -> bool {
        !self.is_flux() && self != Self::Krea2
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub kind: ModelKind,
    pub name: String,
    /// Flux 系: `[T5, CLIP-L]` の順。Anima / Qwen-Image 2.1 / Krea 2: テキストエンコーダ 1 つ。SD 系は空。
    #[serde(default)]
    pub clip: Vec<String>,
    /// Flux 系・Anima / Qwen-Image 2.1 / Krea 2 は必須。SD 系は未指定ならチェックポイント内蔵の VAE を使う。
    #[serde(default)]
    pub vae: Option<String>,
}

/// 顔補正(ADetailer 相当。docs/10-assist-features.md §4)。SD 1.5 / SDXL のみ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceDetailerParams {
    #[serde(default)]
    pub enabled: bool,
    /// 検出モデル(`UltralyticsDetectorProvider` の `bbox/…`)。
    #[serde(default)]
    pub detector: String,
    /// 補正の強さ(再サンプリングの denoise)。
    #[serde(default = "default_face_denoise")]
    pub denoise: f64,
    /// 補正用プロンプト。空なら本体のプロンプトを使う。
    #[serde(default)]
    pub prompt: String,
    /// 検出しきい値。
    #[serde(default = "default_face_confidence")]
    pub confidence: f64,
}

/// ControlNet の前処理(docs/10-assist-features.md §5)。参照画像を条件画像に変換する方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preprocessor {
    Canny,
    Depth,
    Openpose,
    /// 前処理なし(線画・深度マップ・ポーズ画像として用意した画像をそのまま使う)。
    None,
}

/// ControlNet の 1 ユニット。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlnetUnit {
    /// 参照画像。画像保存先からの相対パス(`inputs/<uuid>.png`、`/` 区切り)。
    pub image: String,
    /// ControlNet モデル(`ControlNetLoader` の候補)。
    pub model: String,
    #[serde(default = "default_preprocessor")]
    pub preprocessor: Preprocessor,
    #[serde(default = "default_strength")]
    pub strength: f64,
    #[serde(default)]
    pub start_percent: f64,
    #[serde(default = "default_end_percent")]
    pub end_percent: f64,
}

/// ControlNet(docs/10 §5)。SD 1.5 / SDXL のみ。v0.2 は 1 ユニットまで。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlnetParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub units: Vec<ControlnetUnit>,
}

/// Inpaint(docs/13 §3.2)。SD 1.5 / SDXL のみ。マスクの白い範囲だけを描き直す。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InpaintParams {
    #[serde(default)]
    pub enabled: bool,
    /// 参照画像(`inputs/<uuid>.png`)。
    #[serde(default)]
    pub image: String,
    /// マスク(`inputs/<uuid>.png`。白 = 描き直す範囲)。
    #[serde(default)]
    pub mask: String,
    #[serde(default = "default_inpaint_denoise")]
    pub denoise: f64,
    /// マスクを広げる量(px)。
    #[serde(default = "default_inpaint_grow")]
    pub grow_mask: u32,
    /// 境界のぼかし(px)。
    #[serde(default = "default_inpaint_feather")]
    pub feather: u32,
    /// 結果を元画像へ合成し直す(マスク外を元画像と一致させる)。
    #[serde(default = "default_true")]
    pub composite_back: bool,
}

/// 画像から生成(img2img。docs/19 §4)。SD 1.5 / SDXL のみ。元画像を出発点に、denoise の分だけ描き直す。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Img2imgParams {
    #[serde(default)]
    pub enabled: bool,
    /// 元画像(`inputs/<uuid>.png`)。
    #[serde(default)]
    pub image: String,
    #[serde(default = "default_img2img_denoise")]
    pub denoise: f64,
}

fn default_img2img_denoise() -> f64 {
    0.6
}

/// 参照画像の枚数の上限(docs/24 §4.1。ノードは 16 枚まで受けるが、VRAM と速度の都合でアプリ側は 4 枚)。
pub const MAX_REFERENCE_IMAGES: usize = 4;
/// 参照画像の大きさ(`TextEncodeQwenImage21` の `resolution`。総画素の基準)として許す値(docs/24 Q-120)。
pub const REFERENCE_RESOLUTIONS: [u32; 3] = [768, 1024, 1536];

/// 参照画像つき生成(Qwen-Image 2.1 の画像編集。docs/24 §4.1)。`qwen_image` のみ。
/// 1 枚目が編集される画像で、出力サイズも 1 枚目で決まる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceParams {
    #[serde(default)]
    pub enabled: bool,
    /// 参照画像(`inputs/<uuid>.png`)。1 枚目が編集対象。
    #[serde(default)]
    pub images: Vec<String>,
    /// 参照画像の総画素の基準(768 / 1024 / 1536)。
    #[serde(default = "default_reference_resolution")]
    pub resolution: u32,
}

fn default_reference_resolution() -> u32 {
    1024
}

/// 顔の入れ替えで使う LoRA(docs/42 §7・Q-342。BFS の Qwen-Image 2.1 用 head swap v1.1)。
pub const FACE_SWAP_LORA: &str = "bfs_head_v1.1_qwen_2.1.safetensors";
/// 顔の入れ替えの後段合成で使う顔検出モデルの既定(`UltralyticsDetectorProvider` の `bbox/…`)。
pub const FACE_SWAP_DETECTOR: &str = "bbox/face_yolov9c.pt";

/// 合成する範囲(docs/42 §5・Q-332)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FaceSwapRegion {
    /// 髪を含む頭(既定。実機で継ぎ目が目立たなかった)。
    #[default]
    Head,
    /// 検出した顔だけ(髪・首は元のまま)。
    Face,
}

/// 顔の入れ替え(docs/42 §4.1)。`qwen_image` のみ。参照画像つき生成(24)の上に、顔入れ替え LoRA を 1 つ適用する。
/// 1 ジョブにつき対象 1 枚(複数の対象は呼び出し側が 1 枚ずつ投入する)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceSwapParams {
    #[serde(default)]
    pub enabled: bool,
    /// 対象画像(`inputs/<uuid>.png`)。顔以外はこの画像のまま保つ。
    #[serde(default)]
    pub target: String,
    /// 入れ替える顔の画像(`inputs/<uuid>.png`)。
    #[serde(default)]
    pub face: String,
    /// 顔以外を元のまま保つ(後段合成)。
    #[serde(default = "default_true")]
    pub keep_outside: bool,
    /// 合成する範囲。
    #[serde(default)]
    pub region: FaceSwapRegion,
    /// 参照画像の総画素の基準(768 / 1024 / 1536)。
    #[serde(default = "default_reference_resolution")]
    pub resolution: u32,
    /// 後段合成の顔検出モデル(`bbox/…`)。
    #[serde(default = "default_face_swap_detector")]
    pub detector: String,
}

fn default_face_swap_detector() -> String {
    FACE_SWAP_DETECTOR.to_owned()
}

fn default_inpaint_denoise() -> f64 {
    0.7
}
fn default_inpaint_grow() -> u32 {
    8
}
fn default_inpaint_feather() -> u32 {
    8
}
fn default_true() -> bool {
    true
}

fn default_preprocessor() -> Preprocessor {
    Preprocessor::Canny
}
fn default_strength() -> f64 {
    1.0
}
fn default_end_percent() -> f64 {
    1.0
}

/// 参照画像の相対パスとして妥当か(`inputs/` 直下の安全なファイル名だけを許す)。
/// ここを通ったパスだけを、保存先に結合してファイルを読む。
pub fn is_valid_input_path(rel: &str) -> bool {
    let Some(name) = rel.strip_prefix("inputs/") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 顔検出モデルの名前として妥当か(`UltralyticsDetectorProvider` の `bbox/…`)。
fn is_valid_detector(name: &str) -> bool {
    name.strip_prefix("bbox/").is_some_and(|n| {
        !n.is_empty()
            && n.len() <= 100
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}

fn default_face_denoise() -> f64 {
    0.4
}
fn default_face_confidence() -> f64 {
    0.5
}

fn default_schema_version() -> u32 {
    1
}
fn default_cfg() -> f64 {
    7.0
}
fn default_guidance() -> f64 {
    3.5
}
fn default_batch() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationParams {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub model: ModelSpec,
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    #[serde(default = "default_cfg")]
    pub cfg: f64,
    /// Flux 系の FluxGuidance。
    #[serde(default = "default_guidance")]
    pub guidance: f64,
    pub sampler: String,
    pub scheduler: String,
    pub seed: u64,
    #[serde(default)]
    pub seed_randomize: bool,
    #[serde(default = "default_batch")]
    pub batch_size: u32,
    /// 顔補正。無ければ無効(docs/03「生成補助機能のフィールド」)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_detailer: Option<FaceDetailerParams>,
    /// ControlNet。無ければ無効。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controlnet: Option<ControlnetParams>,
    /// Inpaint。無ければ無効。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inpaint: Option<InpaintParams>,
    /// 画像から生成(img2img)。無ければ無効。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img2img: Option<Img2imgParams>,
    /// 参照画像つき生成(Qwen-Image 2.1 の画像編集。docs/24)。無ければ無効。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceParams>,
    /// 顔の入れ替え(docs/42)。無ければ無効。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_swap: Option<FaceSwapParams>,
}

impl GenerationParams {
    /// 有効な顔の入れ替えの設定(無効・未指定なら `None`)。
    pub fn active_face_swap(&self) -> Option<&FaceSwapParams> {
        self.face_swap.as_ref().filter(|f| f.enabled)
    }

    /// 有効な参照画像の設定(無効・未指定なら `None`)。
    pub fn active_reference(&self) -> Option<&ReferenceParams> {
        self.reference.as_ref().filter(|r| r.enabled)
    }

    /// 有効な img2img の設定(無効・未指定なら `None`)。
    pub fn active_img2img(&self) -> Option<&Img2imgParams> {
        self.img2img.as_ref().filter(|i| i.enabled)
    }

    /// 有効な Inpaint の設定(無効・未指定なら `None`)。
    pub fn active_inpaint(&self) -> Option<&InpaintParams> {
        self.inpaint.as_ref().filter(|i| i.enabled)
    }

    /// エンジンへ送る画像の数。順序は ControlNet の各ユニット、Inpaint の画像、Inpaint のマスク、
    /// img2img の元画像(Inpaint と img2img は排他)、参照画像つき生成の画像(他の画像入力とは排他)。
    pub fn upload_count(&self) -> usize {
        self.active_controlnet_units().len()
            + if self.active_inpaint().is_some() {
                2
            } else {
                0
            }
            + usize::from(self.active_img2img().is_some())
            + self.active_reference().map_or(0, |r| r.images.len())
            // 顔の入れ替え: 対象 → 顔 の 2 枚(参照画像つき生成と排他)
            + if self.active_face_swap().is_some() { 2 } else { 0 }
    }

    /// 有効な ControlNet のユニット(無効・未指定・ユニットなしなら空)。
    pub fn active_controlnet_units(&self) -> &[ControlnetUnit] {
        match &self.controlnet {
            Some(c) if c.enabled => &c.units,
            _ => &[],
        }
    }

    /// 有効な顔補正の設定(無効・未指定なら `None`)。
    pub fn active_face_detailer(&self) -> Option<&FaceDetailerParams> {
        self.face_detailer.as_ref().filter(|f| f.enabled)
    }

    pub fn validate(&self) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));

        // 顔の入れ替えは、空のプロンプトを雛形で補う(フロントが詰める。docs/42 §4.2・Q-389)
        if self.prompt.trim().is_empty() && self.active_face_swap().is_none() {
            return bad("prompt", "プロンプトを入力してください");
        }
        if self.model.name.trim().is_empty() {
            return bad("model", "モデルを選択してください");
        }
        for (field, v) in [("width", self.width), ("height", self.height)] {
            if !(256..=2048).contains(&v) || v % 64 != 0 {
                return bad(field, "256〜2048 の 64 の倍数で指定してください");
            }
        }
        if !(1..=150).contains(&self.steps) {
            return bad("steps", "ステップ数は 1〜150 で指定してください");
        }
        if !(0.0..=30.0).contains(&self.cfg) || !self.cfg.is_finite() {
            return bad("cfg", "CFG は 0〜30 で指定してください");
        }
        if !(0.0..=100.0).contains(&self.guidance) || !self.guidance.is_finite() {
            return bad("guidance", "Guidance は 0〜100 で指定してください");
        }
        if self.sampler.trim().is_empty() {
            return bad("sampler", "サンプラーを選択してください");
        }
        if self.scheduler.trim().is_empty() {
            return bad("scheduler", "スケジューラを選択してください");
        }
        if self.seed > MAX_SEED {
            return bad("seed", "シードが大きすぎます");
        }
        if !(1..=8).contains(&self.batch_size) {
            return bad("batchSize", "枚数は 1〜8 で指定してください");
        }
        if self.model.kind.is_flux() {
            if self.model.clip.len() != 2 || self.model.clip.iter().any(|c| c.trim().is_empty()) {
                return bad("clip", "テキストエンコーダ(T5 と CLIP-L)を選択してください");
            }
        } else if self.model.kind.is_split_model()
            && (self.model.clip.len() != 1 || self.model.clip[0].trim().is_empty())
        {
            return bad("clip", "テキストエンコーダを選択してください");
        }
        if (self.model.kind.is_flux() || self.model.kind.is_split_model())
            && self
                .model
                .vae
                .as_deref()
                .is_none_or(|v| v.trim().is_empty())
        {
            return bad("vae", "VAE を選択してください");
        }
        if let Some(fd) = self.active_face_detailer() {
            if !self.model.kind.supports_extras() {
                return bad("faceDetailer", "顔補正は SD 1.5 / SDXL のみ対応です");
            }
            if fd.detector.trim().is_empty() {
                return bad("faceDetailer.detector", "検出モデルを選択してください");
            }
            if !(0.05..=1.0).contains(&fd.denoise) || !fd.denoise.is_finite() {
                return bad(
                    "faceDetailer.denoise",
                    "強さは 0.05〜1.0 で指定してください",
                );
            }
            if !(0.1..=1.0).contains(&fd.confidence) || !fd.confidence.is_finite() {
                return bad(
                    "faceDetailer.confidence",
                    "検出しきい値は 0.1〜1.0 で指定してください",
                );
            }
        }
        if self.controlnet.as_ref().is_some_and(|c| c.enabled) {
            self.validate_controlnet()?;
        }
        if let Some(ip) = self.active_inpaint() {
            self.validate_inpaint(ip)?;
        }
        if let Some(i2i) = self.active_img2img() {
            self.validate_img2img(i2i)?;
        }
        if let Some(r) = self.active_reference() {
            self.validate_reference(r)?;
        }
        if let Some(f) = self.active_face_swap() {
            self.validate_face_swap(f)?;
        }
        Ok(())
    }

    /// docs/42 §4.2。種別・他の画像入力機能との排他・画像のパス・大きさ・枚数。
    fn validate_face_swap(&self, f: &FaceSwapParams) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));
        if self.model.kind != ModelKind::QwenImage {
            return bad("faceSwap", "顔の入れ替えは Qwen-Image 2.1 のみ対応です");
        }
        if self.active_reference().is_some()
            || self.active_inpaint().is_some()
            || self.active_img2img().is_some()
            || !self.active_controlnet_units().is_empty()
            || self.active_face_detailer().is_some()
        {
            return bad(
                "faceSwap",
                "顔の入れ替えは他の画像入力機能(参照画像・Inpaint・img2img・ControlNet・顔補正)と併用できません",
            );
        }
        if !is_valid_input_path(&f.target) {
            return bad("faceSwap.target", "対象画像を選択してください");
        }
        if !is_valid_input_path(&f.face) {
            return bad("faceSwap.face", "顔画像を選択してください");
        }
        if !REFERENCE_RESOLUTIONS.contains(&f.resolution) {
            return bad(
                "faceSwap.resolution",
                "参照画像の大きさは 768 / 1024 / 1536 のいずれかで指定してください",
            );
        }
        if f.keep_outside && !is_valid_detector(&f.detector) {
            return bad("faceSwap.detector", "顔検出モデルが不正です");
        }
        // 後段合成は 1 枚の元画像に貼り戻すため、同じジョブで複数枚は作らない(枚数は投入数で増やす)
        if self.batch_size != 1 {
            return bad(
                "batchSize",
                "顔の入れ替えでは 1 ジョブ 1 枚です(枚数は投入数で指定)",
            );
        }
        Ok(())
    }

    /// docs/24 §4.2。種別・他の画像入力機能との排他・枚数・パス・大きさ。
    fn validate_reference(&self, r: &ReferenceParams) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));
        if self.model.kind != ModelKind::QwenImage {
            return bad("reference", "参照画像は Qwen-Image 2.1 のみ対応です");
        }
        if self.active_inpaint().is_some()
            || self.active_img2img().is_some()
            || !self.active_controlnet_units().is_empty()
            || self.active_face_detailer().is_some()
            || self.active_face_swap().is_some()
        {
            return bad(
                "reference",
                "参照画像は他の画像入力機能(Inpaint・img2img・ControlNet・顔補正)と併用できません",
            );
        }
        if r.images.is_empty() {
            return bad("reference.images", "参照画像を選択してください");
        }
        if r.images.len() > MAX_REFERENCE_IMAGES {
            return bad("reference.images", "参照画像は 4 枚まで指定できます");
        }
        if r.images.iter().any(|i| !is_valid_input_path(i)) {
            return bad("reference.images", "参照画像のパスが不正です");
        }
        if !REFERENCE_RESOLUTIONS.contains(&r.resolution) {
            return bad(
                "reference.resolution",
                "参照画像の大きさは 768 / 1024 / 1536 のいずれかで指定してください",
            );
        }
        Ok(())
    }

    fn validate_img2img(&self, i2i: &Img2imgParams) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));
        if !self.model.kind.supports_extras() {
            return bad("img2img", "画像から生成は SD 1.5 / SDXL のみ対応です");
        }
        if self.active_inpaint().is_some() {
            return bad("img2img", "画像から生成と Inpaint は同時に使えません");
        }
        if !is_valid_input_path(&i2i.image) {
            return bad("img2img.image", "元画像を選択してください");
        }
        if !(0.05..=1.0).contains(&i2i.denoise) || !i2i.denoise.is_finite() {
            return bad("img2img.denoise", "強さは 0.05〜1.0 で指定してください");
        }
        Ok(())
    }

    fn validate_inpaint(&self, ip: &InpaintParams) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));
        if !self.model.kind.supports_extras() {
            return bad("inpaint", "Inpaint は SD 1.5 / SDXL のみ対応です");
        }
        if self.active_face_detailer().is_some() {
            return bad("inpaint", "Inpaint と顔補正は同時に使えません");
        }
        if !is_valid_input_path(&ip.image) {
            return bad("inpaint.image", "参照画像を選択してください");
        }
        if !is_valid_input_path(&ip.mask) {
            return bad("inpaint.mask", "マスクを描いてください");
        }
        if !(0.05..=1.0).contains(&ip.denoise) || !ip.denoise.is_finite() {
            return bad("inpaint.denoise", "強さは 0.05〜1.0 で指定してください");
        }
        if ip.grow_mask > 64 {
            return bad(
                "inpaint.growMask",
                "マスクの拡張は 0〜64 で指定してください",
            );
        }
        if ip.feather > 31 {
            return bad("inpaint.feather", "境界のぼかしは 0〜31 で指定してください");
        }
        Ok(())
    }

    fn validate_controlnet(&self) -> AppResult<()> {
        let bad = |field: &str, msg: &str| Err(AppError::validation(field, msg));
        if !self.model.kind.supports_extras() {
            return bad("controlnet", "ControlNet は SD 1.5 / SDXL のみ対応です");
        }
        let units = self.active_controlnet_units();
        if units.is_empty() {
            return bad("controlnet.units", "参照画像を選択してください");
        }
        if units.len() > 1 {
            return bad("controlnet.units", "ControlNet は 1 つまで指定できます");
        }
        let u = &units[0];
        if u.image.trim().is_empty() {
            return bad("controlnet.units[0].image", "参照画像を選択してください");
        }
        if !is_valid_input_path(&u.image) {
            return bad("controlnet.units[0].image", "参照画像のパスが不正です");
        }
        if u.model.trim().is_empty() {
            return bad(
                "controlnet.units[0].model",
                "ControlNet モデルを選択してください",
            );
        }
        if !(0.0..=2.0).contains(&u.strength) || !u.strength.is_finite() {
            return bad(
                "controlnet.units[0].strength",
                "強さは 0〜2 で指定してください",
            );
        }
        let range_ok = (0.0..=1.0).contains(&u.start_percent)
            && (0.0..=1.0).contains(&u.end_percent)
            && u.start_percent < u.end_percent;
        if !range_ok {
            return bad(
                "controlnet.units[0].endPercent",
                "適用範囲は 0〜1 で、開始より終了を大きくしてください",
            );
        }
        Ok(())
    }
}

#[cfg(test)]
pub fn sample(kind: ModelKind) -> GenerationParams {
    let flux = kind.is_flux();
    let split = kind.is_split_model();
    GenerationParams {
        schema_version: 1,
        model: ModelSpec {
            kind,
            name: if flux {
                "flux.gguf".into()
            } else {
                "model.safetensors".into()
            },
            clip: if flux {
                vec!["t5xxl.safetensors".into(), "clip_l.safetensors".into()]
            } else if split {
                vec!["text_encoder.safetensors".into()]
            } else {
                vec![]
            },
            vae: (flux || split).then(|| "ae.safetensors".to_string()),
        },
        prompt: "a cat".into(),
        negative_prompt: "blurry".into(),
        width: 512,
        height: 512,
        steps: 20,
        cfg: 7.0,
        guidance: 3.5,
        sampler: "euler".into(),
        scheduler: "normal".into(),
        seed: 42,
        seed_randomize: false,
        batch_size: 1,
        face_detailer: None,
        controlnet: None,
        inpaint: None,
        img2img: None,
        reference: None,
        face_swap: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 顔補正・ControlNet・Inpaint に対応しない種別(docs/17 §3)。
    const NON_SD: [ModelKind; 5] = [
        ModelKind::Flux,
        ModelKind::FluxGguf,
        ModelKind::Anima,
        ModelKind::QwenImage,
        ModelKind::Krea2,
    ];

    fn field_of(r: AppResult<()>) -> String {
        match r.unwrap_err() {
            AppError::Validation { field, .. } => field,
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn valid_samples_pass() {
        for k in [ModelKind::Sd15, ModelKind::Sdxl].into_iter().chain(NON_SD) {
            sample(k).validate().unwrap();
        }
    }

    #[test]
    fn split_models_need_exactly_one_text_encoder_and_a_vae() {
        // docs/17 §3: Anima / Qwen-Image 2.1 / Krea 2 は、テキストエンコーダ 1 つと VAE が必須
        for kind in [ModelKind::Anima, ModelKind::QwenImage, ModelKind::Krea2] {
            let mut p = sample(kind);
            p.model.clip.clear();
            assert_eq!(field_of(p.validate()), "clip", "{kind:?}");

            let mut p = sample(kind);
            p.model.clip.push("second.safetensors".into());
            assert_eq!(field_of(p.validate()), "clip", "{kind:?}");

            let mut p = sample(kind);
            p.model.clip = vec!["  ".into()];
            assert_eq!(field_of(p.validate()), "clip", "{kind:?}");

            let mut p = sample(kind);
            p.model.vae = None;
            assert_eq!(field_of(p.validate()), "vae", "{kind:?}");
        }
        // SD 系はこれまでどおり、エンコーダも VAE も要らない
        let mut p = sample(ModelKind::Sdxl);
        p.model.clip.clear();
        p.model.vae = None;
        p.validate().unwrap();
    }

    #[test]
    fn model_kind_capabilities_and_serde_names() {
        assert!(ModelKind::Sd15.supports_extras() && ModelKind::Sdxl.supports_extras());
        for k in NON_SD {
            assert!(!k.supports_extras(), "{k:?}");
        }
        assert!(ModelKind::Anima.uses_negative() && ModelKind::QwenImage.uses_negative());
        assert!(!ModelKind::Krea2.uses_negative() && !ModelKind::Flux.uses_negative());
        assert!(ModelKind::Sdxl.uses_negative());
        for (kind, name) in [
            (ModelKind::Anima, "anima"),
            (ModelKind::QwenImage, "qwen_image"),
            (ModelKind::Krea2, "krea2"),
            (ModelKind::FluxGguf, "flux_gguf"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<ModelKind>(name.into()).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn rejects_each_invalid_field() {
        let mut p = sample(ModelKind::Sd15);
        p.prompt = "  ".into();
        assert_eq!(field_of(p.validate()), "prompt");

        let mut p = sample(ModelKind::Sd15);
        p.width = 500; // 64 の倍数でない
        assert_eq!(field_of(p.validate()), "width");

        let mut p = sample(ModelKind::Sd15);
        p.height = 4096;
        assert_eq!(field_of(p.validate()), "height");

        let mut p = sample(ModelKind::Sd15);
        p.steps = 0;
        assert_eq!(field_of(p.validate()), "steps");

        let mut p = sample(ModelKind::Sd15);
        p.cfg = f64::NAN;
        assert_eq!(field_of(p.validate()), "cfg");

        let mut p = sample(ModelKind::Sd15);
        p.seed = MAX_SEED + 1;
        assert_eq!(field_of(p.validate()), "seed");

        let mut p = sample(ModelKind::Sd15);
        p.batch_size = 9;
        assert_eq!(field_of(p.validate()), "batchSize");
    }

    #[test]
    fn flux_requires_clip_and_vae() {
        let mut p = sample(ModelKind::Flux);
        p.model.clip.pop();
        assert_eq!(field_of(p.validate()), "clip");

        let mut p = sample(ModelKind::FluxGguf);
        p.model.vae = None;
        assert_eq!(field_of(p.validate()), "vae");
    }

    fn with_face(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.face_detailer = Some(FaceDetailerParams {
            enabled: true,
            detector: "bbox/face_yolov8m.pt".into(),
            denoise: 0.4,
            prompt: String::new(),
            confidence: 0.5,
        });
        p
    }

    #[test]
    fn face_detailer_is_valid_for_sd_only() {
        with_face(ModelKind::Sd15).validate().unwrap();
        with_face(ModelKind::Sdxl).validate().unwrap();
        for k in NON_SD {
            assert_eq!(field_of(with_face(k).validate()), "faceDetailer");
        }
        // 無効のままなら Flux でも通る(モデル切替で残った設定を弾かない)
        let mut p = with_face(ModelKind::Flux);
        p.face_detailer.as_mut().unwrap().enabled = false;
        p.validate().unwrap();
        assert!(p.active_face_detailer().is_none());
    }

    #[test]
    fn face_detailer_rejects_bad_values() {
        let mut p = with_face(ModelKind::Sd15);
        p.face_detailer.as_mut().unwrap().detector = " ".into();
        assert_eq!(field_of(p.validate()), "faceDetailer.detector");

        for bad in [0.0, 1.5, f64::NAN] {
            let mut p = with_face(ModelKind::Sd15);
            p.face_detailer.as_mut().unwrap().denoise = bad;
            assert_eq!(field_of(p.validate()), "faceDetailer.denoise");
        }
        for bad in [0.0, 1.1] {
            let mut p = with_face(ModelKind::Sd15);
            p.face_detailer.as_mut().unwrap().confidence = bad;
            assert_eq!(field_of(p.validate()), "faceDetailer.confidence");
        }
    }

    #[test]
    fn face_detailer_is_optional_and_round_trips() {
        // 古いデータ(フィールド無し)は無効として読める。保存時にも出力しない
        let p = sample(ModelKind::Sd15);
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("faceDetailer").is_none());

        let p: GenerationParams = serde_json::from_str(
            r#"{"model":{"kind":"sd15","name":"m"},"prompt":"x","width":512,"height":512,
                "steps":20,"sampler":"euler","scheduler":"normal","seed":1,
                "faceDetailer":{"enabled":true,"detector":"bbox/face_yolov8m.pt"}}"#,
        )
        .unwrap();
        let fd = p.active_face_detailer().unwrap();
        assert_eq!((fd.denoise, fd.confidence), (0.4, 0.5));
        let again: GenerationParams =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(again, p);
    }

    fn with_cn(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.controlnet = Some(ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: "inputs/0190-abc.png".into(),
                model: "control_canny.safetensors".into(),
                preprocessor: Preprocessor::Canny,
                strength: 1.0,
                start_percent: 0.0,
                end_percent: 1.0,
            }],
        });
        p
    }

    fn unit(p: &mut GenerationParams) -> &mut ControlnetUnit {
        &mut p.controlnet.as_mut().unwrap().units[0]
    }

    #[test]
    fn controlnet_is_valid_for_sd_only() {
        with_cn(ModelKind::Sd15).validate().unwrap();
        with_cn(ModelKind::Sdxl).validate().unwrap();
        for k in NON_SD {
            assert_eq!(field_of(with_cn(k).validate()), "controlnet");
        }
        // 無効のままなら Flux でも通り、ユニットも作られない
        let mut p = with_cn(ModelKind::Flux);
        p.controlnet.as_mut().unwrap().enabled = false;
        p.validate().unwrap();
        assert!(p.active_controlnet_units().is_empty());
    }

    fn with_inpaint(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.inpaint = Some(InpaintParams {
            enabled: true,
            image: "inputs/a.png".into(),
            mask: "inputs/m.png".into(),
            denoise: 0.7,
            grow_mask: 8,
            feather: 8,
            composite_back: true,
        });
        p
    }

    #[test]
    fn inpaint_valid_for_sd_and_rejected_for_flux_or_with_face_detailer() {
        for k in [ModelKind::Sd15, ModelKind::Sdxl] {
            assert!(with_inpaint(k).validate().is_ok());
        }
        for k in NON_SD {
            assert_eq!(field_of(with_inpaint(k).validate()), "inpaint");
        }
        let mut p = with_inpaint(ModelKind::Sd15);
        p.face_detailer = Some(FaceDetailerParams {
            enabled: true,
            detector: "bbox/face.pt".into(),
            denoise: 0.4,
            prompt: String::new(),
            confidence: 0.5,
        });
        assert_eq!(field_of(p.validate()), "inpaint");
        // 無効なら何も検証しない(Flux でも通る)
        let mut off = with_inpaint(ModelKind::Flux);
        off.inpaint.as_mut().unwrap().enabled = false;
        assert!(off.validate().is_ok());
    }

    #[test]
    fn inpaint_rejects_bad_values() {
        type Tweak = fn(&mut InpaintParams);
        let cases: [(&str, Tweak); 6] = [
            ("inpaint.image", |i| i.image = "../x.png".into()),
            ("inpaint.mask", |i| i.mask = String::new()),
            ("inpaint.denoise", |i| i.denoise = 0.0),
            ("inpaint.denoise", |i| i.denoise = f64::NAN),
            ("inpaint.growMask", |i| i.grow_mask = 65),
            ("inpaint.feather", |i| i.feather = 32),
        ];
        for (field, f) in cases {
            let mut p = with_inpaint(ModelKind::Sd15);
            f(p.inpaint.as_mut().unwrap());
            assert_eq!(field_of(p.validate()), field);
        }
    }

    fn with_img2img(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.img2img = Some(Img2imgParams {
            enabled: true,
            image: "inputs/a.png".into(),
            denoise: 0.6,
        });
        p
    }

    #[test]
    fn img2img_valid_for_sd_and_rejected_for_other_kinds_or_with_inpaint() {
        for k in [ModelKind::Sd15, ModelKind::Sdxl] {
            assert!(with_img2img(k).validate().is_ok());
        }
        for k in NON_SD {
            assert_eq!(field_of(with_img2img(k).validate()), "img2img");
        }
        // 顔補正・ControlNet とは併用できる。Inpaint とは排他
        let mut p = with_img2img(ModelKind::Sd15);
        p.face_detailer = Some(FaceDetailerParams {
            enabled: true,
            detector: "bbox/face_yolov8m.pt".into(),
            denoise: 0.4,
            prompt: String::new(),
            confidence: 0.5,
        });
        assert!(p.validate().is_ok());
        p.face_detailer = None;
        p.inpaint = with_inpaint(ModelKind::Sd15).inpaint;
        assert_eq!(field_of(p.validate()), "img2img");
        // 無効なら、他の種別でも検証しない
        let mut off = with_img2img(ModelKind::Flux);
        off.img2img.as_mut().unwrap().enabled = false;
        assert!(off.validate().is_ok());
    }

    #[test]
    fn img2img_rejects_bad_values_and_counts_one_upload() {
        let mut p = with_img2img(ModelKind::Sd15);
        assert_eq!(p.upload_count(), 1);
        p.img2img.as_mut().unwrap().image = "../x.png".into();
        assert_eq!(field_of(p.validate()), "img2img.image");
        for d in [0.0, 1.01, f64::NAN] {
            let mut p = with_img2img(ModelKind::Sd15);
            p.img2img.as_mut().unwrap().denoise = d;
            assert_eq!(field_of(p.validate()), "img2img.denoise");
        }
        let i: Img2imgParams = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(i.denoise, 0.6);
    }

    fn with_reference(kind: ModelKind, images: &[&str]) -> GenerationParams {
        let mut p = sample(kind);
        p.reference = Some(ReferenceParams {
            enabled: true,
            images: images.iter().map(|s| (*s).to_string()).collect(),
            resolution: 1024,
        });
        p
    }

    #[test]
    fn reference_is_valid_only_for_qwen_image_and_counts_one_upload_per_image() {
        let p = with_reference(ModelKind::QwenImage, &["inputs/a.png", "inputs/b.png"]);
        assert!(p.validate().is_ok());
        assert_eq!(p.upload_count(), 2);
        for k in [
            ModelKind::Sd15,
            ModelKind::Sdxl,
            ModelKind::Flux,
            ModelKind::FluxGguf,
            ModelKind::Anima,
            ModelKind::Krea2,
        ] {
            assert_eq!(
                field_of(with_reference(k, &["inputs/a.png"]).validate()),
                "reference",
                "{k:?}"
            );
        }
        // 無効なら、他の種別でも検証しない
        let mut off = with_reference(ModelKind::Flux, &["inputs/a.png"]);
        off.reference.as_mut().unwrap().enabled = false;
        assert!(off.validate().is_ok());
        assert_eq!(off.upload_count(), 0);
    }

    #[test]
    fn reference_rejects_bad_counts_paths_and_resolutions() {
        let none = with_reference(ModelKind::QwenImage, &[]);
        assert_eq!(field_of(none.validate()), "reference.images");
        let five = with_reference(ModelKind::QwenImage, &["inputs/a.png"; 5]);
        assert_eq!(field_of(five.validate()), "reference.images");
        let four = with_reference(ModelKind::QwenImage, &["inputs/a.png"; 4]);
        assert!(four.validate().is_ok());
        let bad_path = with_reference(ModelKind::QwenImage, &["inputs/a.png", "../x.png"]);
        assert_eq!(field_of(bad_path.validate()), "reference.images");
        for res in [0, 512, 1000, 2048] {
            let mut p = with_reference(ModelKind::QwenImage, &["inputs/a.png"]);
            p.reference.as_mut().unwrap().resolution = res;
            assert_eq!(field_of(p.validate()), "reference.resolution", "{res}");
        }
        for res in REFERENCE_RESOLUTIONS {
            let mut p = with_reference(ModelKind::QwenImage, &["inputs/a.png"]);
            p.reference.as_mut().unwrap().resolution = res;
            assert!(p.validate().is_ok());
        }
        let r: ReferenceParams = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!((r.resolution, r.images.len()), (1024, 0));
    }

    #[test]
    fn reference_cannot_be_combined_with_other_image_inputs() {
        // Qwen-Image 2.1 では他の画像入力機能そのものが使えないので、どれを足しても拒否される
        let mut p = with_reference(ModelKind::QwenImage, &["inputs/a.png"]);
        p.img2img = with_img2img(ModelKind::QwenImage).img2img;
        assert!(p.validate().is_err());
        let mut p = with_reference(ModelKind::QwenImage, &["inputs/a.png"]);
        p.controlnet = with_cn(ModelKind::QwenImage).controlnet;
        assert!(p.validate().is_err());
        let mut p = with_reference(ModelKind::QwenImage, &["inputs/a.png"]);
        p.inpaint = with_inpaint(ModelKind::QwenImage).inpaint;
        assert!(p.validate().is_err());
    }

    fn with_face_swap(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.face_swap = Some(FaceSwapParams {
            enabled: true,
            target: "inputs/t.png".into(),
            face: "inputs/f.png".into(),
            keep_outside: true,
            region: FaceSwapRegion::Head,
            resolution: 1024,
            detector: FACE_SWAP_DETECTOR.into(),
        });
        p
    }

    #[test]
    fn face_swap_allows_an_empty_prompt_but_other_features_do_not() {
        let mut p = with_face_swap(ModelKind::QwenImage);
        p.prompt = "  ".into();
        assert!(p.validate().is_ok());
        p.face_swap = None;
        assert_eq!(field_of(p.validate()), "prompt");
    }

    #[test]
    fn face_swap_is_valid_only_for_qwen_image_and_counts_two_uploads() {
        let p = with_face_swap(ModelKind::QwenImage);
        assert!(p.validate().is_ok());
        assert_eq!(p.upload_count(), 2);
        for k in [
            ModelKind::Sd15,
            ModelKind::Sdxl,
            ModelKind::Flux,
            ModelKind::FluxGguf,
            ModelKind::Anima,
            ModelKind::Krea2,
        ] {
            assert_eq!(field_of(with_face_swap(k).validate()), "faceSwap", "{k:?}");
        }
        let mut off = with_face_swap(ModelKind::Flux);
        off.face_swap.as_mut().unwrap().enabled = false;
        assert!(off.validate().is_ok());
        assert_eq!(off.upload_count(), 0);
    }

    #[test]
    fn face_swap_rejects_bad_paths_resolutions_detectors_and_batches() {
        for (field, edit) in [
            (
                "faceSwap.target",
                (|f: &mut FaceSwapParams| f.target = "../x.png".into()) as fn(&mut FaceSwapParams),
            ),
            ("faceSwap.face", |f| f.face = String::new()),
            ("faceSwap.resolution", |f| f.resolution = 2048),
            ("faceSwap.detector", |f| f.detector = "segm/x.pt".into()),
        ] {
            let mut p = with_face_swap(ModelKind::QwenImage);
            edit(p.face_swap.as_mut().unwrap());
            assert_eq!(field_of(p.validate()), field);
        }
        // 検出器は、合成しないなら見ない
        let mut p = with_face_swap(ModelKind::QwenImage);
        let f = p.face_swap.as_mut().unwrap();
        f.keep_outside = false;
        f.detector = String::new();
        assert!(p.validate().is_ok());
        // 1 ジョブ 1 枚
        let mut p = with_face_swap(ModelKind::QwenImage);
        p.batch_size = 2;
        assert_eq!(field_of(p.validate()), "batchSize");
    }

    #[test]
    fn face_swap_cannot_be_combined_with_reference_or_other_image_inputs() {
        let mut p = with_face_swap(ModelKind::QwenImage);
        p.reference = with_reference(ModelKind::QwenImage, &["inputs/a.png"]).reference;
        assert!(p.validate().is_err());
        // 参照画像の側からも拒否される(どちらから見ても排他)
        assert_eq!(field_of(p.validate()), "reference");
        let mut p = with_face_swap(ModelKind::QwenImage);
        p.img2img = with_img2img(ModelKind::QwenImage).img2img;
        assert!(p.validate().is_err());
    }

    #[test]
    fn face_swap_defaults_apply_to_a_minimal_json() {
        let f: FaceSwapParams = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(f.keep_outside);
        assert_eq!(f.region, FaceSwapRegion::Head);
        assert_eq!(f.resolution, 1024);
        assert_eq!(f.detector, FACE_SWAP_DETECTOR);
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["keepOutside"], true);
        assert_eq!(json["region"], "head");
    }

    #[test]
    fn upload_count_covers_controlnet_and_inpaint() {
        assert_eq!(sample(ModelKind::Sd15).upload_count(), 0);
        let mut p = with_inpaint(ModelKind::Sd15);
        assert_eq!(p.upload_count(), 2);
        p.controlnet = with_cn(ModelKind::Sd15).controlnet;
        assert_eq!(p.upload_count(), 3);
    }

    #[test]
    fn inpaint_defaults_apply_to_a_minimal_json() {
        let ip: InpaintParams = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(
            (ip.denoise, ip.grow_mask, ip.feather, ip.composite_back),
            (0.7, 8, 8, true)
        );
    }

    #[test]
    fn controlnet_rejects_bad_values() {
        let mut p = with_cn(ModelKind::Sd15);
        p.controlnet.as_mut().unwrap().units.clear();
        assert_eq!(field_of(p.validate()), "controlnet.units");

        let mut p = with_cn(ModelKind::Sd15);
        let extra = unit(&mut p).clone();
        p.controlnet.as_mut().unwrap().units.push(extra);
        assert_eq!(field_of(p.validate()), "controlnet.units");

        let mut p = with_cn(ModelKind::Sd15);
        unit(&mut p).image = String::new();
        assert_eq!(field_of(p.validate()), "controlnet.units[0].image");

        let mut p = with_cn(ModelKind::Sd15);
        unit(&mut p).model = " ".into();
        assert_eq!(field_of(p.validate()), "controlnet.units[0].model");

        for bad in [-0.1, 2.1, f64::NAN] {
            let mut p = with_cn(ModelKind::Sd15);
            unit(&mut p).strength = bad;
            assert_eq!(field_of(p.validate()), "controlnet.units[0].strength");
        }
        for (s, e) in [(0.5, 0.5), (0.8, 0.2), (-0.1, 1.0), (0.0, 1.1)] {
            let mut p = with_cn(ModelKind::Sd15);
            unit(&mut p).start_percent = s;
            unit(&mut p).end_percent = e;
            assert_eq!(field_of(p.validate()), "controlnet.units[0].endPercent");
        }
    }

    #[test]
    fn input_paths_must_stay_inside_inputs_dir() {
        for ok in ["inputs/a.png", "inputs/0190-ab_c.PNG"] {
            assert!(is_valid_input_path(ok), "{ok}");
        }
        for bad in [
            "",
            "inputs/",
            "a.png",
            "../inputs/a.png",
            "inputs/../a.png",
            "inputs/..",
            "inputs/.hidden",
            "inputs/sub/a.png",
            "inputs\\a.png",
            "/etc/passwd",
            "inputs/C:evil.png",
            "2026/11/a.png",
            "thumbs/a.jpg",
        ] {
            assert!(!is_valid_input_path(bad), "{bad}");
        }
        let mut p = with_cn(ModelKind::Sd15);
        unit(&mut p).image = "inputs/../secret.png".into();
        assert_eq!(field_of(p.validate()), "controlnet.units[0].image");
    }

    #[test]
    fn controlnet_is_optional_and_round_trips() {
        let v = serde_json::to_value(sample(ModelKind::Sd15)).unwrap();
        assert!(v.get("controlnet").is_none());

        let p: GenerationParams = serde_json::from_str(
            r#"{"model":{"kind":"sd15","name":"m"},"prompt":"x","width":512,"height":512,
                "steps":20,"sampler":"euler","scheduler":"normal","seed":1,
                "controlnet":{"enabled":true,"units":[{"image":"inputs/a.png","model":"c.safetensors",
                  "preprocessor":"openpose"}]}}"#,
        )
        .unwrap();
        let u = &p.active_controlnet_units()[0];
        assert_eq!(u.preprocessor, Preprocessor::Openpose);
        assert_eq!(
            (u.strength, u.start_percent, u.end_percent),
            (1.0, 0.0, 1.0)
        );
        let again: GenerationParams =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(again, p);
        assert_eq!(
            serde_json::to_value(Preprocessor::None).unwrap(),
            serde_json::json!("none")
        );
    }

    #[test]
    fn deserializes_with_defaults() {
        let p: GenerationParams = serde_json::from_str(
            r#"{"model":{"kind":"sdxl","name":"m.safetensors"},"prompt":"x",
                "width":1024,"height":1024,"steps":25,"sampler":"euler","scheduler":"normal","seed":1}"#,
        )
        .unwrap();
        assert_eq!(p.schema_version, 1);
        assert_eq!(p.batch_size, 1);
        assert_eq!(p.cfg, 7.0);
        assert!(!p.seed_randomize && p.negative_prompt.is_empty());
        // 往復で値が保たれる
        let again: GenerationParams =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(again, p);
    }
}
