//! Manual selections use normalized coordinates on the transformed, uncropped photo.
use crate::{Settings, color::ColorTransform};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub name: String,
    pub enabled: bool,
    pub invert: bool,
    pub amount: f32,
    pub shapes: Vec<Shape>,
    /// Only the explicitly supported local adjustments can enter the renderer.
    pub adjustments: std::collections::BTreeMap<String, f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub kind: String,
    pub operation: String,
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub radius: f32,
    pub feather: f32,
    pub opacity: f32,
    pub points: Vec<[f32; 2]>,
    pub color: [f32; 3],
    pub range: [f32; 2],
}
pub const LOCAL_KEYS: &[&str] = &[
    "exposure",
    "temperature",
    "tint",
    "contrast",
    "highlights",
    "shadows",
    "whites",
    "blacks",
    "texture",
    "clarity",
    "dehaze",
    "saturation",
    "vibrance",
    "monochrome",
    "sharpen",
    "noise_luma",
    "noise_color",
];
impl Mask {
    pub fn settings(&self) -> Result<Settings> {
        let mut value = serde_json::to_value(Settings::default())?;
        for (key, amount) in &self.adjustments {
            ensure!(
                LOCAL_KEYS.contains(&key.as_str()),
                "Unsupported local adjustment: {key}"
            );
            if key == "monochrome" {
                ensure!(
                    *amount == 0. || *amount == 1.,
                    "Invalid local monochrome value"
                );
                value[key] = serde_json::json!(*amount == 1.);
            } else {
                value[key] = serde_json::json!(amount);
            }
        }
        let settings: Settings = serde_json::from_value(value)?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.name.len() <= 256 && unit(self.amount),
            "Invalid mask name or amount"
        );
        ensure!(
            !self.shapes.is_empty() && self.shapes.len() <= 256,
            "Invalid mask shape count"
        );
        self.settings()?;
        for shape in &self.shapes {
            ensure!(
                ["brush", "linear", "radial", "color", "luminance"].contains(&shape.kind.as_str()),
                "Invalid mask kind"
            );
            ensure!(
                ["add", "subtract", "intersect"].contains(&shape.operation.as_str()),
                "Invalid mask operation"
            );
            ensure!(
                shape
                    .start
                    .iter()
                    .chain(&shape.end)
                    .chain(&shape.color)
                    .chain(&shape.range)
                    .all(|v| unit(*v)),
                "Invalid mask coordinates"
            );
            ensure!(
                unit(shape.feather)
                    && unit(shape.opacity)
                    && shape.radius.is_finite()
                    && (0.001..=1.).contains(&shape.radius),
                "Invalid brush"
            );
            ensure!(shape.range[0] <= shape.range[1], "Invalid range");
            ensure!(
                shape.points.len() <= 4096 && shape.points.iter().flatten().all(|v| unit(*v)),
                "Brush stroke too large or invalid"
            );
        }
        Ok(())
    }
    pub fn weights(&self, pixels: &image::Rgba32FImage, crop: [f32; 4], aspect: f32) -> Vec<f32> {
        use rayon::prelude::*;
        let (width, height) = pixels.dimensions();
        let mut combined = vec![0f32; width as usize * height as usize];
        for (index, shape) in self.shapes.iter().enumerate() {
            let combine = |total: &mut f32, weight: f32| {
                let selection = weight * shape.opacity;
                *total = match shape.operation.as_str() {
                    "subtract" => *total * (1. - selection),
                    "intersect" if index > 0 => *total * selection,
                    _ => total.max(selection),
                };
            };
            if shape.kind == "brush" {
                let mut weights = vec![0f32; combined.len()];
                // Rasterize each segment only inside its brush footprint, rather than
                // searching every stroke for every output pixel (O(pixels*points)).
                let mut segment = shape.clone();
                segment.points = vec![[0.; 2]; 2];
                for (i, start) in shape.points.iter().enumerate() {
                    let end = shape.points.get(i + 1).unwrap_or(start);
                    let rx = shape.radius / aspect;
                    let x0 = (((start[0].min(end[0]) - rx - crop[0]) / crop[2] * width as f32)
                        .floor()
                        .max(0.) as u32)
                        .min(width);
                    let x1 = (((start[0].max(end[0]) + rx - crop[0]) / crop[2] * width as f32)
                        .ceil()
                        .max(0.) as u32)
                        .min(width);
                    let y0 = (((start[1].min(end[1]) - shape.radius - crop[1]) / crop[3]
                        * height as f32)
                        .floor()
                        .max(0.) as u32)
                        .min(height);
                    let y1 = (((start[1].max(end[1]) + shape.radius - crop[1]) / crop[3]
                        * height as f32)
                        .ceil()
                        .max(0.) as u32)
                        .min(height);
                    segment.points[0] = *start;
                    segment.points[1] = *end;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let nx = crop[0] + (x as f32 + 0.5) / width as f32 * crop[2];
                            let ny = crop[1] + (y as f32 + 0.5) / height as f32 * crop[3];
                            let slot = &mut weights[(y * width + x) as usize];
                            *slot = slot.max(segment.weight(nx, ny, [0.; 3], aspect));
                        }
                    }
                }
                combined
                    .par_iter_mut()
                    .zip(weights)
                    .for_each(|(total, weight)| combine(total, weight));
            } else {
                let needs_color = matches!(shape.kind.as_str(), "color" | "luminance");
                combined.par_iter_mut().enumerate().for_each(|(i, total)| {
                    let nx = crop[0]
                        + (i as u32 % width) as f32 / width as f32 * crop[2]
                        + 0.5 / width as f32 * crop[2];
                    let ny = crop[1]
                        + (i as u32 / width) as f32 / height as f32 * crop[3]
                        + 0.5 / height as f32 * crop[3];
                    let rgb = if needs_color {
                        let pixel = pixels.as_raw();
                        [pixel[i * 4], pixel[i * 4 + 1], pixel[i * 4 + 2]].map(crate::color::srgb)
                    } else {
                        [0.; 3]
                    };
                    combine(total, shape.weight(nx, ny, rgb, aspect));
                });
            }
        }
        combined.par_iter_mut().for_each(|weight| {
            if self.invert {
                *weight = 1. - *weight;
            }
            *weight *= self.amount;
        });
        combined
    }
    pub fn weight(&self, x: f32, y: f32, rgb: [f32; 3], aspect: f32) -> f32 {
        let mut weight: f32 = 0.;
        for (index, shape) in self.shapes.iter().enumerate() {
            let selection = shape.weight(x, y, rgb, aspect) * shape.opacity;
            weight = match shape.operation.as_str() {
                "subtract" => weight * (1. - selection),
                "intersect" if index > 0 => weight * selection,
                _ => weight.max(selection),
            };
        }
        if self.invert {
            weight = 1. - weight;
        }
        weight * self.amount
    }
}
fn unit(value: f32) -> bool {
    value.is_finite() && (0. ..=1.).contains(&value)
}
fn fade(distance: f32, feather: f32) -> f32 {
    ((1. - distance) / feather.max(0.001)).clamp(0., 1.)
}
impl Shape {
    fn weight(&self, x: f32, y: f32, rgb: [f32; 3], aspect: f32) -> f32 {
        let [sx, sy] = self.start;
        let [ex, ey] = self.end;
        match self.kind.as_str() {
            "linear" => {
                let dx = (ex - sx) * aspect;
                let dy = ey - sy;
                let length = dx * dx + dy * dy;
                if length < 1e-10 {
                    return 0.;
                }
                (1. - (((x - sx) * aspect * dx + (y - sy) * dy) / length)).clamp(0., 1.)
            }
            "radial" => fade(
                (((x - sx) / (ex - sx).abs().max(0.001)).powi(2)
                    + ((y - sy) / (ey - sy).abs().max(0.001)).powi(2))
                .sqrt(),
                self.feather,
            ),
            "color" => {
                let distance = rgb
                    .iter()
                    .zip(self.color)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    .sqrt()
                    / 3f32.sqrt();
                fade(distance / self.radius, self.feather)
            }
            "luminance" => {
                let light = crate::color::luma(rgb);
                let edge = self.feather * 0.25;
                ((light - self.range[0] + edge) / edge.max(0.001)).clamp(0., 1.)
                    * ((self.range[1] + edge - light) / edge.max(0.001)).clamp(0., 1.)
            }
            _ => {
                let point = [x * aspect, y];
                let mut distance = f32::INFINITY;
                for (i, sample) in self.points.iter().enumerate() {
                    let a = [sample[0] * aspect, sample[1]];
                    let b = self
                        .points
                        .get(i + 1)
                        .map(|v| [v[0] * aspect, v[1]])
                        .unwrap_or(a);
                    let delta = [b[0] - a[0], b[1] - a[1]];
                    let t = (((point[0] - a[0]) * delta[0] + (point[1] - a[1]) * delta[1])
                        / (delta[0] * delta[0] + delta[1] * delta[1]).max(1e-12))
                    .clamp(0., 1.);
                    distance = distance.min(
                        ((point[0] - a[0] - t * delta[0]).powi(2)
                            + (point[1] - a[1] - t * delta[1]).powi(2))
                        .sqrt(),
                    );
                }
                fade(distance / self.radius, self.feather)
            }
        }
    }
}
pub(crate) fn transform(settings: &Settings) -> ColorTransform {
    ColorTransform::new(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_weights_match_scalar_selection_with_crop_color_and_inversion() {
        let pixels = image::Rgba32FImage::from_fn(37, 23, |x, y| {
            image::Rgba([x as f32 / 30. - 0.1, y as f32 / 25., 0.3, 0.5])
        });
        let crop = [0.1, 0.2, 0.7, 0.6];
        let shape = Shape {
            kind: "linear".into(),
            operation: "add".into(),
            start: [0.2, 0.3],
            end: [0.7, 0.8],
            radius: 0.2,
            feather: 0.5,
            opacity: 0.7,
            points: vec![[0.1, 0.3], [0.8, 0.7]],
            color: [0.3, 0.6, 0.2],
            range: [0.3, 0.7],
        };
        for kind in ["linear", "radial", "brush", "color", "luminance"] {
            for operation in ["add", "subtract", "intersect"] {
                for invert in [false, true] {
                    let mut second = shape.clone();
                    second.kind = kind.into();
                    second.operation = operation.into();
                    let mask = Mask {
                        name: "test".into(),
                        enabled: true,
                        invert,
                        amount: 0.8,
                        shapes: vec![shape.clone(), second],
                        adjustments: Default::default(),
                    };
                    let weights = mask.weights(&pixels, crop, 1.7);
                    for (i, pixel) in pixels.pixels().enumerate() {
                        let x =
                            crop[0] + (i as u32 % 37) as f32 / 37. * crop[2] + 0.5 / 37. * crop[2];
                        let y =
                            crop[1] + (i as u32 / 37) as f32 / 23. * crop[3] + 0.5 / 23. * crop[3];
                        let rgb = [pixel[0], pixel[1], pixel[2]].map(crate::color::srgb);
                        assert!(
                            (weights[i] - mask.weight(x, y, rgb, 1.7)).abs() < 1e-5,
                            "{kind} {operation} invert={invert} pixel={i}"
                        );
                    }
                }
            }
        }
    }
}
