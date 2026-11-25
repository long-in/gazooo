//! 生成パラメータから ComfyUI の API 形式ワークフロー(プロンプト JSON)を組み立てる。
//! docs/02-architecture.md §3.4。

use serde_json::{Value, json};

use crate::domain::params::{
    ControlnetUnit, FaceDetailerParams, GenerationParams, Img2imgParams, InpaintParams, ModelKind,
    Preprocessor,
};

/// 顔補正(`FaceDetailer`)のノード ID。進捗の段階判定(generate.rs)でも使う。
pub const FACE_DETAILER_NODE: &str = "22";
/// 顔検出モデルのローダー。
const FACE_DETECTOR_NODE: &str = "20";
/// 補正用プロンプトのエンコード(指定時のみ)。
const FACE_PROMPT_NODE: &str = "21";

/// ノード出力へのリンク `[node_id, output_index]`。
fn link(node: &str, index: u32) -> Value {
    json!([node, index])
}

fn node(class_type: &str, inputs: Value) -> Value {
    json!({ "class_type": class_type, "inputs": inputs })
}

/// `control_images` は有効な ControlNet ユニットの順に並べた、エンジンへアップロード済みの画像名。
pub fn build(params: &GenerationParams, filename_prefix: &str, control_images: &[String]) -> Value {
    match params.model.kind {
        ModelKind::Flux | ModelKind::FluxGguf => build_flux(params, filename_prefix),
        ModelKind::Anima => build_split(
            params,
            filename_prefix,
            "stable_diffusion",
            Conditioning::Plain,
        ),
        ModelKind::QwenImage => {
            let mut wf = build_split(
                params,
                filename_prefix,
                "qwen_image",
                Conditioning::QwenImage21,
            );
            if let Some(r) = params.active_reference() {
                add_reference(&mut wf, params, r, control_images);
            }
            if let Some(f) = params.active_face_swap() {
                add_face_swap(&mut wf, params, f, control_images);
            }
            wf
        }
        ModelKind::Krea2 => build_split(params, filename_prefix, "krea2", Conditioning::ZeroOut),
        ModelKind::Sd15 | ModelKind::Sdxl => build_sd(params, filename_prefix, control_images),
    }
}

/// ユニット `i` のノード ID の起点。LoadImage、ImageScale、前処理、ControlNetLoader、
/// ControlNetApplyAdvanced の順に連番を振る(顔補正の 20〜22 と重ならない)。
fn controlnet_base(i: usize) -> usize {
    30 + 10 * i
}

/// SD 1.5 / SDXL(チェックポイント一体型)。
fn build_sd(p: &GenerationParams, prefix: &str, control_images: &[String]) -> Value {
    // VAE を別指定した場合のみ VAELoader を使う
    let (vae_link, extra_vae) = match &p.model.vae {
        Some(vae) if !vae.trim().is_empty() => (
            link("7", 0),
            Some(node("VAELoader", json!({ "vae_name": vae }))),
        ),
        _ => (link("1", 2), None),
    };

    let mut wf = json!({
        "1": node("CheckpointLoaderSimple", json!({ "ckpt_name": p.model.name })),
        "2": node("CLIPTextEncode", json!({ "text": p.prompt, "clip": link("1", 1) })),
        "3": node("CLIPTextEncode", json!({ "text": p.negative_prompt, "clip": link("1", 1) })),
        "4": node("EmptyLatentImage", json!({
            "width": p.width, "height": p.height, "batch_size": p.batch_size
        })),
        "5": node("KSampler", json!({
            "model": link("1", 0),
            "seed": p.seed,
            "steps": p.steps,
            "cfg": p.cfg,
            "sampler_name": p.sampler,
            "scheduler": p.scheduler,
            "positive": link("2", 0),
            "negative": link("3", 0),
            "latent_image": link("4", 0),
            "denoise": 1.0
        })),
        "6": node("VAEDecode", json!({ "samples": link("5", 0), "vae": vae_link })),
        "8": node("SaveImage", json!({ "images": link("6", 0), "filename_prefix": prefix })),
    });
    if let Some(vae_node) = extra_vae {
        wf["7"] = vae_node;
    }
    // ControlNet は本体のサンプリングにだけ効かせる(顔補正は元の conditioning を使う)
    add_controlnet(&mut wf, p, control_images);
    if let Some(ip) = p.active_inpaint() {
        // 画像の並びは ControlNet のユニットの後ろ: [画像, マスク]
        let n = p.active_controlnet_units().len();
        let name = |i: usize| control_images.get(n + i).cloned().unwrap_or_default();
        add_inpaint(&mut wf, p, ip, &vae_link, &name(0), &name(1));
    }
    if let Some(i2i) = p.active_img2img() {
        // 画像の並びは ControlNet のユニットの後ろ(Inpaint とは排他なので同じ位置)
        let n = p.active_controlnet_units().len();
        let image = control_images.get(n).cloned().unwrap_or_default();
        add_img2img(&mut wf, p, i2i, &vae_link, &image);
    }
    if let Some(fd) = p.active_face_detailer() {
        add_face_detailer(&mut wf, p, fd, &vae_link);
    }
    wf
}

/// img2img の最初のノード ID(顔補正 20〜、ControlNet 30〜、Inpaint 60〜 と重ならない)。
const IMG2IMG_BASE: usize = 80;

/// 元画像を生成サイズへ合わせて VAE エンコードし、空の潜在の代わりにサンプリングする(docs/19 §5)。
fn add_img2img(
    wf: &mut Value,
    p: &GenerationParams,
    i2i: &Img2imgParams,
    vae: &Value,
    image: &str,
) {
    let id = |k: usize| (IMG2IMG_BASE + k).to_string();
    wf[id(0)] = node("LoadImage", json!({ "image": image }));
    wf[id(1)] = node(
        "ImageScale",
        json!({
            "image": link(&id(0), 0),
            "upscale_method": "lanczos",
            "width": p.width,
            "height": p.height,
            "crop": "disabled",
        }),
    );
    wf[id(2)] = node(
        "VAEEncode",
        json!({ "pixels": link(&id(1), 0), "vae": vae }),
    );
    let mut latent = link(&id(2), 0);
    if p.batch_size > 1 {
        wf[id(3)] = node(
            "RepeatLatentBatch",
            json!({ "samples": latent, "amount": p.batch_size }),
        );
        latent = link(&id(3), 0);
    }
    wf["5"]["inputs"]["latent_image"] = latent;
    wf["5"]["inputs"]["denoise"] = json!(i2i.denoise);
}

/// Inpaint の最初のノード ID(ControlNet の 30〜、顔補正の 20〜22 と重ならない)。
const INPAINT_BASE: usize = 60;

/// 参照画像を VAE エンコードして、マスク範囲だけをサンプリングし直す(docs/13 §3.3)。
/// マスクは生成サイズへ合わせ、拡張・ぼかしを掛ける。結果は元画像へ合成し直せる。
/// コアノードだけで組む。
fn add_inpaint(
    wf: &mut Value,
    p: &GenerationParams,
    ip: &InpaintParams,
    vae: &Value,
    image: &str,
    mask: &str,
) {
    let id = |k: usize| (INPAINT_BASE + k).to_string();
    let batch = p.batch_size;

    // 参照画像(生成サイズ。比率が違っても引き伸ばして全体を使う)
    wf[id(0)] = node("LoadImage", json!({ "image": image }));
    wf[id(1)] = node(
        "ImageScale",
        json!({
            "image": link(&id(0), 0),
            "upscale_method": "lanczos",
            "width": p.width,
            "height": p.height,
            "crop": "disabled",
        }),
    );
    wf[id(2)] = node(
        "VAEEncode",
        json!({ "pixels": link(&id(1), 0), "vae": vae }),
    );
    let mut latent = link(&id(2), 0);
    if batch > 1 {
        wf[id(3)] = node(
            "RepeatLatentBatch",
            json!({ "samples": latent, "amount": batch }),
        );
        latent = link(&id(3), 0);
    }

    // マスク: 生成サイズへ → 拡張 → ぼかし
    wf[id(4)] = node("LoadImageMask", json!({ "image": mask, "channel": "red" }));
    wf[id(5)] = node("MaskToImage", json!({ "mask": link(&id(4), 0) }));
    wf[id(6)] = node(
        "ImageScale",
        json!({
            "image": link(&id(5), 0),
            "upscale_method": "bilinear",
            "width": p.width,
            "height": p.height,
            "crop": "disabled",
        }),
    );
    wf[id(7)] = node(
        "ImageToMask",
        json!({ "image": link(&id(6), 0), "channel": "red" }),
    );
    let mut mask_link = link(&id(7), 0);
    if ip.grow_mask > 0 {
        wf[id(8)] = node(
            "GrowMask",
            json!({ "mask": mask_link, "expand": ip.grow_mask, "tapered_corners": true }),
        );
        mask_link = link(&id(8), 0);
    }
    if ip.feather > 0 {
        wf[id(9)] = node("MaskToImage", json!({ "mask": mask_link }));
        wf[id(10)] = node(
            "ImageBlur",
            json!({
                "image": link(&id(9), 0),
                "blur_radius": ip.feather,
                "sigma": (f64::from(ip.feather) / 2.0).clamp(0.1, 10.0),
            }),
        );
        wf[id(11)] = node(
            "ImageToMask",
            json!({ "image": link(&id(10), 0), "channel": "red" }),
        );
        mask_link = link(&id(11), 0);
    }
    wf[id(12)] = node(
        "SetLatentNoiseMask",
        json!({ "samples": latent, "mask": mask_link }),
    );
    wf["5"]["inputs"]["latent_image"] = link(&id(12), 0);
    wf["5"]["inputs"]["denoise"] = json!(ip.denoise);

    // 元画像への合成(マスク外を元画像と一致させる)
    if ip.composite_back {
        let mut dest = link(&id(1), 0);
        if batch > 1 {
            wf[id(13)] = node(
                "RepeatImageBatch",
                json!({ "image": dest, "amount": batch }),
            );
            dest = link(&id(13), 0);
        }
        wf[id(14)] = node(
            "ImageCompositeMasked",
            json!({
                "destination": dest,
                "source": link("6", 0),
                "x": 0,
                "y": 0,
                "resize_source": false,
                "mask": mask_link,
            }),
        );
        wf["8"]["inputs"]["images"] = link(&id(14), 0);
    }
}

