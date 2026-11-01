//! Non-destructive photo development with optional native processing and Metal acceleration.
mod camera_profile;
#[cfg(any(test, all(target_os = "macos", feature = "rtengine")))]
mod camera_saturation;
mod color;
mod curve;
pub mod masks;
mod raster_tone;
mod raw_color;
#[cfg(all(target_os = "macos", feature = "rtengine"))]
mod rtengine;
mod settings;
use anyhow::{Context, Result, ensure};
use image::{
    DynamicImage, ImageBuffer, ImageDecoder, ImageEncoder, Rgba, Rgba32FImage, RgbaImage, imageops,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
pub use settings::Settings;
use std::{borrow::Cow, io::Write, path::Path, time::Instant};

#[cfg(all(target_os = "macos", feature = "rtengine"))]
pub const RENDER_VERSION: &str = "develop-rtengine-5.13-19241791-v12";
#[cfg(not(all(target_os = "macos", feature = "rtengine")))]
pub const RENDER_VERSION: &str = "develop-v9";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_ca_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_saturation: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_source: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<advanced_explorer_media::develop_input::RawColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_wb_temperature: Option<f32>,
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
    pub source_kind: String,
    pub color_note: String,
}
#[derive(Clone)]
pub struct WorkingImage {
    pub pixels: Rgba32FImage,
    pub info: BaseInfo,
}

/// Owns one immutable source and reuses its native stage while Rust-only sliders change.
pub struct PreviewRenderer {
    working: WorkingImage,
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    native_preview: Option<NativePreview>,
}

#[cfg(all(target_os = "macos", feature = "rtengine"))]
struct NativePreview {
    adjustments: advanced_explorer_rtengine::Adjustments,
    source_size: u64,
    source_modified: std::time::SystemTime,
    working: Option<WorkingImage>,
}

impl PreviewRenderer {
    pub fn new(working: WorkingImage) -> Self {
        Self {
            working,
            #[cfg(all(target_os = "macos", feature = "rtengine"))]
            native_preview: None,
        }
    }

    pub fn working(&self) -> &WorkingImage {
        &self.working
    }

