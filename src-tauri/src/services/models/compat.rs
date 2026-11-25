//! モデルファイルと生成種別の整合(docs/20 §2.4)。
//!
//! カタログでファイルの用途が分かるものだけを対象にする。カタログにないファイル(手動配置)は
//! 判断できないため、検証しない。フロントエンドの `lib/generation/compat.ts` と同じ規則。

use super::catalog::{BaseModel, Catalog, Category, FamilyKind};
use crate::domain::params::{GenerationParams, ModelKind};
use crate::error::{AppError, AppResult};

/// パスの区切りを問わず、ファイル名だけを小文字で取り出す。
fn key(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase()
}

/// カタログで分かるファイルが使える種別。カタログにないファイルは `None`。
/// 同じファイルが複数のファミリーにあれば、種別の和集合。
pub fn kinds_of_file(catalog: &Catalog, file_name: &str) -> Option<Vec<ModelKind>> {
    let want = key(file_name);
    let mut kinds: Vec<ModelKind> = Vec::new();
    let mut found = false;
    let mut add = |k: ModelKind| {
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    };
    for f in &catalog.families {
        for v in f.variants.iter().filter(|v| key(&v.file_name) == want) {
            found = true;
            match (f.category, f.kind) {
                (Category::Checkpoint, _) => match v.base_model {
                    Some(BaseModel::Sd15) => add(ModelKind::Sd15),
                    Some(BaseModel::Sdxl) => add(ModelKind::Sdxl),
                    None => {}
                },
                (_, Some(FamilyKind::FluxGguf)) => add(ModelKind::FluxGguf),
                (_, Some(FamilyKind::Anima)) => add(ModelKind::Anima),
                (_, Some(FamilyKind::QwenImage)) => add(ModelKind::QwenImage),
                (_, Some(FamilyKind::Krea2)) => add(ModelKind::Krea2),
                // Flux のテキストエンコーダ・VAE は、Flux と Flux GGUF の両方で使う
                (Category::Flux, None) => {
                    add(ModelKind::Flux);
                    add(ModelKind::FluxGguf);
                }
                (Category::Anima, None) => add(ModelKind::Anima),
                (Category::QwenImage, None) => add(ModelKind::QwenImage),
                (Category::Krea2, None) => add(ModelKind::Krea2),
                // ControlNet・検出モデルは生成の種別に関係しない
                (Category::Controlnet | Category::Detector, None) => found = false,
            }
        }
    }
    found.then_some(kinds)
}

fn kind_label(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Sd15 => "SD 1.5",
        ModelKind::Sdxl => "SDXL",
        ModelKind::Flux => "Flux",
        ModelKind::FluxGguf => "Flux GGUF",
        ModelKind::Anima => "Anima",
        ModelKind::QwenImage => "Qwen-Image 2.1",
        ModelKind::Krea2 => "Krea 2",
    }
}

/// 本体・テキストエンコーダ・VAE が、選択した種別と合うか確認する。合わなければ `VALIDATION`。
pub fn check_params(catalog: &Catalog, p: &GenerationParams) -> AppResult<()> {
    let kind = p.model.kind;
    let check = |field: &str, name: &str| -> AppResult<()> {
        let Some(kinds) = kinds_of_file(catalog, name) else {
            return Ok(());
        };
        if kinds.contains(&kind) {
            return Ok(());
        }
        let owners = kinds
            .iter()
            .map(|k| kind_label(*k))
            .collect::<Vec<_>>()
            .join(" / ");
        Err(AppError::validation(
            field,
            format!(
                "{name} は {owners} 用です({} では使えません)",
                kind_label(kind)
            ),
        ))
    };
    check("model", &p.model.name)?;
    for c in p.model.clip.iter().filter(|c| !c.is_empty()) {
        check("clip", c)?;
    }
    if let Some(v) = p.model.vae.as_deref().filter(|v| !v.is_empty()) {
        check("vae", v)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::sample;
    use crate::services::assistant::capabilities::catalog;

    fn cat() -> &'static Catalog {
        catalog().expect("embedded catalog")
    }

    fn params(kind: ModelKind, name: &str) -> GenerationParams {
        let mut p = sample(kind);
        p.model.name = name.into();
        p
    }

    #[test]
    fn checkpoint_kinds_follow_the_base_model() {
        let k = kinds_of_file(cat(), "sd_xl_base_1.0.safetensors").unwrap();
        assert_eq!(k, [ModelKind::Sdxl]);
        let k = kinds_of_file(cat(), "v1-5-pruned-emaonly.safetensors").unwrap();
        assert_eq!(k, [ModelKind::Sd15]);
        // パスの区切り・大文字小文字を問わない
        assert!(kinds_of_file(cat(), "sub\\SD_XL_BASE_1.0.safetensors").is_some());
        assert!(kinds_of_file(cat(), "my-own-merge.safetensors").is_none());
    }

    #[test]
    fn shared_vae_is_the_union_and_split_bodies_have_one_kind() {
        let mut k = kinds_of_file(cat(), "qwen_image_vae.safetensors").unwrap();
        k.sort_by_key(|k| format!("{k:?}"));
        assert_eq!(k, [ModelKind::Anima, ModelKind::Krea2]);
        assert_eq!(
            kinds_of_file(cat(), "krea2_turbo_fp8_scaled.safetensors").unwrap(),
            [ModelKind::Krea2]
        );
        let ae = kinds_of_file(cat(), "ae.safetensors").unwrap();
        assert!(ae.contains(&ModelKind::Flux) && ae.contains(&ModelKind::FluxGguf));
    }

    #[test]
    fn controlnet_and_detector_files_are_not_judged() {
        assert!(kinds_of_file(cat(), "controlnet-canny-sdxl-1.0_fp16.safetensors").is_none());
    }

    #[test]
    fn rejects_known_files_for_the_wrong_kind_and_passes_unknown_ones() {
        let ok = params(ModelKind::Sdxl, "sd_xl_base_1.0.safetensors");
        assert!(check_params(cat(), &ok).is_ok());

        let bad = params(ModelKind::Sd15, "sd_xl_base_1.0.safetensors");
        let e = check_params(cat(), &bad).unwrap_err();
        assert_eq!(e.code(), "VALIDATION");

        // 手動配置(カタログ外)は通す
        assert!(check_params(cat(), &params(ModelKind::Sd15, "my-own-merge.safetensors")).is_ok());

        // エンコーダ・VAE も確認する(Krea 2 の本体に Qwen-Image 2.1 のエンコーダ)
        let mut p = params(ModelKind::Krea2, "krea2_turbo_fp8_scaled.safetensors");
        p.model.clip = vec!["qwen3vl_4b_fp8_scaled.safetensors".into()];
        p.model.vae = Some("qwen_image_vae.safetensors".into());
        assert!(check_params(cat(), &p).is_ok());
        p.model.clip = vec!["qwen3vl_8b_int8_convrot.safetensors".into()];
        let e = check_params(cat(), &p).unwrap_err();
        assert!(e.to_string().contains("clip") || e.to_string().contains("Qwen"));
        p.model.clip = vec!["qwen3vl_4b_fp8_scaled.safetensors".into()];
        p.model.vae = Some("ae.safetensors".into());
        assert_eq!(check_params(cat(), &p).unwrap_err().code(), "VALIDATION");
    }
}