/// 参照画像を生成サイズへ中央クロップでリサイズし、前処理を通して ControlNet を適用する。
/// KSampler の positive / negative を、適用後の conditioning に付け替える。
fn add_controlnet(wf: &mut Value, p: &GenerationParams, images: &[String]) {
    let mut positive = link("2", 0);
    let mut negative = link("3", 0);
    for (i, unit) in p.active_controlnet_units().iter().enumerate() {
        let base = controlnet_base(i);
        let id = |k: usize| (base + k).to_string();
        wf[id(0)] = node(
            "LoadImage",
            json!({ "image": images.get(i).cloned().unwrap_or_default() }),
        );
        wf[id(1)] = node(
            "ImageScale",
            json!({
                "image": link(&id(0), 0),
                "upscale_method": "lanczos",
                "width": p.width,
                "height": p.height,
                "crop": "center",
            }),
        );
        // 前処理なしなら、リサイズした参照画像をそのまま条件にする
        let hint = match preprocessor_node(unit, p, link(&id(1), 0)) {
            Some(pre) => {
                wf[id(2)] = pre;
                link(&id(2), 0)
            }
            None => link(&id(1), 0),
        };
        wf[id(3)] = node(
            "ControlNetLoader",
            json!({ "control_net_name": unit.model }),
        );
        wf[id(4)] = node(
            "ControlNetApplyAdvanced",
            json!({
                "positive": positive,
                "negative": negative,
                "control_net": link(&id(3), 0),
                "image": hint,
                "strength": unit.strength,
                "start_percent": unit.start_percent,
                "end_percent": unit.end_percent,
            }),
        );
        positive = link(&id(4), 0);
        negative = link(&id(4), 1);
    }
    wf["5"]["inputs"]["positive"] = positive;
    wf["5"]["inputs"]["negative"] = negative;
}

/// 前処理ノード(Canny はコア、Depth / OpenPose は comfyui_controlnet_aux)。
fn preprocessor_node(unit: &ControlnetUnit, p: &GenerationParams, image: Value) -> Option<Value> {
    // 前処理の解像度は短辺に合わせる(64 の倍数。生成サイズが 64 の倍数のため)
    let resolution = p.width.min(p.height);
    match unit.preprocessor {
        Preprocessor::Canny => Some(node(
            "Canny",
            json!({ "image": image, "low_threshold": 0.4, "high_threshold": 0.8 }),
        )),
        Preprocessor::Depth => Some(node(
            "DepthAnythingV2Preprocessor",
            json!({
                "image": image,
                "ckpt_name": "depth_anything_v2_vitl.pth",
                "resolution": resolution,
            }),
        )),
        Preprocessor::Openpose => Some(node(
            "OpenposePreprocessor",
            json!({
                "image": image,
                "detect_hand": "enable",
                "detect_body": "enable",
                "detect_face": "enable",
                "resolution": resolution,
                "scale_stick_for_xinsr_cn": "disable",
            }),
        )),
        Preprocessor::None => None,
    }
}

/// VAEDecode の後ろに顔補正を差し込み、SaveImage の入力を補正後の画像に付け替える。
/// 検出されなければ `FaceDetailer` は元の画像をそのまま返す。
/// ステップ・CFG・サンプラー・シードは本体の値を引き継ぐ。ほかの引数は固定値(docs/10 §4.2)。
fn add_face_detailer(wf: &mut Value, p: &GenerationParams, fd: &FaceDetailerParams, vae: &Value) {
    // SDXL は 1024px 級で学習されているため、切り出し領域の目標サイズを大きくする
    let (guide_size, max_size) = if p.model.kind == ModelKind::Sdxl {
        (1024, 1536)
    } else {
        (512, 1024)
    };
    let positive = if fd.prompt.trim().is_empty() {
        link("2", 0)
    } else {
        wf[FACE_PROMPT_NODE] = node(
            "CLIPTextEncode",
            json!({ "text": fd.prompt, "clip": link("1", 1) }),
        );
        link(FACE_PROMPT_NODE, 0)
    };
    wf[FACE_DETECTOR_NODE] = node(
        "UltralyticsDetectorProvider",
        json!({ "model_name": fd.detector }),
    );
    wf[FACE_DETAILER_NODE] = node(
        "FaceDetailer",
        json!({
            "image": link("6", 0),
            "model": link("1", 0),
            "clip": link("1", 1),
            "vae": vae,
            "guide_size": guide_size,
            "guide_size_for": true,
            "max_size": max_size,
            "seed": p.seed,
            "steps": p.steps,
            "cfg": p.cfg,
            "sampler_name": p.sampler,
            "scheduler": p.scheduler,
            "positive": positive,
            "negative": link("3", 0),
            "denoise": fd.denoise,
            "feather": 5,
            "noise_mask": true,
            "force_inpaint": true,
            "bbox_threshold": fd.confidence,
            "bbox_dilation": 10,
            "bbox_crop_factor": 3.0,
            // SAM は使わない(下の 4 項目は必須入力のため既定値を渡す)
            "sam_detection_hint": "center-1",
            "sam_dilation": 0,
            "sam_threshold": 0.93,
            "sam_bbox_expansion": 0,
            "sam_mask_hint_threshold": 0.7,
            "sam_mask_hint_use_negative": "False",
            "drop_size": 10,
            "bbox_detector": link(FACE_DETECTOR_NODE, 0),
            "wildcard": "",
            "cycle": 1,
        }),
    );
    wf["8"]["inputs"]["images"] = link(FACE_DETAILER_NODE, 0);
}

/// Flux.1(通常 / GGUF)。ネガティブプロンプトは使わない(cfg = 1.0 固定、FluxGuidance で誘導)。
fn build_flux(p: &GenerationParams, prefix: &str) -> Value {
    let is_gguf = p.model.kind == ModelKind::FluxGguf;
    let clip1 = p.model.clip.first().cloned().unwrap_or_default(); // T5
    let clip2 = p.model.clip.get(1).cloned().unwrap_or_default(); // CLIP-L
    let gguf_clip = clip1.ends_with(".gguf") || clip2.ends_with(".gguf");

    let unet = if is_gguf {
        node("UnetLoaderGGUF", json!({ "unet_name": p.model.name }))
    } else {
        node(
            "UNETLoader",
            json!({ "unet_name": p.model.name, "weight_dtype": "default" }),
        )
    };
    let clip_inputs = json!({ "clip_name1": clip1, "clip_name2": clip2, "type": "flux" });
    let clip_loader = if gguf_clip {
        node("DualCLIPLoaderGGUF", clip_inputs)
    } else {
        node("DualCLIPLoader", clip_inputs)
    };

    json!({
        "1": unet,
        "2": clip_loader,
        "3": node("VAELoader", json!({ "vae_name": p.model.vae.clone().unwrap_or_default() })),
        "4": node("CLIPTextEncode", json!({ "text": p.prompt, "clip": link("2", 0) })),
        "5": node("FluxGuidance", json!({ "conditioning": link("4", 0), "guidance": p.guidance })),
        "6": node("ConditioningZeroOut", json!({ "conditioning": link("4", 0) })),
        "7": node("EmptySD3LatentImage", json!({
            "width": p.width, "height": p.height, "batch_size": p.batch_size
        })),
        "8": node("KSampler", json!({
            "model": link("1", 0),
            "seed": p.seed,
            "steps": p.steps,
            "cfg": 1.0,
            "sampler_name": p.sampler,
            "scheduler": p.scheduler,
            "positive": link("5", 0),
            "negative": link("6", 0),
            "latent_image": link("7", 0),
            "denoise": 1.0
        })),
        "9": node("VAEDecode", json!({ "samples": link("8", 0), "vae": link("3", 0) })),
        "10": node("SaveImage", json!({ "images": link("9", 0), "filename_prefix": prefix })),
    })
}