    pub fn render(
        &mut self,
        settings: &Settings,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<RenderedImage> {
        #[cfg(all(target_os = "macos", feature = "rtengine"))]
        if self.working.pixels.width().max(self.working.pixels.height()) <= 1600
            && rtengine::required(&self.working, settings)
        {
            let start = Instant::now();
            settings.validate()?;
            ensure!(!cancelled(), "現像対象が変更されました。");
            let adjustments = rtengine::preview_adjustments(&self.working, settings);
            let metadata = std::fs::metadata(self.working.info.native_source.as_ref().unwrap())?;
            let source_modified = metadata.modified()?;
            if self.native_preview.as_ref().is_none_or(|previous| {
                previous.adjustments != adjustments
                    || previous.source_size != metadata.len()
                    || previous.source_modified != source_modified
            }) {
                self.native_preview = Some(NativePreview {
                    adjustments,
                    source_size: metadata.len(),
                    source_modified,
                    working: None,
                });
                // Changing exposure/WB every frame should not pay for a cache copy with no reuse.
                return render(&self.working, settings, cancelled);
            }
            let cached = self.native_preview.as_mut().unwrap();
            if cached.working.is_none() {
                let prepared = rtengine::render(&self.working, settings)?;
                ensure!(!cancelled(), "現像対象が変更されました。");
                // Retain one 1600px RGBA32F frame (41 MB); larger images stay uncached.
                cached.working = Some(prepared);
            }
            let remaining = rtengine::remaining(settings, self.working.info.raw.is_some());
            let mut frame = render(cached.working.as_ref().unwrap(), &remaining, cancelled)?;
            frame.info.elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
            return Ok(frame);
        }
        render(&self.working, settings, cancelled)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderInfo {
    pub width: u32,
    pub height: u32,
    pub histogram: [Vec<u32>; 3],
    pub clipped_shadows: u64,
    pub clipped_highlights: u64,
    pub elapsed_ms: f64,
}
pub struct RenderedImage {
    pub image: RgbaImage,
    pub info: RenderInfo,
}

#[cfg(all(target_os = "macos", feature = "rtengine"))]
pub fn is_native_raw(source: &Path) -> bool {
    rtengine::is_raw(source)
}

/// Decode only once per source/preview size. RAW always uses sensor data.
pub fn prepare(source: &Path, bound: u32) -> Result<WorkingImage> {
    ensure!(bound == 0 || bound <= 4096, "Invalid working image bound");
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    if rtengine::is_raw(source) {
        return rtengine::prepare_raw(source, bound);
    }
    let mut photo = advanced_explorer_media::develop_input::decode(source)?;
    let is_raw = advanced_explorer_media::is_raw(source);
    let (source_width, source_height) = (photo.image.width(), photo.image.height());
    let mut pixels = photo.image.into_rgba32f();
    let needs_linearization = photo.icc.is_some() || !photo.linear;
    let color_note;
    if let Some(icc) = photo.icc {
        let source_profile = moxcms::ColorProfile::new_from_slice(&icc)
            .context("Cannot parse embedded ICC profile")?;
        let transform = source_profile
            .create_transform_f32(
                moxcms::Layout::Rgba,
                &moxcms::ColorProfile::new_srgb(),
                moxcms::Layout::Rgba,
                moxcms::TransformOptions {
                    allow_extended_range_rgb_xyz: true,
                    prefer_fixed_point: false,
                    ..Default::default()
                },
            )
            .context("Unsupported ICC input profile")?;
        let mut converted = vec![0.; pixels.as_raw().len()];
        transform
            .transform(pixels.as_raw(), &mut converted)
            .context("ICC transform failed")?;
        pixels = ImageBuffer::from_raw(source_width, source_height, converted)
            .context("Invalid ICC dimensions")?;
        color_note = "ICCプロファイルから拡張線形sRGBへ変換".into();
    } else if let Some(raw) = &mut photo.raw {
        color_note = raw_color::describe(raw);
    } else if photo.linear && is_raw {
        color_note = "RAWカメラ行列・撮影時WBから線形sRGBへ変換".into();
    } else if photo.linear {
        color_note = "ICCなし・線形sRGBとして処理".into();
    } else {
        color_note = "ICCなし・sRGBとして処理".into();
    }
    pixels.as_mut().par_chunks_mut(4).for_each(|pixel| {
        for value in &mut pixel[..3] {
            let finite = if value.is_finite() { *value } else { 0. };
            *value = if !needs_linearization {
                finite
            } else {
                color::linear(finite)
            };
        }
        pixel[3] = if pixel[3].is_finite() {
            pixel[3].clamp(0., 1.)
        } else {
            0.
        };
    });
    let has_raw_color = photo.raw.is_some();
    let raster_wb_temperature = None;
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    let raster_wb_temperature = if !is_raw {
        advanced_explorer_rtengine::raster_temperature(source).unwrap_or(None)
    } else {
        raster_wb_temperature
    };
    let mut image = WorkingImage {
        pixels,
        info: BaseInfo {
            auto_ca_supported: None,
            camera_saturation: None,
            native_source: if cfg!(all(target_os = "macos", feature = "rtengine")) {
                Some(source.canonicalize()?)
            } else {
                None
            },
            raw: photo.raw,
            raster_wb_temperature,
            source_width,
            source_height,
            width: source_width,
            height: source_height,
            source_kind: if has_raw_color {
                "raw-camera-linear".into()
            } else if photo.linear && is_raw {
                "raw-sensor-linear".into()
            } else if !photo.linear {
                "raster-display-linear".into()
            } else {
                "raster-linear".into()
            },
            color_note,
        },
    };
    if bound > 0 {
        image.pixels = resize_linear(image.pixels, bound)?;
    }
    image.info.width = image.pixels.width();
    image.info.height = image.pixels.height();
    Ok(image)
}

pub fn raw_profile_is_current(info: &BaseInfo) -> bool {
    if info.source_kind == "raw-rtengine-linear" {
        return info.auto_ca_supported.is_some();
    }
    info.raw
        .as_ref()
        .is_none_or(|raw| raw.profile_signature == camera_profile::signature(&raw.camera))
}
pub fn refresh_raw_profile(info: &mut BaseInfo) {
    if info.source_kind == "raw-rtengine-linear" {
        return;
    }
    if let Some(raw) = &mut info.raw {
        info.color_note = raw_color::describe(raw);
    }
}

pub fn resize_linear(pixels: Rgba32FImage, bound: u32) -> Result<Rgba32FImage> {
    ensure!((1..=4096).contains(&bound), "Invalid resize bound");
    let longest = pixels.width().max(pixels.height());
    if longest <= bound {
        return Ok(pixels);
    }
    let ratio = bound as f64 / longest as f64;
    let width = (pixels.width() as f64 * ratio).round().max(1.) as u32;
    let height = (pixels.height() as f64 * ratio).round().max(1.) as u32;
    let source = DynamicImage::ImageRgba32F(pixels);
    let mut destination = DynamicImage::ImageRgba32F(Rgba32FImage::new(width, height));
    let options = fast_image_resize::ResizeOptions::new().resize_alg(
        fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3),
    );
    fast_image_resize::Resizer::new().resize(&source, &mut destination, &options)?;
    Ok(destination.into_rgba32f())
}

/// TIFF retains linear floating point values, including RAW highlight headroom.
pub fn write_base(path: &Path, working: &WorkingImage) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let result = (|| -> Result<()> {
        let mut writer = std::io::BufWriter::new(file);
        working
            .pixels
            .write_to(&mut writer, image::ImageFormat::Tiff)?;
        writer.flush()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
pub fn read_base(path: &Path, info: BaseInfo) -> Result<WorkingImage> {
    ensure!(
        info.raster_wb_temperature
            .is_none_or(|v| v.is_finite() && (2000. ..=60000.).contains(&v)),
        "Invalid raster white balance metadata"
    );
    ensure!(
        info.source_kind != "raw-rtengine-linear" || info.native_source.is_some(),
        "Native working cache is missing its source identity"
    );
    if let Some(raw) = &info.raw {
        raw.validate()?;
    }
    // Check the TIFF header before allocating its pixel buffer, even for our own cache.
    let mut decoder =
        image::codecs::tiff::TiffDecoder::new(std::io::BufReader::new(std::fs::File::open(path)?))?;
    let (width, height) = decoder.dimensions();
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 64_000_000,
        "Working cache exceeds preview limit"
    );
    ensure!(
        (width, height) == (info.width, info.height)
            && info.source_width > 0
            && info.source_height > 0,
        "Working cache dimensions changed"
    );
    ensure!(
        decoder.color_type() == image::ColorType::Rgba32F,
        "Working cache is not linear RGBA32F"
    );
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1024 * 1024 * 1024);
    decoder.set_limits(limits)?;
    let pixels = DynamicImage::from_decoder(decoder)?.into_rgba32f();
    ensure!(
        pixels.as_raw().par_chunks(4).all(|pixel| {
            pixel.iter().all(|value| value.is_finite()) && (0. ..=1.).contains(&pixel[3])
        }),
        "Working cache contains invalid floating-point samples"
    );
    Ok(WorkingImage { pixels, info })
}

fn geometry<'a>(source: Cow<'a, Rgba32FImage>, settings: &Settings) -> Cow<'a, Rgba32FImage> {
    let mut pixels = match settings.quarter_turns {
        1 => Cow::Owned(imageops::rotate90(source.as_ref())),
        2 => Cow::Owned(imageops::rotate180(source.as_ref())),
        3 => Cow::Owned(imageops::rotate270(source.as_ref())),
        _ => source,
    };
    if settings.flip_h {
        imageops::flip_horizontal_in_place(pixels.to_mut());
    }
    if settings.flip_v {
        imageops::flip_vertical_in_place(pixels.to_mut());
    }
    if settings.angle != 0.
        || settings.perspective_h != 0.
        || settings.perspective_v != 0.
        || settings.scale != 100.
        || settings.distortion != 0.
        || settings.aspect != 0.
        || settings.offset_x != 0.
        || settings.offset_y != 0.
    {
        use imageproc::geometric_transformations::{Border, Interpolation, warp_with};
        let (width, height) = (pixels.width() as f32, pixels.height() as f32);
        let (sine, cosine) = (-settings.angle.to_radians()).sin_cos();
        // Cover the unchanged rectangular canvas with one uniform scale and one resampling.
        let pixel_aspect = (width - 1.).max(1.) / (height - 1.).max(1.);
        let rotation_scale = if settings.angle == 0. {
            1.
        } else {
            (cosine.abs() + sine.abs() * pixel_aspect.max(1. / pixel_aspect)) * 1.000001
        };
        pixels = Cow::Owned(warp_with(
            pixels.as_ref(),
            |x, y| {
                let nx = ((x + 0.5 - width * 0.5) / (width * 0.5) - settings.offset_x / 100.)
                    / 2f32.powf(settings.aspect / 100.);
                let ny = (y + 0.5 - height * 0.5) / (height * 0.5) - settings.offset_y / 100.;
                let denominator =
                    1. + settings.perspective_h / 250. * nx + settings.perspective_v / 250. * ny;
                let factor = (1. + settings.distortion / 200. * (nx * nx + ny * ny))
                    / (settings.scale / 100.)
                    / rotation_scale
                    / denominator;
                let x = nx * width * 0.5 * factor;
                let y = ny * height * 0.5 * factor;
                (
                    cosine * x - sine * y + width * 0.5 - 0.5,
                    sine * x + cosine * y + height * 0.5 - 0.5,
                )
            },
            Interpolation::Bilinear,
            Border::Constant(Rgba([0., 0., 0., 0.])),
        ));
    }
    let [x, y, width, height] = settings.crop;
    if settings.crop != [0., 0., 1., 1.] {
        let x = (x * pixels.width() as f32).floor() as u32;
        let y = (y * pixels.height() as f32).floor() as u32;
        let width = ((width * pixels.width() as f32).round() as u32).clamp(1, pixels.width() - x);
        let height =
            ((height * pixels.height() as f32).round() as u32).clamp(1, pixels.height() - y);
        pixels = Cow::Owned(imageops::crop_imm(pixels.as_ref(), x, y, width, height).to_image());
    }
    pixels
}

