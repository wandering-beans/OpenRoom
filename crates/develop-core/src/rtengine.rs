//! Single ownership of native settings; the remaining pipeline never applies them twice.
use crate::{BaseInfo, Settings, WorkingImage, resize_linear};
use advanced_explorer_media::develop_input::RawColor;
use advanced_explorer_rtengine::{Adjustments, Rendered, Session};
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

struct CachedSession {
    key: (PathBuf, u64, std::time::SystemTime, u32, usize),
    session: Session,
    source_edge: u32,
}
static SESSION: Mutex<Option<CachedSession>> = Mutex::new(None);
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

struct CameraSaturation {
    key: (PathBuf, u64, std::time::SystemTime),
    gain: f32,
}
static CAMERA_SATURATION: Mutex<Option<CameraSaturation>> = Mutex::new(None);

fn camera_saturation(source: &Path) -> Result<f32> {
    let metadata = std::fs::metadata(source)?;
    ensure!(metadata.len() <= 1024 * 1024 * 1024, "Photo exceeds 1 GiB");
    let key = (source.to_path_buf(), metadata.len(), metadata.modified()?);
    let mut cache = CAMERA_SATURATION
        .lock()
        .map_err(|_| anyhow::anyhow!("Camera color cache failed"))?;
    if let Some(cached) = cache.as_ref().filter(|cached| cached.key == key) {
        return Ok(cached.gain);
    }
    let gain = match advanced_explorer_media::embedded_raw_preview(source) {
        Ok(camera) => {
            // Fix the calibration size and settings so editor, library and full-size exports agree.
            native(source, None, 384, &Adjustments::default())
                .ok()
                .filter(|reference| reference.camera_matched)
                .map_or(1., |reference| {
                    crate::camera_saturation::estimate(&reference.pixels, &camera.to_rgb8())
                })
        }
        Err(_) => 1.,
    };
    *cache = Some(CameraSaturation { key, gain });
    Ok(gain)
}

pub use advanced_explorer_media::is_raw;

fn native(
    source: &Path,
    pixels: Option<&image::Rgba32FImage>,
    bound: u32,
    adjustments: &Adjustments,
) -> Result<Rendered> {
    let mut cached = SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("rtengine session failed"))?;
    INITIALIZED
        .get_or_init(|| advanced_explorer_rtengine::init().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let metadata = std::fs::metadata(source)?;
    ensure!(metadata.len() <= 1024 * 1024 * 1024, "Photo exceeds 1 GiB");
    let mut key = (
        source.to_path_buf(),
        metadata.len(),
        metadata.modified()?,
        bound,
        pixels.map_or(0, |p| p.as_raw().as_ptr() as usize),
    );
    if let Some(hit) = cached.as_ref()
        && key.0 == hit.key.0
        && key.1 == hit.key.1
        && key.2 == hit.key.2
        && bound >= hit.source_edge
    {
        key.3 = 0;
    }
    if cached.as_ref().is_none_or(|hit| hit.key != key) {
        *cached = None;
        let session = Session::open(source, pixels, bound)?;
        *cached = Some(CachedSession {
            key,
            session,
            source_edge: u32::MAX,
        });
    }
    let cached = cached.as_mut().unwrap();
    let rendered = cached.session.render(adjustments)?;
    cached.source_edge = rendered.source_width.max(rendered.source_height);
    if cached.key.3 >= cached.source_edge {
        cached.key.3 = 0;
    }
    Ok(rendered)
}

pub fn prepare_raw(source: &Path, bound: u32) -> Result<WorkingImage> {
    prepare_raw_adjusted(source, bound, &Adjustments::default()).map(|(working, _)| working)
}

pub fn prepare_export(source: &Path, settings: &Settings) -> Result<WorkingImage> {
    let (mut working, camera_matched) =
        prepare_raw_adjusted(source, 0, &adjustments(settings, true, 1.))?;
    // An export never reuses native stages; release them before Rust's full-size filters.
    *SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("rtengine session failed"))? = None;
    working.info.native_source = None;
    working.info.raw = None;
    working.info.source_kind = if camera_matched {
        "rtengine-raw-camera"
    } else {
        "rtengine-raw-linear"
    }
    .into();
    Ok(working)
}

fn prepare_raw_adjusted(
    source: &Path,
    bound: u32,
    adjustments: &Adjustments,
) -> Result<(WorkingImage, bool)> {
    let source = source.canonicalize()?;
    let camera_saturation = camera_saturation(&source)?;
    let rendered = native(&source, None, bound, adjustments)?;
    let mut pixels = rendered.pixels;
    if bound > 0 {
        pixels = resize_linear(pixels, bound)?;
    }
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let camera_matched = rendered.camera_matched;
    let tone_note = if camera_matched {
        "撮影時JPEGに階調を合わせたRAW"
    } else {
        "RawTherapeeカメラ標準"
    };
    let color_note = format!("{} / {tone_note}", rendered.camera);
    // The UI only uses as-shot WB here. Camera conversion belongs entirely to rtengine.
    let raw = RawColor {
        camera: rendered.camera,
        neutral: [1.; 3],
        xyz_to_camera: identity,
        baseline_exposure: 0.,
        calibration: [identity; 2],
        calibration_signature: String::new(),
        analog_balance: [1.; 3],
        as_shot: [
            rendered.temperature,
            -rendered.green.max(0.02).log2() * 150.,
        ],
        profile_signature: advanced_explorer_rtengine::VERSION.into(),
    };
    Ok((
        WorkingImage {
            info: BaseInfo {
                camera_saturation: Some(camera_saturation),
                native_source: Some(source),
                raw: Some(raw),
                raster_wb_temperature: None,
                source_width: rendered.source_width,
                source_height: rendered.source_height,
                width: pixels.width(),
                height: pixels.height(),
                source_kind: "raw-rtengine-linear".into(),
                color_note,
            },
            pixels,
        },
        camera_matched,
    ))
}