/// 参照画像の `LoadImage` の最初のノード ID(顔補正 20〜、ControlNet 30〜、Inpaint 60〜、img2img 80〜 と重ならない)。
const REFERENCE_BASE: usize = 90;
/// 枚数が 2 以上のときの `RepeatLatentBatch`。`LoadImage`(90〜93)の後ろ。
const REFERENCE_REPEAT_NODE: &str = "100";

/// Qwen-Image 2.1 の参照画像つき生成(docs/24 §4.3)。`TextEncodeQwenImage21` に参照画像(`images.image_N`)と
/// VAE をつなぎ、空の潜在の代わりにそのノードの `latent` 出力(1 枚目の参照画像のサイズ)でサンプリングする。
/// `images` は有効な参照画像の順に並べた、エンジンへアップロード済みの画像名。
fn add_reference(
    wf: &mut Value,
    p: &GenerationParams,
    r: &crate::domain::params::ReferenceParams,
    images: &[String],
) {
    // 画像の並びは ControlNet・Inpaint・img2img の後ろ(参照画像は他の画像入力と排他なので先頭になる)
    let skip = p.upload_count() - r.images.len();
    for i in 0..r.images.len() {
        let id = (REFERENCE_BASE + i).to_string();
        wf[&id] = node(
            "LoadImage",
            json!({ "image": images.get(skip + i).cloned().unwrap_or_default() }),
        );
        wf["4"]["inputs"][format!("images.image_{}", i + 1)] = link(&id, 0);
    }
    wf["4"]["inputs"]["vae"] = link("3", 0);
    wf["4"]["inputs"]["resolution"] = json!(r.resolution);

    let mut latent = link("4", 2);
    if p.batch_size > 1 {
        wf[REFERENCE_REPEAT_NODE] = node(
            "RepeatLatentBatch",
            json!({ "samples": latent, "amount": p.batch_size }),
        );
        latent = link(REFERENCE_REPEAT_NODE, 0);
    }
    wf["7"]["inputs"]["latent_image"] = latent;
    // 出力サイズは 1 枚目の参照画像で決まる。空の潜在はどこにもつながらないので外す
    if let Some(nodes) = wf.as_object_mut() {
        nodes.remove("6");
    }
}

/// 顔の入れ替えの LoRA の適用ノード。
const FACE_SWAP_LORA_NODE: &str = "110";
/// 顔の入れ替えの後段合成のノードの起点(111〜119。参照画像 90〜94・100 と重ならない)。
const FACE_SWAP_BASE: usize = 111;
/// 顔の有無の事前確認(`build_face_probe`)で、数を出す出力ノードの ID。
pub(super) const FACE_PROBE_OUTPUT: &str = "4";

/// 顔の入れ替えの前に、対象画像に顔があるかだけを調べる軽いワークフロー(モデル不要。docs/46 §6 E)。
/// 顔のない画像で生成を始めると ComfyUI が止まることがあるため、生成の前に 0 件を弾く。
/// 結果は出力ノード(`FACE_PROBE_OUTPUT`)の数として履歴に出る。
pub(super) fn build_face_probe(image: &str, detector: &str) -> Value {
    json!({
        "1": node("LoadImage", json!({ "image": image })),
        "2": node("UltralyticsDetectorProvider", json!({ "model_name": detector })),
        "3": node("BboxDetectorSEGS", json!({
            "bbox_detector": link("2", 0),
            "image": link("1", 0),
            "threshold": 0.5,
            "dilation": 10,
            "crop_factor": 1.0,
            "drop_size": 10,
            "labels": "all"
        })),
        // "4" = FACE_PROBE_OUTPUT
        "4": node("PreviewAny", json!({ "source": link("5", 0) })),
        "5": node("ImpactCount_Elts_in_SEGS", json!({ "segs": link("3", 0) })),
    })
}

/// 「頭」のマスクを下方向へ広げるノード(docs/46 §5)。111〜119 の次の ID。
const FACE_SWAP_EXTEND_NODE: &str = "120";
/// 検出した顔の数を数えるノードと、その数を履歴へ出す出力ノード(`PreviewAny`)。
/// 0 のとき、アプリは `FACE_NOT_DETECTED` で失敗にする(docs/46 §6 E)。
const FACE_SWAP_COUNT_NODE: &str = "121";
pub(super) const FACE_SWAP_COUNT_OUTPUT: &str = "122";
/// 「頭」のマスクを下方向へ広げる量(px。顔画像の髪が長いときの切れ対策)。
const FACE_SWAP_HEAD_EXTEND_DOWN: u32 = 60;

/// 顔の入れ替え(docs/42 §4.4・§5)。参照画像つき生成(`images` = [対象, 顔])に、顔入れ替え LoRA を 1 つ適用し、
/// `keep_outside` なら生成結果の顔(頭)の領域だけを元の対象画像へ貼り戻す。
/// `images` は [対象, 顔] の順に、エンジンへアップロード済みの画像名。
fn add_face_swap(
    wf: &mut Value,
    p: &GenerationParams,
    f: &crate::domain::params::FaceSwapParams,
    images: &[String],
) {
    use crate::domain::params::{FACE_SWAP_LORA, FaceSwapRegion, ReferenceParams};

    // 参照画像つき生成と同じ骨格(1 枚目 = 対象、2 枚目 = 顔)。LoadImage は REFERENCE_BASE(対象)と +1(顔)
    let reference = ReferenceParams {
        enabled: true,
        images: vec![f.target.clone(), f.face.clone()],
        resolution: f.resolution,
    };
    add_reference(wf, p, &reference, images);

    // LoRA を UNET と KSampler の間に挟む(強さ 1.0 固定)
    wf[FACE_SWAP_LORA_NODE] = node(
        "LoraLoaderModelOnly",
        json!({ "model": link("1", 0), "lora_name": FACE_SWAP_LORA, "strength_model": 1.0 }),
    );
    wf["7"]["inputs"]["model"] = link(FACE_SWAP_LORA_NODE, 0);

    if !f.keep_outside {
        return;
    }

    // 後段合成: 元の対象画像から顔を検出し(最も大きい 1 つ)、マスクを広げてぼかし、
    // 生成画像を元のサイズへ戻して、マスクの内側だけを貼り戻す
    let target = REFERENCE_BASE.to_string();
    let id = |k: usize| (FACE_SWAP_BASE + k).to_string();
    // (検出領域の倍率、マスクの拡大 px、ぼかしのカーネル、下方向へ広げる px)
    // 頭は、顔画像の髪が長いと貼り戻す範囲の外へ伸びて途中で切れるため、下側を広げる(docs/46 §5)
    let (crop_factor, grow, blur, extend_down) = match f.region {
        FaceSwapRegion::Head => (1.6, 50, 51, FACE_SWAP_HEAD_EXTEND_DOWN),
        FaceSwapRegion::Face => (1.0, 20, 31, 0),
    };
    wf[id(0)] = node(
        "UltralyticsDetectorProvider",
        json!({ "model_name": f.detector }),
    );
    wf[id(1)] = node(
        "BboxDetectorSEGS",
        json!({
            "bbox_detector": link(&id(0), 0),
            "image": link(&target, 0),
            "threshold": 0.5,
            "dilation": 10,
            "crop_factor": crop_factor,
            "drop_size": 10,
            "labels": "all"
        }),
    );
    wf[id(2)] = node(
        "ImpactSEGSOrderedFilter",
        json!({
            "segs": link(&id(1), 0),
            "target": "area(=w*h)",
            "order": true,
            "take_start": 0,
            "take_count": 1
        }),
    );
    wf[id(3)] = node("SegsToCombinedMask", json!({ "segs": link(&id(2), 0) }));
    // 検出した顔の数(絞り込み後。0 または 1)を履歴へ出す
    wf[FACE_SWAP_COUNT_NODE] = node(
        "ImpactCount_Elts_in_SEGS",
        json!({ "segs": link(&id(2), 0) }),
    );
    wf[FACE_SWAP_COUNT_OUTPUT] = node(
        "PreviewAny",
        json!({ "source": link(FACE_SWAP_COUNT_NODE, 0) }),
    );
    // 下方向へ広げる: マスクを下へずらした複製との和集合にする(MaskComposite の or)
    let grow_input = if extend_down > 0 {
        wf[FACE_SWAP_EXTEND_NODE] = node(
            "MaskComposite",
            json!({
                "destination": link(&id(3), 0),
                "source": link(&id(3), 0),
                "x": 0,
                "y": extend_down,
                "operation": "or"
            }),
        );
        link(FACE_SWAP_EXTEND_NODE, 0)
    } else {
        link(&id(3), 0)
    };
    wf[id(4)] = node(
        "GrowMask",
        json!({ "mask": grow_input, "expand": grow, "tapered_corners": true }),
    );
    wf[id(5)] = node(
        "ImpactGaussianBlurMask",
        json!({ "mask": link(&id(4), 0), "kernel_size": blur, "sigma": f64::from(blur) / 2.0 }),
    );
    wf[id(6)] = node("GetImageSize", json!({ "image": link(&target, 0) }));
    wf[id(7)] = node(
        "ImageScale",
        json!({
            "image": link("8", 0),
            "upscale_method": "lanczos",
            "width": link(&id(6), 0),
            "height": link(&id(6), 1),
            "crop": "disabled"
        }),
    );
    wf[id(8)] = node(
        "ImageCompositeMasked",
        json!({
            "destination": link(&target, 0),
            "source": link(&id(7), 0),
            "x": 0,
            "y": 0,
            "resize_source": false,
            "mask": link(&id(5), 0)
        }),
    );
    wf["9"]["inputs"]["images"] = link(&id(8), 0);
}

