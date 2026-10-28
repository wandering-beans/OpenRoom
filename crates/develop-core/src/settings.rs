use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u8,
    pub masks: Vec<crate::masks::Mask>,
    pub aspect: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub constrain_crop: bool,
    pub exposure: f32,
    pub temperature: f32,
    pub tint: f32,
    /// None retains as-shot WB and the legacy relative adjustments.
    pub raw_temperature: Option<f32>,
    pub raw_tint: f32,
    /// Raster images have baked WB; 6504 K is a reference, not camera metadata.
    pub raster_temperature: Option<f32>,
    /// Captured when editing; None keeps the original 6504 K reference of older recipes.
    pub raster_reference_temperature: Option<f32>,
    pub raster_tint: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub monochrome: bool,
    /// Five output values at fixed input positions 0, .25, .5, .75, 1; master/R/G/B.
    pub curves: [[f32; 5]; 4],
    /// Empty channels retain legacy five-point rendering for existing catalogs.
    pub point_curves: [Vec<[f32; 2]>; 4],
    /// Hue, saturation, luminance for red/orange/yellow/green/aqua/blue/purple/magenta.
    pub mixer: [[f32; 3]; 8],
    /// Hue degrees, saturation, luminance for shadows/midtones/highlights/global.
    pub grading: [[f32; 3]; 4],
    pub grading_balance: f32,
    pub grading_blending: f32,
    pub sharpen: f32,
    pub sharpen_radius: f32,
    pub sharpen_detail: f32,
    pub sharpen_masking: f32,
    pub noise_luma: f32,
    pub noise_color: f32,
    pub ca_auto: bool,
    pub ca_red: f32,
    pub ca_blue: f32,
    pub defringe: bool,
    pub defringe_radius: f32,
    pub defringe_threshold: f32,
    /// Normalized x/y/width/height after rotation, flip and geometry correction.
    pub crop: [f32; 4],
    pub quarter_turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub angle: f32,
    pub perspective_h: f32,
    pub perspective_v: f32,
    pub scale: f32,
    pub distortion: f32,
    pub vignette: f32,
    pub vignette_midpoint: f32,
    pub vignette_feather: f32,
    pub grain: f32,
    pub grain_size: f32,
    pub grain_roughness: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            masks: Vec::new(),
            aspect: 0.,
            offset_x: 0.,
            offset_y: 0.,
            constrain_crop: false,
            exposure: 0.,
            temperature: 0.,
            tint: 0.,
            raw_temperature: None,
            raw_tint: 0.,
            raster_temperature: None,
            raster_reference_temperature: None,
            raster_tint: 0.,
            contrast: 0.,
            highlights: 0.,
            shadows: 0.,
            whites: 0.,
            blacks: 0.,
            texture: 0.,
            clarity: 0.,
            dehaze: 0.,
            vibrance: 0.,
            saturation: 0.,
            monochrome: false,
            curves: [[0., 0.25, 0.5, 0.75, 1.]; 4],
            point_curves: Default::default(),
            mixer: [[0.; 3]; 8],
            grading: [[0.; 3]; 4],
            grading_balance: 0.,
            grading_blending: 50.,
            sharpen: 0.,
            sharpen_radius: 1.,
            sharpen_detail: 25.,
            sharpen_masking: 0.,
            noise_luma: 0.,
            noise_color: 0.,
            ca_auto: false,
            ca_red: 0.,
            ca_blue: 0.,
            defringe: false,
            defringe_radius: 2.,
            defringe_threshold: 13.,
            crop: [0., 0., 1., 1.],
            quarter_turns: 0,
            flip_h: false,
            flip_v: false,
            angle: 0.,
            perspective_h: 0.,
            perspective_v: 0.,
            scale: 100.,
            distortion: 0.,
            vignette: 0.,
            vignette_midpoint: 50.,
            vignette_feather: 50.,
            grain: 0.,
            grain_size: 25.,
            grain_roughness: 50.,
        }
    }
}
impl Settings {
    /// Version 1 defaults are part of the on-disk format; changing them requires a new version.
    pub fn catalog_recipe(&self) -> Result<String> {
        self.validate()?;
        let defaults = serde_json::to_value(Self::default())?;
        let mut recipe = serde_json::to_value(self)?;
        recipe
            .as_object_mut()
            .expect("Settings serializes as an object")
            .retain(|key, value| key == "version" || defaults.get(key) != Some(value));
        Ok(serde_json::to_string(&recipe)?)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported development recipe version");
        let range =
            |value: f32, min: f32, max: f32| value.is_finite() && (min..=max).contains(&value);
        ensure!(
            self.raw_temperature.is_none_or(|v| range(v, 2000., 60000.)),
            "RAW temperature must be 2000..60000 K"
        );
        ensure!(
            range(self.raw_tint, -150., 150.),
            "RAW tint must be -150..150"
        );
        ensure!(
            self.raster_temperature
                .is_none_or(|v| range(v, 2000., 60000.))
                && self
                    .raster_reference_temperature
                    .is_none_or(|v| range(v, 2000., 60000.))
                && range(self.raster_tint, -150., 150.),
            "Raster WB must be 2000..60000 K and tint -150..150"
        );
        ensure!(range(self.exposure, -5., 5.), "Exposure must be -5..5 EV");
        for value in [
            self.aspect,
            self.offset_x,
            self.offset_y,
            self.temperature,
            self.tint,
            self.contrast,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
            self.texture,
            self.clarity,
            self.dehaze,
            self.vibrance,
            self.perspective_h,
            self.perspective_v,
            self.distortion,
            self.vignette,
            self.grading_balance,
            self.ca_red,
            self.ca_blue,
        ] {
            ensure!(
                range(value, -100., 100.),
                "Adjustment must be finite and -100..100"
            );
        }
        ensure!(
            range(self.saturation, -100., 300.),
            "Saturation must be finite and -100..300"
        );
        for value in [
            self.sharpen_detail,
            self.sharpen_masking,
            self.noise_luma,
            self.noise_color,
            self.vignette_midpoint,
            self.vignette_feather,
            self.grain,
            self.grain_size,
            self.grain_roughness,
            self.grading_blending,
            self.defringe_threshold,
        ] {
            ensure!(
                range(value, 0., 100.),
                "Adjustment must be finite and 0..100"
            );
        }
        ensure!(
            range(self.defringe_radius, 0.5, 5.),
            "Defringe radius must be 0.5..5"
        );
        ensure!(
            range(self.sharpen, 0., 150.) && range(self.sharpen_radius, 0.5, 3.),
            "Invalid sharpening settings"
        );
        ensure!(
            range(self.angle, -45., 45.) && range(self.scale, 50., 200.) && self.quarter_turns < 4,
            "Invalid geometry"
        );
        let [x, y, width, height] = self.crop;
        ensure!(
            range(x, 0., 1.)
                && range(y, 0., 1.)
                && range(width, 0.01, 1.)
                && range(height, 0.01, 1.)
                && x + width <= 1.00001
                && y + height <= 1.00001,
            "Crop must lie within the photo"
        );
        ensure!(
            self.curves
                .iter()
                .flatten()
                .all(|value| range(*value, 0., 1.)),
            "Invalid tone curve"
        );
        for points in &self.point_curves {
            ensure!(
                points.is_empty()
                    || (points.len() >= 2
                        && points.len() <= 32
                        && points[0][0] == 0.
                        && points.last().unwrap()[0] == 1.
                        && points
                            .iter()
                            .flatten()
                            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
                        && points.windows(2).all(|p| p[1][0] - p[0][0] >= 0.0001)),
                "Invalid point curve"
            );
        }
        ensure!(
            self.mixer
                .iter()
                .flatten()
                .all(|value| range(*value, -100., 100.)),
            "Invalid color mixer"
        );
        for [hue, saturation, luminance] in self.grading {
            ensure!(
                range(hue, 0., 360.)
                    && range(saturation, 0., 100.)
                    && range(luminance, -100., 100.),
                "Invalid color grading"
            );
        }
        ensure!(self.masks.len() <= 32, "Too many masks (maximum 32)");
        let point_count: usize = self
            .masks
            .iter()
            .flat_map(|m| &m.shapes)
            .map(|s| s.points.len())
            .sum();
        ensure!(point_count <= 32768, "Too many brush points");
        for mask in &self.masks {
            mask.validate()?;
        }
        Ok(())
    }
}