pub fn render(working: &WorkingImage, settings: &Settings) -> Result<WorkingImage> {
    let source = working
        .info
        .native_source
        .as_deref()
        .context("Missing rtengine source")?;
    let bound = working.info.width.max(working.info.height);
    let is_raw = working.info.source_kind == "raw-rtengine-linear";
    let adjustments = adjustments(
        settings,
        is_raw,
        bound as f32 / working.info.source_width.max(working.info.source_height) as f32,
    );
    let mut rendered = native(
        source,
        (!is_raw).then_some(&working.pixels),
        if bound >= working.info.source_width.max(working.info.source_height) {
            0
        } else {
            bound
        },
        &adjustments,
    )?;
    if rendered.pixels.dimensions() != working.pixels.dimensions() {
        rendered.pixels = resize_linear(rendered.pixels, bound)?;
    }
    ensure!(
        rendered.pixels.dimensions() == working.pixels.dimensions(),
        "rtengine preview dimensions changed"
    );
    if !is_raw {
        for (pixel, input) in rendered.pixels.pixels_mut().zip(working.pixels.pixels()) {
            pixel[3] = input[3];
        }
    }
    let mut info = working.info.clone();
    info.native_source = None;
    info.raw = None;
    if is_raw {
        info.source_kind = if rendered.camera_matched {
            "rtengine-raw-camera"
        } else {
            "rtengine-raw-linear"
        }
        .into();
    }
    Ok(WorkingImage {
        pixels: rendered.pixels,
        info,
    })
}

fn adjustments(settings: &Settings, is_raw: bool, scale: f32) -> Adjustments {
    Adjustments {
        // Raster tone/WB stays with Rust's calibrated display and linear input paths.
        exposure: if is_raw { settings.exposure } else { 0. },
        temperature: if is_raw { settings.temperature } else { 0. },
        tint: if is_raw { settings.tint } else { 0. },
        raw_temperature: if is_raw {
            settings.raw_temperature.unwrap_or(0.)
        } else {
            0.
        },
        raw_tint: if is_raw { settings.raw_tint } else { 0. },
        sharpen: settings.sharpen,
        sharpen_radius: settings.sharpen_radius,
        sharpen_detail: settings.sharpen_detail,
        sharpen_masking: settings.sharpen_masking,
        noise_luma: settings.noise_luma,
        noise_color: settings.noise_color,
        scale,
    }
}

pub fn remaining(settings: &Settings, is_raw: bool) -> Settings {
    Settings {
        exposure: if is_raw { 0. } else { settings.exposure },
        temperature: if is_raw { 0. } else { settings.temperature },
        tint: if is_raw { 0. } else { settings.tint },
        raw_temperature: None,
        raw_tint: 0.,
        raster_temperature: if is_raw {
            None
        } else {
            settings.raster_temperature
        },
        sharpen: 0.,
        noise_luma: 0.,
        noise_color: 0.,
        ..settings.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ownership_preserves_local_and_rust_adjustments() {
        let settings: Settings = serde_json::from_str(
            r#"{
            "exposure":1,"temperature":30,"tint":-20,"raw_temperature":5200,"raw_tint":15,
            "raster_temperature":8000,"raster_reference_temperature":5200,"raster_tint":10,
            "sharpen":40,"sharpen_radius":2,"sharpen_detail":70,"sharpen_masking":30,
            "noise_luma":15,"noise_color":20,"texture":25,"clarity":10,"contrast":12,
            "highlights":-60,"crop":[0.1,0.1,0.8,0.8],"quarter_turns":1,
            "masks":[{"name":"Local","enabled":true,"invert":false,"amount":1,
              "adjustments":{"exposure":0.5,"sharpen":25,"noise_luma":10},
              "shapes":[{"kind":"linear","operation":"add","start":[0,0],"end":[0,0.5],
                "radius":0.1,"feather":0.5,"opacity":1,"points":[],"color":[0.5,0.5,0.5],"range":[0,1]}]}]
        }"#,
        )
        .unwrap();
        for is_raw in [true, false] {
            let native = adjustments(&settings, is_raw, 0.5);
            let rust = remaining(&settings, is_raw);
            assert_eq!(native.exposure + rust.exposure, settings.exposure);
            assert_eq!(native.temperature + rust.temperature, settings.temperature);
            assert_eq!(native.tint + rust.tint, settings.tint);
            assert_eq!(native.sharpen, settings.sharpen);
            assert_eq!(native.noise_luma, settings.noise_luma);
            assert_eq!(native.noise_color, settings.noise_color);
            let mut expected = settings.clone();
            expected.sharpen = 0.;
            expected.noise_luma = 0.;
            expected.noise_color = 0.;
            expected.raw_temperature = None;
            expected.raw_tint = 0.;
            if is_raw {
                expected.exposure = 0.;
                expected.temperature = 0.;
                expected.tint = 0.;
                expected.raster_temperature = None;
            }
            assert_eq!(rust, expected);
        }
    }
}