fn blur(
    image: &DynamicImage,
    sigma: f32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Rgba32FImage> {
    let pixels = image.as_rgba32f().expect("development uses RGBA32F");
    let (width, height) = pixels.dimensions();
    if u64::from(width) * u64::from(height) < 256 * 256 {
        return Ok(image.blur(sigma).into_rgba32f());
    }
    const STRIPE_ROWS: usize = 256;
    let row_len = width as usize * 4;
    // Overlap covers image 0.25's finite Gaussian support (less than 4*sigma+2).
    // Only the centers are copied, so stripe seams and image-edge clamping stay identical.
    let halo = (sigma * 4.).ceil() as usize + 2;
    let mut output = Rgba32FImage::new(width, height);
    // Keep at most four Gaussian scratch images alive, even in an eight-thread export.
    let rows_per_job = if rayon::current_num_threads() > 4 {
        STRIPE_ROWS * (height as usize).div_ceil(STRIPE_ROWS * 4)
    } else {
        STRIPE_ROWS
    };
    output
        .as_mut()
        .par_chunks_mut(row_len * rows_per_job)
        .enumerate()
        .try_for_each(|(job, target)| -> Result<()> {
            for (stripe, target) in target.chunks_mut(row_len * STRIPE_ROWS).enumerate() {
                ensure!(!cancelled(), "現像対象が変更されました。");
                let first = job * rows_per_job + stripe * STRIPE_ROWS;
                let rows = target.len() / row_len;
                let top = first.saturating_sub(halo);
                let bottom = (first + rows + halo).min(height as usize);
                let source = pixels.as_raw()[top * row_len..bottom * row_len].to_vec();
                let stripe = DynamicImage::ImageRgba32F(
                    Rgba32FImage::from_raw(width, (bottom - top) as u32, source).unwrap(),
                );
                let blurred = stripe.blur(sigma).into_rgba32f();
                ensure!(!cancelled(), "現像対象が変更されました。");
                let offset = (first - top) * row_len;
                target.copy_from_slice(&blurred.as_raw()[offset..offset + rows * row_len]);
            }
            Ok(())
        })?;
    Ok(output)
}

fn detail(
    pixels: Rgba32FImage,
    settings: &Settings,
    scale: f32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Rgba32FImage> {
    let mut image = DynamicImage::ImageRgba32F(pixels);
    // The typed DynamicImage path avoids two full float copies per Gaussian filter.
    // Library Gaussian filters are shared by texture, edge-aware detail and noise reduction.
    for (amount, sigma, masking, preservation) in [
        (
            settings.texture / 100. * 0.7,
            // Texture targets medium detail; subpixel kernels disappear in fit previews.
            (4. * scale).max(0.7),
            0.,
            0.55,
        ),
        (
            settings.clarity / 100. * 0.6,
            (12. * scale).max(1.),
            0.,
            0.55,
        ),
        (
            settings.sharpen / 100.,
            (settings.sharpen_radius * scale).max(0.3),
            settings.sharpen_masking / 100.,
            0.4 + settings.sharpen_detail / 100. * 0.6,
        ),
    ] {
        if amount == 0. {
            continue;
        }
        ensure!(!cancelled(), "現像対象が変更されました。");
        let blurred = blur(&image, sigma, cancelled)?;
        ensure!(!cancelled(), "現像対象が変更されました。");
        image
            .as_mut_rgba32f()
            .unwrap()
            .as_mut()
            .par_chunks_mut(4)
            .zip(blurred.as_raw().par_chunks(4))
            .for_each(|(pixel, blur)| {
                let edge = (color::luma([pixel[0], pixel[1], pixel[2]])
                    - color::luma([blur[0], blur[1], blur[2]]))
                .abs();
                let weight = if masking > 0. {
                    (edge / (masking * 0.08 + 0.001)).clamp(0., 1.)
                } else {
                    1.
                };
                for channel in 0..3 {
                    pixel[channel] +=
                        (pixel[channel] - blur[channel]) * amount * weight * preservation;
                }
            });
    }
    if settings.noise_luma > 0. || settings.noise_color > 0. {
        ensure!(!cancelled(), "現像対象が変更されました。");
        let blurred = blur(&image, (1.2 * scale).max(0.5), cancelled)?;
        ensure!(!cancelled(), "現像対象が変更されました。");
        image
            .as_mut_rgba32f()
            .unwrap()
            .as_mut()
            .par_chunks_mut(4)
            .zip(blurred.as_raw().par_chunks(4))
            .for_each(|(pixel, blur)| {
                let luminance = color::luma([pixel[0], pixel[1], pixel[2]]);
                let smoothed = color::luma([blur[0], blur[1], blur[2]]);
                let edge_weight = (1. - (luminance - smoothed).abs() / 0.1).clamp(0., 1.);
                let luma_mix = settings.noise_luma / 100. * edge_weight;
                let color_mix = settings.noise_color / 100. * edge_weight;
                for channel in 0..3 {
                    pixel[channel] = pixel[channel]
                        + (smoothed - luminance) * luma_mix
                        + ((blur[channel] - smoothed) - (pixel[channel] - luminance)) * color_mix;
                }
            });
    }
    Ok(image.into_rgba32f())
}

fn needs_detail(settings: &Settings) -> bool {
    [settings.texture, settings.clarity, settings.sharpen, settings.noise_luma, settings.noise_color]
        .iter().any(|value| *value != 0.)
}

fn spatial<'a>(
    pixels: Cow<'a, Rgba32FImage>,
    info: &BaseInfo,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Cow<'a, Rgba32FImage>> {
    settings.validate()?;
    ensure!(!cancelled(), "現像対象が変更されました。");
    ensure!(
        pixels.width() > 0
            && pixels.height() > 0
            && info.source_width > 0
            && info.source_height > 0,
        "Empty working image"
    );
    let scale = pixels.width().max(pixels.height()) as f32
        / info.source_width.max(info.source_height) as f32;
    let pixels = geometry(pixels, settings);
    if !needs_detail(settings) {
        return Ok(pixels);
    }
    Ok(Cow::Owned(detail(
        pixels.into_owned(),
        settings,
        scale,
        cancelled,
    )?))
}

fn process(
    use_gpu: bool,
    mut working: Cow<'_, WorkingImage>,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Rgba32FImage> {
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    if rtengine::required(&working, settings) {
        settings.validate()?;
        ensure!(!cancelled(), "現像対象が変更されました。");
        let base = rtengine::render(&working, settings)?;
        ensure!(!cancelled(), "現像対象が変更されました。");
        return process(
            use_gpu,
            Cow::Owned(base),
            &rtengine::remaining(settings, working.info.raw.is_some()),
            cancelled,
        );
    }
    let raw_transform = working
        .info
        .raw
        .as_ref()
        .map(|raw| raw_color::RawTransform::new(raw, settings));
    // Detail processing and color-range masks require RGB, not camera sensor channels.
    if let Some(transform) = raw_transform.as_ref() {
        working
            .to_mut()
            .pixels
            .as_mut()
            .par_chunks_mut(4)
            .for_each(|pixel| {
                let rgb = transform.input([pixel[0], pixel[1], pixel[2]]);
                pixel[..3].copy_from_slice(&rgb);
            });
    }
    let info = working.info.clone();
    let (width, height) = working.pixels.dimensions();
    let scale = width.max(height) as f32 / info.source_width.max(info.source_height) as f32;
    let aspect = if settings.quarter_turns.is_multiple_of(2) {
        width as f32 / height as f32
    } else {
        height as f32 / width as f32
    };
    let pixels = match working {
        Cow::Borrowed(working) => Cow::Borrowed(&working.pixels),
        Cow::Owned(working) => Cow::Owned(working.pixels),
    };
    let mut pixels = spatial(pixels, &info, settings, cancelled)?.into_owned();
    for mask in settings.masks.iter().filter(|mask| {
        mask.enabled && mask.amount > 0. && mask.adjustments.values().any(|value| *value != 0.)
    }) {
        ensure!(!cancelled(), "現像対象が変更されました。");
        let local = mask.settings()?;
        let local_transform = masks::transform(&local);
        let detailed = if needs_detail(&local) {
            Some(detail(pixels.clone(), &local, scale, cancelled)?)
        } else {
            None
        };
        let weights = mask.weights(&pixels, settings.crop, aspect);
        let (width, height) = pixels.dimensions();
        pixels
            .as_mut()
            .par_chunks_mut(width as usize * 4)
            .enumerate()
            .try_for_each(|(y, row)| -> Result<()> {
                ensure!(!cancelled(), "現像対象が変更されました。");
                for (x, pixel) in row.chunks_mut(4).enumerate() {
                    let nx = settings.crop[0] + (x as f32 + 0.5) / width as f32 * settings.crop[2];
                    let ny = settings.crop[1] + (y as f32 + 0.5) / height as f32 * settings.crop[3];
                    let weight = weights[y * width as usize + x];
                    if weight == 0. {
                        continue;
                    }
                    let detail_pixel = detailed.as_ref().map_or(&pixel[..], |image| {
                        let offset = (y * width as usize + x) * 4;
                        &image.as_raw()[offset..offset + 4]
                    });
                    let adjusted = local_transform
                        .apply(
                            [detail_pixel[0], detail_pixel[1], detail_pixel[2]],
                            &local,
                            nx,
                            ny,
                        )
                        .map(color::linear);
                    for channel in 0..3 {
                        pixel[channel] += (adjusted[channel] - pixel[channel]) * weight;
                    }
                }
                Ok(())
            })?;
    }
    let transform = if let Some(raw) = raw_transform {
        color::ColorTransform::for_raw(settings, raw)
    } else if info.source_kind == "rtengine-raw-linear" {
        color::ColorTransform::for_native_raw(settings)
    } else if info.source_kind == "raster-display-linear" {
        color::ColorTransform::for_raster(settings)
    } else {
        color::ColorTransform::new(settings)
    }.with_camera_saturation(info.camera_saturation.unwrap_or(1.));
    let (width, height) = pixels.dimensions();
    #[cfg(all(target_os = "macos", feature = "metal"))]
    let gpu_applied = use_gpu && transform.apply_gpu(&mut pixels, settings);
    #[cfg(not(all(target_os = "macos", feature = "metal")))]
    let gpu_applied = { let _ = use_gpu; false };
    if !gpu_applied {
        pixels
            .as_mut()
            .par_chunks_mut(width as usize * 4)
            .enumerate()
            .try_for_each(|(y, row)| -> Result<()> {
                ensure!(!cancelled(), "現像対象が変更されました。");
                for (x, pixel) in row.chunks_mut(4).enumerate() {
                    let rgb = transform.apply(
                        [pixel[0], pixel[1], pixel[2]],
                        settings,
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    );
                    pixel[..3].copy_from_slice(&rgb);
                }
                Ok(())
            })?;
    }
    ensure!(!cancelled(), "現像対象が変更されました。");
    if settings.constrain_crop {
        // Fit the existing aspect ratio inside each opaque rectangle, including skewed borders.
        let (width, height) = pixels.dimensions();
        let mut heights = vec![0u32; width as usize + 1];
        let mut best = (0u64, 0u32, 0u32, 0u32, 0u32);
        for y in 0..height {
            ensure!(!cancelled(), "現像対象が変更されました。");
            for x in 0..width {
                heights[x as usize] = if pixels.get_pixel(x, y)[3] >= 0.999 {
                    heights[x as usize] + 1
                } else {
                    0
                };
            }
            let mut stack: Vec<usize> = Vec::new();
            for x in 0..=width as usize {
                while stack.last().is_some_and(|&last| heights[last] > heights[x]) {
                    let h = heights[stack.pop().unwrap()];
                    let left = stack.last().map_or(0, |last| last + 1);
                    let w = (x - left) as u32;
                    let (crop_width, crop_height) =
                        if u64::from(w) * u64::from(height) > u64::from(h) * u64::from(width) {
                            (
                                (u64::from(h) * u64::from(width) / u64::from(height)) as u32,
                                h,
                            )
                        } else {
                            (
                                w,
                                (u64::from(w) * u64::from(height) / u64::from(width)) as u32,
                            )
                        };
                    let area = u64::from(crop_width) * u64::from(crop_height);
                    if area > best.0 {
                        best = (
                            area,
                            left as u32 + (w - crop_width) / 2,
                            y + 1 - h + (h - crop_height) / 2,
                            crop_width,
                            crop_height,
                        );
                    }
                }
                stack.push(x);
            }
        }
        ensure!(best.0 > 0, "変形後に有効な画像領域がありません");
        pixels = imageops::crop_imm(&pixels, best.1, best.2, best.3, best.4).to_image();
    }
    Ok(pixels)
}

pub fn render(
    working: &WorkingImage,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<RenderedImage> {
    let start = Instant::now();
    let processed = process(true, Cow::Borrowed(working), settings, cancelled)?;
    let image = preview_pixels(&processed);
    let histogram = histogram(&image);
    let info = RenderInfo {
        width: image.width(),
        height: image.height(),
        clipped_shadows: histogram.iter().map(|bins| u64::from(bins[0])).sum(),
        clipped_highlights: histogram.iter().map(|bins| u64::from(bins[255])).sum(),
        histogram,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
    };
    Ok(RenderedImage { image, info })
}
fn preview_pixels(processed: &Rgba32FImage) -> RgbaImage {
    let mut image = RgbaImage::new(processed.width(), processed.height());
    image
        .as_mut()
        .par_chunks_mut(4096)
        .zip(processed.as_raw().par_chunks(4096))
        .for_each(|(output, input)| {
            for (output, value) in output.iter_mut().zip(input) {
                // Match image's float conversion, including its treatment of NaN and rounding.
                let value = if *value < 1. { value.max(0.) } else { 1. };
                *output = (value * 255.).round() as u8;
            }
        });
    image
}
fn empty_histogram() -> [Vec<u32>; 3] {
    std::array::from_fn(|_| vec![0; 256])
}
fn merge_histograms(mut first: [Vec<u32>; 3], second: [Vec<u32>; 3]) -> [Vec<u32>; 3] {
    for channel in 0..3 {
        for value in 0..256 {
            first[channel][value] += second[channel][value];
        }
    }
    first
}
pub fn histogram(image: &RgbaImage) -> [Vec<u32>; 3] {
    image
        .as_raw()
        .par_chunks(4)
        .fold(empty_histogram, |mut bins, pixel| {
            if pixel[3] != 0 {
                for channel in 0..3 {
                    bins[channel][pixel[channel] as usize] += 1;
                }
            }
            bins
        })
        .reduce(empty_histogram, merge_histograms)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExportOptions {
    pub quality: u8,
    pub long_edge: u32,
    pub max_bytes: u64,
    pub border: Option<ExportBorder>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderColor {
    White,
    Black,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportBorder {
    pub color: BorderColor,
    /// Width / height of the finished image; zero keeps the photo's aspect ratio.
    pub aspect_ratio: f64,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            quality: 92,
            long_edge: 0,
            max_bytes: 0,
            border: None,
        }
    }
}
impl ExportOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=100).contains(&self.quality) && self.long_edge <= 65535,
            "Invalid export options"
        );
        if let Some(border) = &self.border {
            ensure!(
                border.aspect_ratio == 0. || (0.5..=2.).contains(&border.aspect_ratio),
                "Invalid border aspect ratio"
            );
        }
        Ok(())
    }
}