/// 正・負の条件づけの作り方(docs/17 §4)。
#[derive(Clone, Copy)]
enum Conditioning {
    /// `CLIPTextEncode` が正・負の 2 つ(Anima)。
    Plain,
    /// `TextEncodeQwenImage21` が正・負を 1 ノードで出す(Qwen-Image 2.1)。
    QwenImage21,
    /// 正は `CLIPTextEncode`、負は `ConditioningZeroOut`(Krea 2)。
    ZeroOut,
}

/// Anima / Qwen-Image 2.1 / Krea 2。拡散モデル・テキストエンコーダ(1 つ)・VAE を別々に読む。
/// ノード構成と既定値は、ComfyUI 同梱の公式テンプレート(`image_anima_base_v1` / `image_qwen_image_2_1_t2i` /
/// `image_krea2_turbo_t2i`)に合わせている。`clip_type` は `CLIPLoader` の `type`。
fn build_split(p: &GenerationParams, prefix: &str, clip_type: &str, cond: Conditioning) -> Value {
    let clip = p.model.clip.first().cloned().unwrap_or_default();
    let vae = p.model.vae.clone().unwrap_or_default();

    let (positive, negative, extra) = match cond {
        Conditioning::Plain => (
            link("4", 0),
            link("5", 0),
            vec![
                (
                    "4",
                    node(
                        "CLIPTextEncode",
                        json!({ "text": p.prompt, "clip": link("2", 0) }),
                    ),
                ),
                (
                    "5",
                    node(
                        "CLIPTextEncode",
                        json!({ "text": p.negative_prompt, "clip": link("2", 0) }),
                    ),
                ),
            ],
        ),
        Conditioning::QwenImage21 => (
            link("4", 0),
            link("4", 1),
            vec![(
                "4",
                node(
                    "TextEncodeQwenImage21",
                    json!({
                        "clip": link("2", 0),
                        "prompt": p.prompt,
                        "negative_prompt": p.negative_prompt,
                        "resolution": 1024
                    }),
                ),
            )],
        ),
        Conditioning::ZeroOut => (
            link("4", 0),
            link("5", 0),
            vec![
                (
                    "4",
                    node(
                        "CLIPTextEncode",
                        json!({ "text": p.prompt, "clip": link("2", 0) }),
                    ),
                ),
                (
                    "5",
                    node(
                        "ConditioningZeroOut",
                        json!({ "conditioning": link("4", 0) }),
                    ),
                ),
            ],
        ),
    };

    let mut wf = json!({
        "1": node("UNETLoader", json!({ "unet_name": p.model.name, "weight_dtype": "default" })),
        "2": node("CLIPLoader", json!({ "clip_name": clip, "type": clip_type, "device": "default" })),
        "3": node("VAELoader", json!({ "vae_name": vae })),
        "6": node("EmptyLatentImage", json!({
            "width": p.width, "height": p.height, "batch_size": p.batch_size
        })),
        "7": node("KSampler", json!({
            "model": link("1", 0),
            "seed": p.seed,
            "steps": p.steps,
            "cfg": p.cfg,
            "sampler_name": p.sampler,
            "scheduler": p.scheduler,
            "positive": positive,
            "negative": negative,
            "latent_image": link("6", 0),
            "denoise": 1.0
        })),
        "8": node("VAEDecode", json!({ "samples": link("7", 0), "vae": link("3", 0) })),
        "9": node("SaveImage", json!({ "images": link("8", 0), "filename_prefix": prefix })),
    });
    for (id, n) in extra {
        wf[id] = n;
    }
    wf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::{ControlnetParams, sample};

    const ALL: [ModelKind; 7] = [
        ModelKind::Sd15,
        ModelKind::Sdxl,
        ModelKind::Flux,
        ModelKind::FluxGguf,
        ModelKind::Anima,
        ModelKind::QwenImage,
        ModelKind::Krea2,
    ];

    fn class_of<'a>(wf: &'a Value, id: &str) -> &'a str {
        wf[id]["class_type"].as_str().unwrap()
    }

    fn classes(wf: &Value) -> Vec<&str> {
        wf.as_object()
            .unwrap()
            .values()
            .map(|n| n["class_type"].as_str().unwrap())
            .collect()
    }

    /// すべてのリンクが存在するノードを指し、出力 SaveImage が 1 つあること。
    fn assert_well_formed(wf: &Value) {
        let nodes = wf.as_object().unwrap();
        for (id, n) in nodes {
            assert!(n["class_type"].is_string(), "node {id} has no class_type");
            for (name, input) in n["inputs"].as_object().unwrap() {
                if let Some(arr) = input.as_array() {
                    let target = arr[0]
                        .as_str()
                        .unwrap_or_else(|| panic!("{id}.{name}: bad link"));
                    assert!(
                        nodes.contains_key(target),
                        "{id}.{name} -> missing node {target}"
                    );
                    assert!(arr[1].is_u64(), "{id}.{name}: output index");
                    assert_ne!(target, id, "{id}.{name}: self link");
                }
            }
        }
        assert_eq!(classes(wf).iter().filter(|c| **c == "SaveImage").count(), 1);
    }

    #[test]
    fn all_kinds_are_well_formed() {
        for kind in ALL {
            let wf = build(&sample(kind), "gazooo", &[]);
            assert_well_formed(&wf);
        }
    }

    #[test]
    fn sd_uses_checkpoint_and_both_prompts() {
        let wf = build(&sample(ModelKind::Sdxl), "gazooo", &[]);
        assert_eq!(class_of(&wf, "1"), "CheckpointLoaderSimple");
        assert_eq!(wf["1"]["inputs"]["ckpt_name"], "model.safetensors");
        assert_eq!(wf["2"]["inputs"]["text"], "a cat");
        assert_eq!(wf["3"]["inputs"]["text"], "blurry");
        assert_eq!(wf["5"]["inputs"]["seed"], 42);
        assert_eq!(wf["4"]["inputs"]["batch_size"], 1);
        // VAE 未指定ならチェックポイントの VAE(出力 2)を使う
        assert_eq!(wf["6"]["inputs"]["vae"], json!(["1", 2]));
        assert!(!classes(&wf).contains(&"VAELoader"));
    }

    #[test]
    fn sd_with_explicit_vae_uses_vae_loader() {
        let mut p = sample(ModelKind::Sd15);
        p.model.vae = Some("custom.safetensors".into());
        let wf = build(&p, "gazooo", &[]);
        assert_well_formed(&wf);
        assert_eq!(wf["6"]["inputs"]["vae"], json!(["7", 0]));
        assert_eq!(wf["7"]["inputs"]["vae_name"], "custom.safetensors");
    }

    fn face_params(kind: ModelKind, prompt: &str) -> GenerationParams {
        let mut p = sample(kind);
        p.face_detailer = Some(FaceDetailerParams {
            enabled: true,
            detector: "bbox/face_yolov8m.pt".into(),
            denoise: 0.35,
            prompt: prompt.into(),
            confidence: 0.6,
        });
        p
    }

    #[test]
    fn face_detailer_sits_between_decode_and_save() {
        for kind in [ModelKind::Sd15, ModelKind::Sdxl] {
            let wf = build(&face_params(kind, ""), "gazooo", &[]);
            assert_well_formed(&wf);
            assert_eq!(class_of(&wf, FACE_DETAILER_NODE), "FaceDetailer");
            assert_eq!(class_of(&wf, "20"), "UltralyticsDetectorProvider");
            assert_eq!(wf["20"]["inputs"]["model_name"], "bbox/face_yolov8m.pt");
            let fd = &wf[FACE_DETAILER_NODE]["inputs"];
            assert_eq!(fd["image"], json!(["6", 0]));
            assert_eq!(fd["bbox_detector"], json!(["20", 0]));
            assert_eq!(fd["denoise"], 0.35);
            assert_eq!(fd["bbox_threshold"], 0.6);
            // 本体のサンプリング設定を引き継ぐ
            assert_eq!(
                (&fd["seed"], &fd["steps"], &fd["sampler_name"]),
                (&json!(42), &json!(20), &json!("euler"))
            );
            assert_eq!(fd["negative"], json!(["3", 0]));
            // 保存されるのは補正後の画像
            assert_eq!(wf["8"]["inputs"]["images"], json!([FACE_DETAILER_NODE, 0]));
        }
    }

    #[test]
    fn face_detailer_prompt_override_adds_encoder() {
        let wf = build(
            &face_params(ModelKind::Sd15, "detailed eyes"),
            "gazooo",
            &[],
        );
        assert_well_formed(&wf);
        assert_eq!(wf["21"]["inputs"]["text"], "detailed eyes");
        assert_eq!(
            wf[FACE_DETAILER_NODE]["inputs"]["positive"],
            json!(["21", 0])
        );

        // 空(空白のみ)なら本体のプロンプトを使い、エンコーダは増やさない
        let wf = build(&face_params(ModelKind::Sd15, "  "), "gazooo", &[]);
        assert!(wf.get("21").is_none());
        assert_eq!(
            wf[FACE_DETAILER_NODE]["inputs"]["positive"],
            json!(["2", 0])
        );
    }

    #[test]
    fn face_detailer_uses_explicit_vae_and_sdxl_sizes() {
        let mut p = face_params(ModelKind::Sdxl, "");
        p.model.vae = Some("custom.safetensors".into());
        let wf = build(&p, "gazooo", &[]);
        assert_well_formed(&wf);
        assert_eq!(wf[FACE_DETAILER_NODE]["inputs"]["vae"], json!(["7", 0]));
        assert_eq!(wf[FACE_DETAILER_NODE]["inputs"]["guide_size"], 1024);
        let wf = build(&face_params(ModelKind::Sd15, ""), "gazooo", &[]);
        assert_eq!(wf[FACE_DETAILER_NODE]["inputs"]["guide_size"], 512);
        assert_eq!(wf[FACE_DETAILER_NODE]["inputs"]["vae"], json!(["1", 2]));
    }

    #[test]
    fn disabled_face_detailer_leaves_workflow_unchanged() {
        let plain = build(&sample(ModelKind::Sd15), "gazooo", &[]);
        let mut p = face_params(ModelKind::Sd15, "x");
        p.face_detailer.as_mut().unwrap().enabled = false;
        assert_eq!(build(&p, "gazooo", &[]), plain);
        assert!(!classes(&plain).contains(&"FaceDetailer"));
    }

    fn cn_params(kind: ModelKind, pre: Preprocessor) -> GenerationParams {
        let mut p = sample(kind);
        p.width = 768;
        p.height = 512;
        p.controlnet = Some(ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: "inputs/a.png".into(),
                model: "control_canny.safetensors".into(),
                preprocessor: pre,
                strength: 0.8,
                start_percent: 0.1,
                end_percent: 0.9,
            }],
        });
        p
    }

    const IMG: [&str; 1] = ["gazooo-1.png"];

    #[test]
    fn controlnet_conditions_the_sampler_with_scaled_preprocessed_image() {
        for kind in [ModelKind::Sd15, ModelKind::Sdxl] {
            let wf = build(
                &cn_params(kind, Preprocessor::Canny),
                "gazooo",
                &IMG.map(String::from),
            );
            assert_well_formed(&wf);
            assert_eq!(class_of(&wf, "30"), "LoadImage");
            assert_eq!(wf["30"]["inputs"]["image"], "gazooo-1.png");
            // 生成サイズへ中央クロップ
            assert_eq!(class_of(&wf, "31"), "ImageScale");
            assert_eq!(wf["31"]["inputs"]["width"], 768);
            assert_eq!(wf["31"]["inputs"]["height"], 512);
            assert_eq!(wf["31"]["inputs"]["crop"], "center");
            assert_eq!(class_of(&wf, "32"), "Canny");
            assert_eq!(wf["32"]["inputs"]["image"], json!(["31", 0]));
            assert_eq!(class_of(&wf, "33"), "ControlNetLoader");
            assert_eq!(
                wf["33"]["inputs"]["control_net_name"],
                "control_canny.safetensors"
            );
            let apply = &wf["34"]["inputs"];
            assert_eq!(class_of(&wf, "34"), "ControlNetApplyAdvanced");
            assert_eq!(apply["image"], json!(["32", 0]));
            assert_eq!(apply["positive"], json!(["2", 0]));
            assert_eq!(apply["negative"], json!(["3", 0]));
            assert_eq!(
                (
                    &apply["strength"],
                    &apply["start_percent"],
                    &apply["end_percent"]
                ),
                (&json!(0.8), &json!(0.1), &json!(0.9))
            );
            // KSampler は適用後の conditioning を使う
            assert_eq!(wf["5"]["inputs"]["positive"], json!(["34", 0]));
            assert_eq!(wf["5"]["inputs"]["negative"], json!(["34", 1]));
        }
    }

    #[test]
    fn controlnet_preprocessor_choices() {
        let imgs = IMG.map(String::from);
        let depth = build(&cn_params(ModelKind::Sd15, Preprocessor::Depth), "g", &imgs);
        assert_well_formed(&depth);
        assert_eq!(class_of(&depth, "32"), "DepthAnythingV2Preprocessor");
        assert_eq!(depth["32"]["inputs"]["resolution"], 512, "短辺に合わせる");

        let pose = build(
            &cn_params(ModelKind::Sd15, Preprocessor::Openpose),
            "g",
            &imgs,
        );
        assert_well_formed(&pose);
        assert_eq!(class_of(&pose, "32"), "OpenposePreprocessor");

        // 前処理なし: リサイズ後の画像をそのまま条件にする(前処理ノードを作らない)
        let none = build(&cn_params(ModelKind::Sd15, Preprocessor::None), "g", &imgs);
        assert_well_formed(&none);
        assert!(none.get("32").is_none());
        assert_eq!(none["34"]["inputs"]["image"], json!(["31", 0]));
    }

    #[test]
    fn controlnet_and_face_detailer_coexist_and_face_keeps_plain_conditioning() {
        let mut p = cn_params(ModelKind::Sd15, Preprocessor::Canny);
        p.face_detailer = face_params(ModelKind::Sd15, "").face_detailer;
        let wf = build(&p, "gazooo", &IMG.map(String::from));
        assert_well_formed(&wf);
        assert_eq!(wf["5"]["inputs"]["positive"], json!(["34", 0]));
        // 顔補正は ControlNet 適用前の conditioning を使う(構図条件は顔の切り出しに合わない)
        let fd = &wf[FACE_DETAILER_NODE]["inputs"];
        assert_eq!(fd["positive"], json!(["2", 0]));
        assert_eq!(fd["negative"], json!(["3", 0]));
        assert_eq!(wf["8"]["inputs"]["images"], json!([FACE_DETAILER_NODE, 0]));
    }

    #[test]
    fn disabled_controlnet_leaves_workflow_unchanged() {
        let plain = build(&sample(ModelKind::Sd15), "gazooo", &[]);
        let mut p = cn_params(ModelKind::Sd15, Preprocessor::Canny);
        p.controlnet.as_mut().unwrap().enabled = false;
        p.width = 512;
        p.height = 512;
        assert_eq!(build(&p, "gazooo", &[]), plain);
        assert!(!classes(&plain).contains(&"ControlNetApplyAdvanced"));
    }

    fn inpaint_params(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.inpaint = Some(InpaintParams {
            enabled: true,
            image: "inputs/a.png".into(),
            mask: "inputs/m.png".into(),
            denoise: 0.65,
            grow_mask: 8,
            feather: 8,
            composite_back: true,
        });
        p
    }

    const INP: [&str; 2] = ["img.png", "mask.png"];

    #[test]
    fn inpaint_resamples_only_the_masked_latent_and_composites_back() {
        for kind in [ModelKind::Sd15, ModelKind::Sdxl] {
            let wf = build(&inpaint_params(kind), "gazooo", &INP.map(String::from));
            assert_well_formed(&wf);
            assert_eq!(wf["60"]["inputs"]["image"], "img.png");
            assert_eq!(wf["64"]["inputs"]["image"], "mask.png");
            assert_eq!(class_of(&wf, "62"), "VAEEncode");
            assert_eq!(wf["62"]["inputs"]["vae"], json!(["1", 2]));
            // KSampler は、ノイズマスク付きの潜在と inpaint の denoise を使う
            let ks = &wf["5"]["inputs"];
            assert_eq!(ks["latent_image"], json!(["72", 0]));
            assert_eq!(ks["denoise"], 0.65);
            assert_eq!(class_of(&wf, "72"), "SetLatentNoiseMask");
            assert_eq!(wf["72"]["inputs"]["samples"], json!(["62", 0]));
            // 拡張とぼかしを経たマスクを使う
            assert_eq!(wf["8"]["inputs"]["images"], json!(["74", 0]));
            let comp = &wf["74"]["inputs"];
            assert_eq!(class_of(&wf, "74"), "ImageCompositeMasked");
            assert_eq!(comp["destination"], json!(["61", 0]));
            assert_eq!(comp["source"], json!(["6", 0]));
            assert_eq!(comp["mask"], json!(["71", 0]));
            assert_eq!(wf["68"]["inputs"]["expand"], 8);
            assert_eq!(wf["70"]["inputs"]["blur_radius"], 8);
        }
    }

    #[test]
    fn inpaint_options_drop_unneeded_nodes() {
        let mut p = inpaint_params(ModelKind::Sd15);
        let ip = p.inpaint.as_mut().unwrap();
        ip.grow_mask = 0;
        ip.feather = 0;
        ip.composite_back = false;
        let wf = build(&p, "gazooo", &INP.map(String::from));
        assert_well_formed(&wf);
        for id in ["68", "69", "70", "71", "73", "74"] {
            assert!(wf.get(id).is_none(), "{id}");
        }
        assert_eq!(wf["72"]["inputs"]["mask"], json!(["67", 0]));
        // 合成しないので、保存されるのは VAEDecode の出力
        assert_eq!(wf["8"]["inputs"]["images"], json!(["6", 0]));
    }

    #[test]
    fn inpaint_repeats_latent_and_destination_for_batches() {
        let mut p = inpaint_params(ModelKind::Sd15);
        p.batch_size = 3;
        let wf = build(&p, "gazooo", &INP.map(String::from));
        assert_well_formed(&wf);
        assert_eq!(wf["63"]["inputs"]["amount"], 3);
        assert_eq!(wf["72"]["inputs"]["samples"], json!(["63", 0]));
        assert_eq!(wf["73"]["inputs"]["amount"], 3);
        assert_eq!(wf["74"]["inputs"]["destination"], json!(["73", 0]));
    }

    #[test]
    fn inpaint_images_follow_controlnet_images() {
        let mut p = cn_params(ModelKind::Sd15, Preprocessor::Canny);
        p.inpaint = inpaint_params(ModelKind::Sd15).inpaint;
        let imgs = ["cn.png", "img.png", "mask.png"].map(String::from);
        let wf = build(&p, "gazooo", &imgs);
        assert_well_formed(&wf);
        assert_eq!(wf["30"]["inputs"]["image"], "cn.png");
        assert_eq!(wf["60"]["inputs"]["image"], "img.png");
        assert_eq!(wf["64"]["inputs"]["image"], "mask.png");
        // ControlNet の条件付けと両立する
        assert_eq!(wf["5"]["inputs"]["positive"], json!(["34", 0]));
    }

    fn img2img_params(kind: ModelKind) -> GenerationParams {
        let mut p = sample(kind);
        p.img2img = Some(Img2imgParams {
            enabled: true,
            image: "inputs/a.png".into(),
            denoise: 0.55,
        });
        p
    }

    #[test]
    fn img2img_replaces_the_empty_latent_with_the_encoded_image() {
        for kind in [ModelKind::Sd15, ModelKind::Sdxl] {
            let mut p = img2img_params(kind);
            p.width = 768;
            p.height = 512;
            let wf = build(&p, "gazooo", &["src.png".to_string()]);
            assert_well_formed(&wf);
            assert_eq!(class_of(&wf, "80"), "LoadImage");
            assert_eq!(wf["80"]["inputs"]["image"], "src.png");
            assert_eq!(wf["81"]["inputs"]["width"], 768);
            assert_eq!(wf["81"]["inputs"]["height"], 512);
            assert_eq!(class_of(&wf, "82"), "VAEEncode");
            assert_eq!(wf["82"]["inputs"]["vae"], json!(["1", 2]));
            let ks = &wf["5"]["inputs"];
            assert_eq!(ks["latent_image"], json!(["82", 0]));
            assert_eq!(ks["denoise"], 0.55);
            assert!(wf.get("83").is_none());
        }
    }

    #[test]
    fn img2img_uses_explicit_vae_repeats_for_batches_and_follows_controlnet_images() {
        let mut p = cn_params(ModelKind::Sd15, Preprocessor::Canny);
        p.img2img = img2img_params(ModelKind::Sd15).img2img;
        p.model.vae = Some("v.safetensors".into());
        p.batch_size = 2;
        let wf = build(&p, "gazooo", &["cn.png".to_string(), "src.png".to_string()]);
        assert_well_formed(&wf);
        assert_eq!(wf["30"]["inputs"]["image"], "cn.png");
        assert_eq!(wf["80"]["inputs"]["image"], "src.png");
        assert_eq!(wf["82"]["inputs"]["vae"], json!(["7", 0]));
        assert_eq!(wf["83"]["inputs"]["amount"], 2);
        assert_eq!(wf["5"]["inputs"]["latent_image"], json!(["83", 0]));
        // ControlNet の条件付けと両立する
        assert_eq!(wf["5"]["inputs"]["positive"], json!(["34", 0]));
    }

    #[test]
    fn img2img_coexists_with_face_detailer_and_disabled_leaves_workflow_unchanged() {
        let mut p = img2img_params(ModelKind::Sd15);
        p.face_detailer = Some(FaceDetailerParams {
            enabled: true,
            detector: "bbox/face_yolov8m.pt".into(),
            denoise: 0.4,
            prompt: String::new(),
            confidence: 0.5,
        });
        let wf = build(&p, "gazooo", &["src.png".to_string()]);
        assert_well_formed(&wf);
        assert_eq!(class_of(&wf, "22"), "FaceDetailer");

        let plain = build(&sample(ModelKind::Sd15), "gazooo", &[]);
        let mut off = img2img_params(ModelKind::Sd15);
        off.img2img.as_mut().unwrap().enabled = false;
        assert_eq!(build(&off, "gazooo", &[]), plain);
    }

    #[test]
    fn disabled_inpaint_leaves_workflow_unchanged() {
        let plain = build(&sample(ModelKind::Sd15), "gazooo", &[]);
        let mut p = inpaint_params(ModelKind::Sd15);
        p.inpaint.as_mut().unwrap().enabled = false;
        assert_eq!(build(&p, "gazooo", &[]), plain);
    }

    fn split(kind: ModelKind) -> Value {
        let mut p = sample(kind);
        p.model.name = "unet.safetensors".into();
        p.model.clip = vec!["te.safetensors".into()];
        p.model.vae = Some("vae.safetensors".into());
        p.cfg = 4.0;
        p.steps = 30;
        build(&p, "gazooo", &[])
    }

    #[test]
    fn split_models_share_the_loader_skeleton() {
        // docs/17 §4: UNETLoader → CLIPLoader(type)→ VAELoader → EmptyLatentImage → KSampler → VAEDecode → SaveImage
        for (kind, clip_type) in [
            (ModelKind::Anima, "stable_diffusion"),
            (ModelKind::QwenImage, "qwen_image"),
            (ModelKind::Krea2, "krea2"),
        ] {
            let wf = split(kind);
            assert_well_formed(&wf);
            assert_eq!(class_of(&wf, "1"), "UNETLoader");
            assert_eq!(wf["1"]["inputs"]["unet_name"], "unet.safetensors");
            assert_eq!(wf["1"]["inputs"]["weight_dtype"], "default");
            assert_eq!(class_of(&wf, "2"), "CLIPLoader");
            assert_eq!(wf["2"]["inputs"]["clip_name"], "te.safetensors");
            assert_eq!(wf["2"]["inputs"]["type"], clip_type, "{kind:?}");
            assert_eq!(wf["2"]["inputs"]["device"], "default");
            assert_eq!(class_of(&wf, "3"), "VAELoader");
            assert_eq!(wf["3"]["inputs"]["vae_name"], "vae.safetensors");
            // SD3 用ではなく標準の EmptyLatentImage(公式テンプレートと同じ)
            assert_eq!(class_of(&wf, "6"), "EmptyLatentImage");
            assert_eq!(class_of(&wf, "7"), "KSampler");
            assert_eq!(
                wf["7"]["inputs"]["model"],
                json!(["1", 0]),
                "ModelSampling などを挟まない"
            );
            assert_eq!(wf["7"]["inputs"]["cfg"], 4.0);
            assert_eq!(wf["7"]["inputs"]["steps"], 30);
            assert_eq!(wf["8"]["inputs"]["vae"], json!(["3", 0]));
            // チェックポイントを読むノードや Flux 用のノードは使わない
            let used = classes(&wf);
            for banned in [
                "CheckpointLoaderSimple",
                "DualCLIPLoader",
                "FluxGuidance",
                "EmptySD3LatentImage",
            ] {
                assert!(!used.contains(&banned), "{kind:?}: {banned}");
            }
        }
    }

    #[test]
    fn anima_encodes_positive_and_negative_with_clip_text_encode() {
        let wf = split(ModelKind::Anima);
        assert_eq!(class_of(&wf, "4"), "CLIPTextEncode");
        assert_eq!(wf["4"]["inputs"]["text"], "a cat");
        assert_eq!(class_of(&wf, "5"), "CLIPTextEncode");
        assert_eq!(wf["5"]["inputs"]["text"], "blurry");
        assert_eq!(wf["7"]["inputs"]["positive"], json!(["4", 0]));
        assert_eq!(wf["7"]["inputs"]["negative"], json!(["5", 0]));
    }

    #[test]
    fn qwen_image_encodes_both_prompts_with_one_node() {
        let wf = split(ModelKind::QwenImage);
        assert_eq!(class_of(&wf, "4"), "TextEncodeQwenImage21");
        assert_eq!(wf["4"]["inputs"]["prompt"], "a cat");
        assert_eq!(wf["4"]["inputs"]["negative_prompt"], "blurry");
        assert_eq!(wf["4"]["inputs"]["clip"], json!(["2", 0]));
        assert_eq!(wf["7"]["inputs"]["positive"], json!(["4", 0]));
        assert_eq!(wf["7"]["inputs"]["negative"], json!(["4", 1]));
        assert!(wf.get("5").is_none());
    }

    fn reference_params(images: &[&str]) -> GenerationParams {
        let mut p = sample(ModelKind::QwenImage);
        p.model.name = "unet.safetensors".into();
        p.model.clip = vec!["te.safetensors".into()];
        p.model.vae = Some("vae.safetensors".into());
        p.reference = Some(crate::domain::params::ReferenceParams {
            enabled: true,
            images: images.iter().map(|s| (*s).to_string()).collect(),
            resolution: 768,
        });
        p
    }

    #[test]
    fn reference_connects_images_and_vae_and_samples_from_the_encoder_latent() {
        let p = reference_params(&["inputs/a.png", "inputs/b.png"]);
        let wf = build(&p, "gazooo", &["a.png".to_string(), "b.png".to_string()]);
        assert_well_formed(&wf);
        assert_eq!(class_of(&wf, "90"), "LoadImage");
        assert_eq!(wf["90"]["inputs"]["image"], "a.png");
        assert_eq!(wf["91"]["inputs"]["image"], "b.png");
        assert!(wf.get("92").is_none());
        let enc = &wf["4"]["inputs"];
        assert_eq!(enc["images.image_1"], json!(["90", 0]));
        assert_eq!(enc["images.image_2"], json!(["91", 0]));
        assert!(enc.get("images.image_3").is_none());
        assert_eq!(enc["vae"], json!(["3", 0]));
        assert_eq!(enc["resolution"], 768);
        // 空の潜在ではなく、エンコーダの latent 出力(3 つ目)を使う。denoise は 1
        assert!(wf.get("6").is_none());
        assert_eq!(wf["7"]["inputs"]["latent_image"], json!(["4", 2]));
        assert_eq!(wf["7"]["inputs"]["denoise"], 1.0);
        assert!(wf.get("100").is_none());
        // QwenImage21Cache は使わない(docs/24 Q-121)
        assert!(!classes(&wf).contains(&"QwenImage21Cache"));
        assert_eq!(wf["7"]["inputs"]["model"], json!(["1", 0]));
    }

    #[test]
    fn reference_repeats_the_latent_for_batches() {
        let mut p = reference_params(&["inputs/a.png"]);
        p.batch_size = 3;
        let wf = build(&p, "gazooo", &["a.png".to_string()]);
        assert_well_formed(&wf);
        assert_eq!(class_of(&wf, "100"), "RepeatLatentBatch");
        assert_eq!(wf["100"]["inputs"]["samples"], json!(["4", 2]));
        assert_eq!(wf["100"]["inputs"]["amount"], 3);
        assert_eq!(wf["7"]["inputs"]["latent_image"], json!(["100", 0]));
    }

    #[test]
    fn disabled_reference_leaves_the_text_to_image_workflow_unchanged() {
        let plain = build(&split_params(), "gazooo", &[]);
        let mut off = reference_params(&["inputs/a.png"]);
        off.reference.as_mut().unwrap().enabled = false;
        assert_eq!(build(&off, "gazooo", &[]), plain);
        assert!(plain["4"]["inputs"].get("vae").is_none());
    }

    fn face_swap_params(
        keep_outside: bool,
        region: crate::domain::params::FaceSwapRegion,
    ) -> GenerationParams {
        let mut p = split_params();
        p.face_swap = Some(crate::domain::params::FaceSwapParams {
            enabled: true,
            target: "inputs/t.png".into(),
            face: "inputs/f.png".into(),
            keep_outside,
            region,
            resolution: 1024,
            detector: "bbox/face_yolov9c.pt".into(),
        });
        p
    }

    fn face_swap_images() -> [String; 2] {
        ["t.png".to_string(), "f.png".to_string()]
    }

    #[test]
    fn face_swap_uses_two_references_and_applies_the_lora_between_unet_and_sampler() {
        use crate::domain::params::FaceSwapRegion;
        let p = face_swap_params(false, FaceSwapRegion::Head);
        let wf = build(&p, "gazooo", &face_swap_images());
        assert_well_formed(&wf);
        // 1 枚目 = 対象、2 枚目 = 顔(参照画像つき生成と同じ骨格)
        assert_eq!(wf["90"]["inputs"]["image"], "t.png");
        assert_eq!(wf["91"]["inputs"]["image"], "f.png");
        assert_eq!(wf["4"]["inputs"]["images.image_1"], json!(["90", 0]));
        assert_eq!(wf["4"]["inputs"]["images.image_2"], json!(["91", 0]));
        assert_eq!(wf["4"]["inputs"]["vae"], json!(["3", 0]));
        assert_eq!(wf["7"]["inputs"]["latent_image"], json!(["4", 2]));
        // LoRA は UNET と KSampler の間。強さ 1.0 固定
        assert_eq!(class_of(&wf, "110"), "LoraLoaderModelOnly");
        assert_eq!(wf["110"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(
            wf["110"]["inputs"]["lora_name"],
            crate::domain::params::FACE_SWAP_LORA
        );
        assert_eq!(wf["110"]["inputs"]["strength_model"], 1.0);
        assert_eq!(wf["7"]["inputs"]["model"], json!(["110", 0]));
        // 合成なし: 保存は VAEDecode の出力のまま。検出・合成のノードは作らない
        assert_eq!(wf["9"]["inputs"]["images"], json!(["8", 0]));
        let used = classes(&wf);
        for banned in ["UltralyticsDetectorProvider", "ImageCompositeMasked"] {
            assert!(!used.contains(&banned), "{banned}");
        }
    }

    #[test]
    fn face_swap_composites_the_head_back_onto_the_original() {
        use crate::domain::params::FaceSwapRegion;
        let p = face_swap_params(true, FaceSwapRegion::Head);
        let wf = build(&p, "gazooo", &face_swap_images());
        assert_well_formed(&wf);
        assert_eq!(class_of(&wf, "111"), "UltralyticsDetectorProvider");
        assert_eq!(wf["111"]["inputs"]["model_name"], "bbox/face_yolov9c.pt");
        // 検出は元の対象画像(LoadImage 90)で、最も大きい 1 つだけ
        assert_eq!(wf["112"]["inputs"]["image"], json!(["90", 0]));
        assert_eq!(wf["112"]["inputs"]["crop_factor"], 1.6);
        assert_eq!(class_of(&wf, "113"), "ImpactSEGSOrderedFilter");
        assert_eq!(wf["113"]["inputs"]["take_count"], 1);
        assert_eq!(wf["113"]["inputs"]["target"], "area(=w*h)");
        // マスクは拡大 → ぼかし(FeatherMask は輪郭に効かないので使わない)
        assert_eq!(wf["115"]["inputs"]["expand"], 50);
        assert_eq!(class_of(&wf, "116"), "ImpactGaussianBlurMask");
        assert_eq!(wf["116"]["inputs"]["kernel_size"], 51);
        assert!(!classes(&wf).contains(&"FeatherMask"));
        // 生成画像を元のサイズへ戻し、元の対象画像に貼り戻す
        assert_eq!(class_of(&wf, "117"), "GetImageSize");
        assert_eq!(wf["117"]["inputs"]["image"], json!(["90", 0]));
        assert_eq!(wf["118"]["inputs"]["image"], json!(["8", 0]));
        assert_eq!(wf["118"]["inputs"]["width"], json!(["117", 0]));
        assert_eq!(wf["118"]["inputs"]["height"], json!(["117", 1]));
        assert_eq!(wf["119"]["inputs"]["destination"], json!(["90", 0]));
        assert_eq!(wf["119"]["inputs"]["source"], json!(["118", 0]));
        assert_eq!(wf["119"]["inputs"]["mask"], json!(["116", 0]));
        assert_eq!(wf["9"]["inputs"]["images"], json!(["119", 0]));
    }

    #[test]
    fn face_swap_face_region_uses_a_smaller_mask_than_head() {
        use crate::domain::params::FaceSwapRegion;
        let face = build(
            &face_swap_params(true, FaceSwapRegion::Face),
            "gazooo",
            &face_swap_images(),
        );
        assert_eq!(face["112"]["inputs"]["crop_factor"], 1.0);
        assert_eq!(face["115"]["inputs"]["expand"], 20);
        assert_eq!(face["116"]["inputs"]["kernel_size"], 31);
        // 顔は下方向へ広げない(MaskComposite を作らず、GrowMask は検出マスクを直接受ける)
        assert!(!classes(&face).contains(&"MaskComposite"));
        assert_eq!(face["115"]["inputs"]["mask"], json!(["114", 0]));
    }

    #[test]
    fn face_swap_composite_reports_the_detected_face_count() {
        use crate::domain::params::FaceSwapRegion;
        let wf = build(
            &face_swap_params(true, FaceSwapRegion::Head),
            "gazooo",
            &face_swap_images(),
        );
        assert_well_formed(&wf);
        // 絞り込み後の SEGS(113)の数を、出力ノード(PreviewAny)で履歴へ出す
        assert_eq!(class_of(&wf, "121"), "ImpactCount_Elts_in_SEGS");
        assert_eq!(wf["121"]["inputs"]["segs"], json!(["113", 0]));
        assert_eq!(class_of(&wf, "122"), "PreviewAny");
        assert_eq!(wf["122"]["inputs"]["source"], json!(["121", 0]));
        // 合成なしのときは作らない(検出自体が無い)
        let off = build(
            &face_swap_params(false, FaceSwapRegion::Head),
            "gazooo",
            &face_swap_images(),
        );
        assert!(!classes(&off).contains(&"PreviewAny"));
    }

    #[test]
    fn face_probe_workflow_counts_faces_without_any_model() {
        let wf = build_face_probe("t.png", "bbox/face_yolov9c.pt");
        // 画像は保存しない(SaveImage なし。出力ノードは PreviewAny のみ)
        assert!(!classes(&wf).contains(&"SaveImage"));
        assert_eq!(wf["1"]["inputs"]["image"], "t.png");
        assert_eq!(wf["2"]["inputs"]["model_name"], "bbox/face_yolov9c.pt");
        assert_eq!(wf["3"]["inputs"]["image"], json!(["1", 0]));
        assert_eq!(class_of(&wf, "5"), "ImpactCount_Elts_in_SEGS");
        assert_eq!(wf["5"]["inputs"]["segs"], json!(["3", 0]));
        assert_eq!(class_of(&wf, FACE_PROBE_OUTPUT), "PreviewAny");
        assert_eq!(wf[FACE_PROBE_OUTPUT]["inputs"]["source"], json!(["5", 0]));
        // 生成のモデルやサンプラーは使わない
        let used = classes(&wf);
        for banned in ["KSampler", "UNETLoader", "LoraLoaderModelOnly"] {
            assert!(!used.contains(&banned), "{banned}");
        }
    }

    #[test]
    fn face_swap_head_mask_is_extended_downward_before_growing() {
        use crate::domain::params::FaceSwapRegion;
        let wf = build(
            &face_swap_params(true, FaceSwapRegion::Head),
            "gazooo",
            &face_swap_images(),
        );
        assert_well_formed(&wf);
        // 検出マスク(114)とそれを下へずらした複製の和集合 → GrowMask(115)
        assert_eq!(class_of(&wf, "120"), "MaskComposite");
        assert_eq!(wf["120"]["inputs"]["destination"], json!(["114", 0]));
        assert_eq!(wf["120"]["inputs"]["source"], json!(["114", 0]));
        assert_eq!(wf["120"]["inputs"]["x"], 0);
        assert_eq!(wf["120"]["inputs"]["y"], 60);
        assert_eq!(wf["120"]["inputs"]["operation"], "or");
        assert_eq!(wf["115"]["inputs"]["mask"], json!(["120", 0]));
    }

    #[test]
    fn face_swap_node_ids_do_not_collide_with_other_features() {
        use crate::domain::params::FaceSwapRegion;
        let wf = build(
            &face_swap_params(true, FaceSwapRegion::Head),
            "gazooo",
            &face_swap_images(),
        );
        // 90・91(LoadImage)、100(参照のバッチ用。使わない)と、110〜120
        let ids: Vec<u32> = wf
            .as_object()
            .unwrap()
            .keys()
            .filter_map(|k| k.parse().ok())
            .filter(|n| *n >= 20)
            .collect();
        assert!(
            ids.iter().all(|n| matches!(n, 90 | 91 | 110..=122)),
            "{ids:?}"
        );
    }

    #[test]
    fn disabled_face_swap_leaves_the_text_to_image_workflow_unchanged() {
        use crate::domain::params::FaceSwapRegion;
        let plain = build(&split_params(), "gazooo", &[]);
        let mut off = face_swap_params(true, FaceSwapRegion::Head);
        off.face_swap.as_mut().unwrap().enabled = false;
        assert_eq!(build(&off, "gazooo", &[]), plain);
    }

    fn split_params() -> GenerationParams {
        let mut p = sample(ModelKind::QwenImage);
        p.model.name = "unet.safetensors".into();
        p.model.clip = vec!["te.safetensors".into()];
        p.model.vae = Some("vae.safetensors".into());
        p
    }

    #[test]
    fn krea2_zeroes_out_the_negative() {
        let wf = split(ModelKind::Krea2);
        assert_eq!(class_of(&wf, "4"), "CLIPTextEncode");
        assert_eq!(class_of(&wf, "5"), "ConditioningZeroOut");
        assert_eq!(wf["5"]["inputs"]["conditioning"], json!(["4", 0]));
        assert_eq!(wf["7"]["inputs"]["negative"], json!(["5", 0]));
        // ネガティブプロンプトの文字列はワークフローに入らない
        assert!(!wf.to_string().contains("blurry"));
    }

    #[test]
    fn split_models_apply_size_batch_seed_and_prefix() {
        let mut p = sample(ModelKind::Krea2);
        p.width = 832;
        p.height = 1216;
        p.batch_size = 3;
        p.seed = 99;
        let wf = build(&p, "out", &[]);
        assert_eq!(wf["6"]["inputs"]["width"], 832);
        assert_eq!(wf["6"]["inputs"]["height"], 1216);
        assert_eq!(wf["6"]["inputs"]["batch_size"], 3);
        assert_eq!(wf["7"]["inputs"]["seed"], 99);
        assert_eq!(wf["9"]["inputs"]["filename_prefix"], "out");
    }

    #[test]
    fn flux_uses_unet_dual_clip_and_guidance() {
        let wf = build(&sample(ModelKind::Flux), "gazooo", &[]);
        assert_eq!(class_of(&wf, "1"), "UNETLoader");
        assert_eq!(class_of(&wf, "2"), "DualCLIPLoader");
        assert_eq!(wf["2"]["inputs"]["clip_name1"], "t5xxl.safetensors");
        assert_eq!(wf["2"]["inputs"]["clip_name2"], "clip_l.safetensors");
        assert_eq!(wf["2"]["inputs"]["type"], "flux");
        assert_eq!(wf["5"]["inputs"]["guidance"], 3.5);
        // Flux は cfg 1.0 固定でネガティブプロンプトは使わない
        assert_eq!(wf["8"]["inputs"]["cfg"], 1.0);
        assert!(!wf.to_string().contains("blurry"));
        assert!(classes(&wf).contains(&"EmptySD3LatentImage"));
    }

    #[test]
    fn flux_gguf_uses_gguf_loader() {
        let wf = build(&sample(ModelKind::FluxGguf), "gazooo", &[]);
        assert_eq!(class_of(&wf, "1"), "UnetLoaderGGUF");
        assert_eq!(class_of(&wf, "2"), "DualCLIPLoader");
        assert!(wf["1"]["inputs"].get("weight_dtype").is_none());
    }

    #[test]
    fn gguf_text_encoder_switches_clip_loader() {
        let mut p = sample(ModelKind::FluxGguf);
        p.model.clip[0] = "t5-v1_1-xxl-Q4_K_M.gguf".into();
        let wf = build(&p, "gazooo", &[]);
        assert_eq!(class_of(&wf, "2"), "DualCLIPLoaderGGUF");
        assert_well_formed(&wf);
    }

    #[test]
    fn filename_prefix_and_size_are_applied() {
        let mut p = sample(ModelKind::Flux);
        p.width = 1024;
        p.height = 768;
        p.batch_size = 4;
        let wf = build(&p, "prefix", &[]);
        assert_eq!(wf["10"]["inputs"]["filename_prefix"], "prefix");
        assert_eq!(wf["7"]["inputs"]["width"], 1024);
        assert_eq!(wf["7"]["inputs"]["height"], 768);
        assert_eq!(wf["7"]["inputs"]["batch_size"], 4);
    }
}
