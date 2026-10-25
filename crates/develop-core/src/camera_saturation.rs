//! Match RAW color strength to its embedded camera rendering.
use crate::color::{luma, srgb};
use image::{RgbImage, Rgba32FImage};

fn saturation(rgb: [f32; 3]) -> Option<f32> {
    let high = rgb.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let low = rgb.into_iter().fold(f32::INFINITY, f32::min);
    let saturation = (high - low) / high;
    ((0.1..0.95).contains(&high) && saturation > 0.05).then_some(saturation)
}

pub fn estimate(raw: &Rgba32FImage, camera: &RgbImage) -> f32 {
    let aspect =
        |width: u32, height: u32| width.max(height) as f32 / width.min(height).max(1) as f32;
    // A differently cropped thumbnail is not a reliable color reference for the whole sensor.
    if (aspect(raw.width(), raw.height()) / aspect(camera.width(), camera.height()) - 1.).abs()
        > 0.05
    {
        return 1.;
    }
    let reference: Vec<_> = camera
        .pixels()
        .filter_map(|p| saturation(p.0.map(|v| v as f32 / 255.)))
        .collect();
    if reference.len() < 128 {
        return 1.;
    }
    let target = reference.iter().sum::<f32>() / reference.len() as f32;
    if target < 0.08 {
        return 1.;
    }
    // thumbnail's integer rounding bias also affects float pixels; use the float-safe resampler.
    let raw = image::DynamicImage::ImageRgba32F(raw.clone())
        .resize(256, 256, image::imageops::FilterType::Triangle)
        .into_rgba32f();
    let samples: Vec<_> = raw
        .pixels()
        .filter_map(|p| {
            let rgb = [p[0], p[1], p[2]];
            saturation(rgb.map(srgb)).map(|_| (rgb, luma(rgb)))
        })
        .collect();
    if samples.len() < 128 {
        return 1.;
    }
    let mean = |gain| {
        samples
            .iter()
            .map(|(rgb, neutral)| {
                let display = rgb.map(|v| srgb(neutral + (v - neutral) * gain).clamp(0., 1.));
                let high = display.into_iter().fold(0., f32::max);
                let low = display.into_iter().fold(1., f32::min);
                (high - low) / high.max(1e-6)
            })
            .sum::<f32>()
            / samples.len() as f32
    };
    let initial = mean(1.);
    // Leave already-matched images and small differences from JPEG compression alone.
    if (target - initial).abs() <= 0.02 || (target / initial - 1.).abs() <= 0.08 {
        return 1.;
    }
    // shortcut: One bounded gain matches overall saturation; exact film looks need separate profiles.
    (0..=42)
        .map(|step| (0.75 + step as f32 * 0.025).min(1.8))
        .map(|gain| (gain, (mean(gain) - target).abs()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(1., |(gain, _)| gain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, Rgba};

    #[test]
    fn matches_camera_chroma_in_both_directions_without_changing_gray_or_matched_images() {
        let raw = Rgba32FImage::from_fn(32, 32, |x, _| {
            Rgba([0.15 + x as f32 * 0.003, 0.24, 0.12, 1.])
        });
        let camera = |gain| {
            RgbImage::from_fn(32, 32, |x, y| {
                let p = raw.get_pixel(x, y);
                let rgb = [p[0], p[1], p[2]];
                let neutral = luma(rgb);
                Rgb(rgb.map(|v| (srgb(neutral + (v - neutral) * gain) * 255.).round() as u8))
            })
        };
        assert!((estimate(&raw, &camera(1.35)) - 1.35).abs() <= 0.025);
        let large = image::imageops::resize(&raw, 512, 512, image::imageops::FilterType::Nearest);
        assert!((estimate(&large, &camera(1.35)) - 1.35).abs() <= 0.025);
        assert_eq!(estimate(&raw, &camera(1.)), 1.);
        assert!((estimate(&raw, &camera(0.8)) - 0.8).abs() <= 0.025);
        assert_eq!(estimate(&raw, &camera(0.4)), 0.75);
        assert_eq!(estimate(&raw, &camera(2.5)), 1.8);
        let cropped =
            image::imageops::resize(&camera(1.35), 48, 32, image::imageops::FilterType::Triangle);
        assert_eq!(estimate(&raw, &cropped), 1.);
        assert_eq!(
            estimate(&raw, &RgbImage::from_pixel(32, 32, Rgb([128; 3]))),
            1.
        );
        assert_eq!(estimate(&raw, &RgbImage::new(8, 8)), 1.);
    }
}