/// Decode and export once; native RAW stages can be released before output filtering.
pub fn export_source_with_options(
    source: &Path,
    settings: &Settings,
    destination: &Path,
    format: &str,
    options: &ExportOptions,
) -> Result<()> {
    settings.validate()?;
    options.validate()?;
    ensure!(!destination.exists(), "Destination already exists");
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    if rtengine::is_raw(source) {
        let working = rtengine::prepare_export(source, settings)?;
        return export_working(
            Cow::Owned(working),
            &rtengine::remaining(settings, true),
            destination,
            format,
            options,
        );
    }
    let working = prepare(source, 0)?;
    export_working(Cow::Owned(working), settings, destination, format, options)
}

/// Always creates a new destination; never truncates an existing file or original.
pub fn export(
    working: &WorkingImage,
    settings: &Settings,
    destination: &Path,
    format: &str,
) -> Result<()> {
    export_with_options(
        working,
        settings,
        destination,
        format,
        &ExportOptions::default(),
    )
}
pub fn export_with_options(
    working: &WorkingImage,
    settings: &Settings,
    destination: &Path,
    format: &str,
    options: &ExportOptions,
) -> Result<()> {
    export_working(
        Cow::Borrowed(working),
        settings,
        destination,
        format,
        options,
    )
}

fn export_working(
    working: Cow<'_, WorkingImage>,
    settings: &Settings,
    destination: &Path,
    format: &str,
    options: &ExportOptions,
) -> Result<()> {
    options.validate()?;
    ensure!(
        ["jpeg", "png", "tiff"].contains(&format),
        "Unsupported export format"
    );
    ensure!(!destination.exists(), "Destination already exists");
    let pixels = process(false, working, settings, &|| false)?;
    let image = DynamicImage::ImageRgba32F(pixels);
    let image = if let Some(border) = &options.border {
        add_export_border(image, border, options.long_edge)?
    } else if options.long_edge > 0 && image.width().max(image.height()) > options.long_edge {
        image.resize(
            options.long_edge,
            options.long_edge,
            imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = (|| -> Result<()> {
        let mut writer = std::io::BufWriter::new(file);
        let profile = moxcms::ColorProfile::new_srgb().encode()?;
        match format {
            "jpeg" => {
                let pixels = image.to_rgb8();
                let encode = |quality| -> Result<Vec<u8>> {
                    let mut bytes = Vec::new();
                    let mut encoder =
                        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
                    encoder.set_icc_profile(profile.clone())?;
                    encoder.encode_image(&pixels)?;
                    Ok(bytes)
                };
                let mut bytes = encode(options.quality)?;
                if options.max_bytes > 0 && bytes.len() as u64 > options.max_bytes {
                    let mut best = None;
                    let (mut low, mut high) = (1, options.quality - 1);
                    while low <= high {
                        let quality = low + (high - low) / 2;
                        let candidate = encode(quality)?;
                        if candidate.len() as u64 <= options.max_bytes {
                            best = Some(candidate);
                            low = quality + 1;
                        } else {
                            high = quality - 1;
                        }
                    }
                    bytes = best.ok_or_else(|| {
                        anyhow::anyhow!(
                            "容量上限に収まりません。上限を増やすか、長辺を小さくしてください。"
                        )
                    })?;
                }
                writer.write_all(&bytes)?;
            }
            "png" => {
                let mut encoder = image::codecs::png::PngEncoder::new(&mut writer);
                encoder.set_icc_profile(profile)?;
                DynamicImage::ImageRgba16(image.to_rgba16()).write_with_encoder(encoder)?;
            }
            "tiff" => {
                let mut encoder = image::codecs::tiff::TiffEncoder::new(&mut writer);
                encoder.set_icc_profile(profile)?;
                DynamicImage::ImageRgba16(image.to_rgba16()).write_with_encoder(encoder)?;
            }
            _ => unreachable!(),
        }
        writer.flush()?;
        ensure!(
            options.max_bytes == 0 || writer.get_ref().metadata()?.len() <= options.max_bytes,
            "容量上限に収まりません。上限を増やすか、長辺を小さくしてください。"
        );
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn add_export_border(
    image: DynamicImage,
    border: &ExportBorder,
    long_edge: u32,
) -> Result<DynamicImage> {
    let (width, height) = (image.width() as f64, image.height() as f64);
    let margin = (width.min(height) * 0.04).ceil().max(1.);
    let ratio = if border.aspect_ratio == 0. {
        width / height
    } else {
        border.aspect_ratio
    };
    let canvas_width = (width + margin * 2.)
        .max((height + margin * 2.) * ratio)
        .ceil();
    let canvas_height = (canvas_width / ratio).ceil();
    let scale = if long_edge == 0 {
        1.
    } else {
        (long_edge as f64 / canvas_width.max(canvas_height)).min(1.)
    };
    let canvas_width = (canvas_width * scale).round().max(1.) as u32;
    let canvas_height = (canvas_height * scale).round().max(1.) as u32;
    ensure!(
        canvas_width >= 3
            && canvas_height >= 3
            && canvas_width <= 65535
            && canvas_height <= 65535
            && u64::from(canvas_width) * u64::from(canvas_height) <= 268_435_456,
        "枠を追加できない画像サイズです。長辺を3〜65535pxの範囲で調整してください。"
    );
    // Resize the photo before allocating its background, keeping full-resolution export memory bounded.
    let photo = if scale < 1. {
        image.resize(
            ((width * scale).round() as u32).clamp(1, canvas_width - 2),
            ((height * scale).round() as u32).clamp(1, canvas_height - 2),
            imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let value = match border.color {
        BorderColor::White => 1.,
        BorderColor::Black => 0.,
    };
    let mut canvas =
        Rgba32FImage::from_pixel(canvas_width, canvas_height, Rgba([value, value, value, 1.]));
    let left = (canvas_width - photo.width()) / 2;
    let top = (canvas_height - photo.height()) / 2;
    imageops::overlay(
        &mut canvas,
        &photo.into_rgba32f(),
        i64::from(left),
        i64::from(top),
    );
    Ok(DynamicImage::ImageRgba32F(canvas))
}

#[cfg(test)]
mod regressions {
    use super::*;

    #[test]
    fn parallel_preview_conversion_matches_image_for_extended_values_and_rounding() {
        let mut values = vec![
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -1.,
            0.,
            1.,
            2.,
            -0.,
        ];
        for byte in 0..255 {
            let midpoint = (byte as f32 + 0.5) / 255.;
            values.extend([
                midpoint.next_down(),
                midpoint,
                midpoint.next_up(),
                byte as f32 / 255.,
            ]);
        }
        let values = values.repeat(17);
        let pixels = Rgba32FImage::from_raw((values.len() / 4) as u32, 1, values).unwrap();
        assert_eq!(
            preview_pixels(&pixels),
            DynamicImage::ImageRgba32F(pixels).into_rgba8()
        );
    }

    #[test]
    fn owned_processing_matches_borrowed_without_mutating_the_source() {
        let working = WorkingImage {
            pixels: Rgba32FImage::from_fn(48, 32, |x, y| {
                Rgba([x as f32 / 40., y as f32 / 30., 0.3, (x % 4) as f32 / 3.])
            }),
            info: BaseInfo {
                auto_ca_supported: None,
                camera_saturation: None,
                native_source: None,
                raw: None,
                raster_wb_temperature: None,
                width: 48,
                height: 32,
                source_width: 96,
                source_height: 64,
                source_kind: "test".into(),
                color_note: String::new(),
            },
        };
        let original = working.pixels.clone();
        for settings in [
            Settings::default(),
            Settings {
                quarter_turns: 1,
                crop: [0.1, 0.2, 0.7, 0.6],
                texture: 15.,
                clarity: 10.,
                sharpen: 40.,
                noise_luma: 15.,
                exposure: 0.5,
                ..Default::default()
            },
        ] {
            let borrowed = process(false, Cow::Borrowed(&working), &settings, &|| false).unwrap();
            let owned = process(false, Cow::Owned(working.clone()), &settings, &|| false).unwrap();
            assert_eq!(owned, borrowed);
            assert_eq!(working.pixels, original);
        }
    }

    #[test]
    fn parallel_blur_matches_generic_float_blur_including_stripe_seams() {
        let pixels = Rgba32FImage::from_fn(137, 1301, |x, y| {
            Rgba([
                x as f32 / 10. - 0.5,
                y as f32 / 10.,
                ((x * 7 + y * 13) % 47) as f32 / 47.,
                0.5,
            ])
        });
        let dynamic = DynamicImage::ImageRgba32F(pixels.clone());
        for sigma in [0.3, 0.5, 1., 3.2, 12.] {
            assert_eq!(
                blur(&dynamic, sigma, &|| false).unwrap(),
                imageops::blur(&pixels, sigma)
            );
        }
    }
}

#[cfg(test)]
mod slider_audit;
