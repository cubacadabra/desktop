use crate::options::DesktopError;
use image::{GenericImage, RgbaImage, imageops::FilterType};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};

const MAX_ATLAS_DIMENSION: u32 = 2048;
const MAX_IMAGE_DIMENSION: u32 = 1020;
const PADDING: u32 = 2;

#[derive(Debug)]
pub struct ImageAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub regions: BTreeMap<String, [f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct ModelAsset {
    pub id: String,
    pub bytes: Vec<u8>,
}

pub fn load(root: &Path, manifest_source: &str) -> Result<Option<ImageAtlas>, Box<dyn Error>> {
    load_images(manifest_source, |relative| {
        let path = safe_asset_path(root, relative)?;
        Ok(fs::read(path)?)
    })
}

pub fn load_bundled(
    files: &[(&str, &[u8])],
    manifest_source: &str,
) -> Result<Option<ImageAtlas>, Box<dyn Error>> {
    load_images(manifest_source, |relative| {
        files
            .iter()
            .find_map(|(path, bytes)| (*path == relative).then_some(*bytes))
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| {
                Box::new(DesktopError(format!(
                    "bundled asset file does not exist: {relative}"
                ))) as Box<dyn Error>
            })
    })
}

pub fn load_from_files(
    files: &[(String, Vec<u8>)],
    manifest_source: &str,
) -> Result<Option<ImageAtlas>, Box<dyn Error>> {
    load_images(manifest_source, |relative| {
        files
            .iter()
            .find_map(|(path, bytes)| (path == relative).then_some(bytes.clone()))
            .ok_or_else(|| {
                Box::new(DesktopError(format!(
                    "remote package asset file does not exist: {relative}"
                ))) as Box<dyn Error>
            })
    })
}

pub fn load_models(root: &Path, manifest_source: &str) -> Result<Vec<ModelAsset>, Box<dyn Error>> {
    load_models_with(manifest_source, |relative| {
        let path = safe_asset_path(root, relative)?;
        Ok(fs::read(path)?)
    })
}

pub fn load_bundled_models(
    files: &[(&str, &[u8])],
    manifest_source: &str,
) -> Result<Vec<ModelAsset>, Box<dyn Error>> {
    load_models_with(manifest_source, |relative| {
        files
            .iter()
            .find_map(|(path, bytes)| (*path == relative).then_some(*bytes))
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| {
                Box::new(DesktopError(format!(
                    "bundled asset file does not exist: {relative}"
                ))) as Box<dyn Error>
            })
    })
}

pub fn load_models_from_files(
    files: &[(String, Vec<u8>)],
    manifest_source: &str,
) -> Result<Vec<ModelAsset>, Box<dyn Error>> {
    load_models_with(manifest_source, |relative| {
        files
            .iter()
            .find_map(|(path, bytes)| (path == relative).then_some(bytes.clone()))
            .ok_or_else(|| {
                Box::new(DesktopError(format!(
                    "remote package asset file does not exist: {relative}"
                ))) as Box<dyn Error>
            })
    })
}

fn load_models_with(
    manifest_source: &str,
    mut read_asset: impl FnMut(&str) -> Result<Vec<u8>, Box<dyn Error>>,
) -> Result<Vec<ModelAsset>, Box<dyn Error>> {
    let manifest: Value = serde_json::from_str(manifest_source)?;
    let Some(models) = manifest
        .get("assets")
        .and_then(|assets| assets.get("models"))
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    if models.len() > 64 {
        return Err(Box::new(DesktopError(
            "a game package may declare at most 64 world models".into(),
        )));
    }
    models
        .iter()
        .map(|(id, definition)| {
            let path = definition
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| DesktopError(format!("model asset {id:?} has no path")))?;
            if !path.to_ascii_lowercase().ends_with(".glb") {
                return Err(Box::new(DesktopError(format!(
                    "model asset {id:?} must be an embedded .glb file"
                ))) as Box<dyn Error>);
            }
            Ok(ModelAsset {
                id: id.clone(),
                bytes: read_asset(path)?,
            })
        })
        .collect()
}

fn load_images(
    manifest_source: &str,
    mut read_asset: impl FnMut(&str) -> Result<Vec<u8>, Box<dyn Error>>,
) -> Result<Option<ImageAtlas>, Box<dyn Error>> {
    let manifest: Value = serde_json::from_str(manifest_source)?;
    let Some(images) = manifest
        .get("assets")
        .and_then(|assets| assets.get("images"))
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }

    let mut loaded = Vec::with_capacity(images.len());
    for (id, definition) in images {
        let relative = definition
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| DesktopError(format!("image asset {id:?} has no path")))?;
        let bytes = read_asset(relative)?;
        let mut image = image::load_from_memory(&bytes)?.to_rgba8();
        let longest = image.width().max(image.height());
        if longest > MAX_IMAGE_DIMENSION {
            let scale = MAX_IMAGE_DIMENSION as f32 / longest as f32;
            image = image::imageops::resize(
                &image,
                (image.width() as f32 * scale).round().max(1.0) as u32,
                (image.height() as f32 * scale).round().max(1.0) as u32,
                FilterType::Lanczos3,
            );
        }
        loaded.push((id.clone(), image));
    }

    let mut placements = Vec::with_capacity(loaded.len());
    let mut x = PADDING;
    let mut y = PADDING;
    let mut row_height = 0;
    for (id, image) in &loaded {
        if image.width() + PADDING * 2 > MAX_ATLAS_DIMENSION
            || image.height() + PADDING * 2 > MAX_ATLAS_DIMENSION
        {
            return Err(Box::new(DesktopError(format!(
                "image asset {id:?} is too large for the world atlas"
            ))));
        }
        if x + image.width() + PADDING > MAX_ATLAS_DIMENSION {
            x = PADDING;
            y += row_height + PADDING;
            row_height = 0;
        }
        if y + image.height() + PADDING > MAX_ATLAS_DIMENSION {
            return Err(Box::new(DesktopError(
                "the game's images do not fit in a 2048px world atlas".into(),
            )));
        }
        placements.push((x, y));
        x += image.width() + PADDING;
        row_height = row_height.max(image.height());
    }

    let height = (y + row_height + PADDING)
        .next_power_of_two()
        .min(MAX_ATLAS_DIMENSION);
    let mut atlas = RgbaImage::new(MAX_ATLAS_DIMENSION, height.max(1));
    let mut regions = BTreeMap::new();
    for ((id, image), (left, top)) in loaded.iter().zip(placements) {
        atlas.copy_from(image, left, top)?;
        regions.insert(
            id.clone(),
            [
                (left as f32 + 0.5) / atlas.width() as f32,
                (top as f32 + 0.5) / atlas.height() as f32,
                image.width().saturating_sub(1).max(1) as f32 / atlas.width() as f32,
                image.height().saturating_sub(1).max(1) as f32 / atlas.height() as f32,
            ],
        );
    }
    Ok(Some(ImageAtlas {
        width: atlas.width(),
        height: atlas.height(),
        pixels: atlas.into_raw(),
        regions,
    }))
}

fn safe_asset_path(root: &Path, relative: &str) -> Result<PathBuf, Box<dyn Error>> {
    let relative = Path::new(relative);
    let safe = relative.starts_with("assets")
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !safe {
        return Err(Box::new(DesktopError(format!(
            "asset path is outside assets/: {relative:?}"
        ))));
    }
    let path = root.join(relative);
    if !path.is_file() {
        return Err(Box::new(DesktopError(format!(
            "asset file does not exist: {}",
            path.display()
        ))));
    }
    Ok(path)
}
