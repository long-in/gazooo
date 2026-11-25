//! 画像生成エンジンの抽象(docs/02-architecture.md §3.3)。
//!
//! 接続確認・モデル一覧に加え、生成の投入・完了待ち(進捗通知つき)・キャンセルを提供する。

pub mod comfyui;

use async_trait::async_trait;
use serde::Serialize;

use crate::domain::job::ProgressPhase;
use crate::domain::params::{GenerationParams, Preprocessor};
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineFailure {
    Unreachable,
    BadResponse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineFeatures {
    /// ComfyUI-GGUF(`UnetLoaderGGUF`)が導入されているか。
    pub gguf_node: bool,
    /// 顔補正に必要なノード(Impact Pack の `FaceDetailer` と、Subpack の
    /// `UltralyticsDetectorProvider`)が導入されているか(docs/10 §3)。
    pub face_detailer: bool,
    /// 利用できる ControlNet の前処理(`Preprocessor::None` は含めない。docs/10 §3)。
    pub controlnet_preprocessors: Vec<Preprocessor>,
    /// Inpaint に必要なコアノードがすべて使えるか(docs/13 §3.3)。
    pub inpaint: bool,
    /// 画像から生成(img2img)に必要なコアノードがすべて使えるか(docs/19 §5)。
    pub img2img: bool,
    /// 参照画像つき生成(Qwen-Image 2.1 の画像編集)に使えるエンコーダ定義があるか(docs/24 §4.2)。
    pub reference_edit: bool,
    /// 顔の入れ替えの本体に使えるか: 参照画像つき生成(`reference_edit`)と、LoRA の適用ノード(`LoraLoaderModelOnly`)、
    /// 顔入れ替え LoRA の導入(docs/42 §6.1)。
    pub face_swap: bool,
    /// 顔の入れ替えの後段合成(顔以外を元のまま保つ)に必要なノードがすべて使えるか(docs/42 §5)。
    pub face_swap_composite: bool,
    /// 画像のタグ付け(WD14 Tagger。`WD14Tagger|pysssss`)が導入されているか(docs/39 §3)。
    pub image_tag: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub connected: bool,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vram_free: Option<u64>,
    pub features: EngineFeatures,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EngineFailure>,
}

impl EngineStatus {
    pub fn disconnected(url: impl Into<String>, error: EngineFailure) -> Self {
        Self {
            connected: false,
            url: url.into(),
            version: None,
            device: None,
            vram_free: None,
            features: EngineFeatures::default(),
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineModels {
    pub checkpoints: Vec<String>,
    pub unets: Vec<String>,
    pub gguf_unets: Vec<String>,
    pub clips: Vec<String>,
    pub vaes: Vec<String>,
    pub loras: Vec<String>,
    /// `UltralyticsDetectorProvider` の `bbox/…`(ノード未導入なら空)。
    pub face_detectors: Vec<String>,
    /// `ControlNetLoader.control_net_name`。
    pub controlnets: Vec<String>,
    pub samplers: Vec<String>,
    pub schedulers: Vec<String>,
    /// `WD14Tagger|pysssss.model` の候補(ノード未導入なら空。docs/39 §6.1)。
    pub taggers: Vec<String>,
}

/// 画像のタグ付けの指定(docs/39 §4.2。値の範囲は呼び出し側で検証済み)。
#[derive(Debug, Clone, PartialEq)]
pub struct TagRequest {
    pub model: String,
    pub threshold: f64,
    pub character_threshold: f64,
    pub exclude_tags: String,
}

/// 生成中の進捗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub step: u32,
    pub total: u32,
    pub node: Option<String>,
    pub phase: Option<ProgressPhase>,
}

/// 生成結果の画像 1 枚(エンジンから取得済みの PNG など)。
#[derive(Debug, Clone)]
pub struct ImageData {
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[async_trait]
pub trait GenerationEngine: Send + Sync {
    fn url(&self) -> String;
    /// 接続できれば `connected = true` の状態を返す。接続不能・非互換は `Err`。
    async fn health(&self) -> AppResult<EngineStatus>;
    async fn list_models(&self) -> AppResult<EngineModels>;
    /// エンジンの起動引数(`/system_stats` の `argv`)。モデルフォルダの検出に使う(docs/12 §3)。
    /// 取得できないエンジンは空を返す。
    async fn launch_args(&self) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// 参照画像(PNG)をエンジンへ送り、ワークフローから参照する名前を返す(ControlNet)。
    async fn upload_input(&self, png: Vec<u8>) -> AppResult<String>;
    /// 画像(PNG)をタグ付けし、タグ列(カンマ + スペース区切り)を返す(docs/39 §5)。
    /// 未対応のエンジンは、ノード未導入として扱う。
    async fn tag_image(&self, _png: Vec<u8>, _req: &TagRequest) -> AppResult<String> {
        Err(crate::error::AppError::EngineNodeMissing(
            "WD14Tagger|pysssss".into(),
        ))
    }
    /// 生成ジョブを投入し、エンジン側の ID を返す。
    /// `inputs` は有効な ControlNet ユニットの順に並べた、`upload_input` が返した名前。
    async fn submit(&self, params: &GenerationParams, inputs: &[String]) -> AppResult<String>;
    /// 完了まで待ち、結果画像を取得して返す。進捗があれば `on_progress` を呼ぶ。
    /// この Future を drop すれば待機を中止できる(エンジン側の停止は `cancel`)。
    async fn wait(
        &self,
        remote_id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> AppResult<Vec<ImageData>>;
    /// 待機中のジョブを取り消し、実行中なら中断する。
    async fn cancel(&self, remote_id: &str) -> AppResult<()>;
}
