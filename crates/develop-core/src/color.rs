use crate::Settings;

pub struct ColorTransform {
    camera_saturation: f32,
    raster: Option<crate::raster_tone::RasterTone>,
    raw: Option<crate::raw_color::RawTransform>,
    raster_wb: Option<[[f32; 3]; 3]>,
    gains: [f32; 3],
    tone: Vec<f32>,
    has_tone: bool,
    has_curves: [bool; 4],
    curve_tables: [Vec<f32>; 4],
    curve_to_wide: [[f32; 3]; 3],
    curve_from_wide: [[f32; 3]; 3],
    has_mixer: bool,
    has_grading: bool,
}
impl ColorTransform {
    pub fn new(settings: &Settings) -> Self {
        Self::with_raw_tone(settings, false)
    }
    pub fn for_native_raw(settings: &Settings) -> Self {
        Self::with_raw_tone(settings, true)
    }
    fn with_raw_tone(settings: &Settings, raw_tone: bool) -> Self {
        let raster_wb = settings.raster_temperature.map(|kelvin| {
            crate::raw_color::raster_white_balance(
                kelvin,
                settings.raster_tint,
                settings.raster_reference_temperature.unwrap_or(6504.),
            )
        });
        let exposure = 2f32.powf(settings.exposure);
        let (temperature, tint) = if raster_wb.is_some() {
            (0., 0.)
        } else {
            (settings.temperature, settings.tint)
        };
        let gains = [
            2f32.powf(temperature * 0.004),
            2f32.powf(-tint * 0.004),
            2f32.powf(-temperature * 0.004),
        ]
        .map(|gain| gain * exposure);
        let has_tone = raw_tone
            || [
                settings.contrast,
                settings.highlights,
                settings.shadows,
                settings.whites,
                settings.blacks,
                settings.dehaze,
            ]
            .iter()
            .any(|value| *value != 0.);
        let tone = (0..=4096)
            .map(|index| {
                let input = (index as f32 / 4096.).powi(2) * 16.;
                // Blend the calibrated RAW curve with linear light to retain shadow separation.
                let input = if raw_tone && input < 1. {
                    0.5 * (input + crate::raw_color::default_tone(input))
                } else {
                    input
                };
                let mut output = contrast(input, settings.contrast);
                // Compose monotone mappings; overlapping gains from the original input can reverse highlights.
                output *=
                    2f32.powf(settings.highlights * 0.01 * srgb(output).clamp(0., 1.).powi(2));
                output *=
                    2f32.powf(settings.shadows * 0.018 * (1. - srgb(output).clamp(0., 1.)).powi(2));
                output *= 2f32.powf(settings.whites * 0.008 * srgb(output).clamp(0., 1.).powi(4));
                output = black_point(output, settings.blacks);
                let haze = settings.dehaze / 100.;
                ((output - haze * 0.025) / (1. - haze * 0.15)).max(0.)
            })
            .collect();
        let srgb_profile = moxcms::ColorProfile::new_srgb();
        let wide_profile = moxcms::ColorProfile::new_pro_photo_rgb();
        Self {
            camera_saturation: 1.,
            raster: None,
            raw: None,
            raster_wb,
            gains,
            tone,
            has_tone,
            has_curves: std::array::from_fn(|channel| {
                let points = &settings.point_curves[channel];
                if points.is_empty() {
                    settings.curves[channel] != [0., 0.25, 0.5, 0.75, 1.]
                } else {
                    points.iter().any(|p| p[0] != p[1])
                }
            }),
            curve_tables: std::array::from_fn(|channel| {
                if settings.point_curves[channel].is_empty() {
                    return Vec::new();
                }
                let curve = crate::curve::PointCurve::new(&settings.point_curves[channel]);
                (0..=4096).map(|i| curve.sample(i as f32 / 4096.)).collect()
            }),
            curve_to_wide: srgb_profile
                .transform_matrix(&wide_profile)
                .v
                .map(|row| row.map(|v| v as f32)),
            curve_from_wide: wide_profile
                .transform_matrix(&srgb_profile)
                .v
                .map(|row| row.map(|v| v as f32)),
            has_mixer: settings.mixer.iter().flatten().any(|value| *value != 0.),
            has_grading: settings
                .grading
                .iter()
                .any(|grade| grade[1] != 0. || grade[2] != 0.),
        }
    }
    pub fn for_raster(settings: &Settings) -> Self {
        let mut remaining = settings.clone();
        if settings.raster_temperature.is_some() {
            remaining.temperature = 0.;
            remaining.tint = 0.;
        }
        let raster = crate::raster_tone::RasterTone::new(&remaining);
        remaining.exposure = 0.;
        remaining.temperature = 0.;
        remaining.tint = 0.;
        remaining.contrast = 0.;
        let mut transform = Self::new(&remaining);
        transform.raster = raster;
        transform
    }
    pub fn with_camera_saturation(mut self, gain: f32) -> Self {
        if (0.75..=1.8).contains(&gain) {
            self.camera_saturation = gain;
        }
        self
    }
    pub fn for_raw(settings: &Settings, raw: crate::raw_color::RawTransform) -> Self {
        let mut remaining = settings.clone();
        remaining.raster_temperature = None;
        remaining.exposure = 0.;
        remaining.temperature = 0.;
        remaining.tint = 0.;
        remaining.contrast = 0.;
        let mut result = Self::new(&remaining);
        result.raw = Some(raw);
        result
    }
    fn map_curve(&self, value: f32, channel: usize, settings: &Settings) -> f32 {
        if !self.has_curves[channel] {
            return value;
        }
        let table = &self.curve_tables[channel];
        if table.is_empty() {
            return curve(value, &settings.curves[channel]);
        }
        let position = value.clamp(0., 1.) * 4096.;
        let index = (position as usize).min(4095);
        let fraction = position - index as f32;
        // Curve endpoints must not discard extended highlight or out-of-gamut channel values.
        table[index] * (1. - fraction) + table[index + 1] * fraction + (value - value.clamp(0., 1.))
    }
    pub fn apply(&self, mut rgb: [f32; 3], settings: &Settings, x: f32, y: f32) -> [f32; 3] {
        if let Some(matrix) = self.raster_wb {
            rgb = matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]);
        }
        if let Some(raw) = &self.raw {
            rgb = raw.apply(rgb);
        }
        if let Some(raster) = &self.raster {
            rgb = raster.apply(rgb);
        }
        for (value, gain) in rgb.iter_mut().zip(self.gains) {
            // Extended linear sRGB needs negative channels to retain colors outside its gamut.
            *value *= gain;
        }
        if self.has_tone {
            let luminance = luma(rgb);
            let position = (luminance / 16.).clamp(0., 1.).sqrt() * 4096.;
            let index = (position as usize).min(4095);
            let fraction = position - index as f32;
            let mapped = if luminance > 16. {
                // Above white the tone function is affine; do not cap exposure at the LUT limit.
                let last_input = (4095f32 / 4096.).powi(2) * 16.;
                let slope = (self.tone[4096] - self.tone[4095]) / (16. - last_input);
                self.tone[4096] + (luminance - 16.) * slope
            } else {
                self.tone[index] * (1. - fraction) + self.tone[index + 1] * fraction
            };
            let ratio = if luminance > 1e-8 {
                mapped / luminance
            } else {
                1.
            };
            rgb = if luminance > 1e-8 {
                rgb.map(|value| srgb(value * ratio))
            } else {
                [srgb(mapped); 3]
            };
        } else {
            rgb = rgb.map(srgb);
        }
        if self.has_curves.iter().any(|active| *active) {
            use crate::raster_tone::multiply;
            // Measured LRC channel curves match ProPhoto primaries with the sRGB transfer curve.
            let mut wide = multiply(self.curve_to_wide, rgb.map(linear)).map(srgb);
            if self.has_curves[0] {
                // shortcut: Composite hue preservation is approximate; extend LRC calibration before claiming parity.
                let low = wide.into_iter().fold(f32::INFINITY, f32::min);
                let high = wide.into_iter().fold(f32::NEG_INFINITY, f32::max);
                let mapped_low = self.map_curve(low, 0, settings);
                let mapped_high = self.map_curve(high, 0, settings);
                wide = wide.map(|v| {
                    if high - low > 1e-7 {
                        mapped_low + (v - low) / (high - low) * (mapped_high - mapped_low)
                    } else {
                        mapped_low
                    }
                });
            }
            for (channel, value) in wide.iter_mut().enumerate() {
                *value = self.map_curve(*value, channel + 1, settings);
            }
            rgb = multiply(self.curve_from_wide, wide.map(linear)).map(srgb);
        }
        if self.camera_saturation != 1. || settings.saturation != 0. || settings.vibrance != 0. {
            let linear_rgb = rgb.map(linear);
            let neutral = luma(linear_rgb);
            let spread = rgb.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - rgb.iter().copied().fold(f32::INFINITY, f32::min);
            let skin_hue = hsl(rgb)[0];
            let skin_protection =
                1. - (1. - circular_distance(skin_hue, 30.) / 30.).clamp(0., 1.) * 0.5;
            let saturation = 1.
                + settings.saturation / 100.
                + settings.vibrance / 100. * (1. - spread.clamp(0., 1.)) * skin_protection;
            let saturation = saturation.max(0.) * self.camera_saturation;
            rgb = linear_rgb.map(|value| srgb(neutral + (value - neutral) * saturation));
        }
        if self.has_mixer {
            let peak = rgb.into_iter().fold(1., f32::max);
            let [mut hue, mut saturation, mut lightness] =
                hsl(rgb.map(|value| (value / peak).clamp(0., 1.)));
            let mut adjustment = [0.; 3];
            let mut weight_sum = 0.;
            for (center, values) in [0., 30., 60., 120., 180., 240., 270., 300.]
                .into_iter()
                .zip(settings.mixer)
            {
                let weight = (1. - circular_distance(hue, center) / 45.).max(0.);
                weight_sum += weight;
                for channel in 0..3 {
                    adjustment[channel] += values[channel] * weight;
                }
            }
            if saturation > 1e-8 && weight_sum > 0. && adjustment.iter().any(|value| *value != 0.) {
                hue += adjustment[0] / weight_sum * 0.45;
                saturation *= 1. + adjustment[1] / weight_sum / 100.;
                let delta = adjustment[2] / weight_sum / 100.;
                lightness += delta
                    * if delta >= 0. {
                        1. - lightness
                    } else {
                        lightness
                    };
                rgb = from_hsl(hue, saturation.clamp(0., 1.), lightness.clamp(0., 1.))
                    .map(|value| value * peak);
            }
        }
        if settings.monochrome {
            rgb = [srgb(luma(rgb.map(linear))); 3];
        }
        if self.has_grading {
            let luminance = (luma(rgb) + settings.grading_balance / 200.).clamp(0., 1.);
            let exponent = 4. - settings.grading_blending / 100. * 3.;
            let weights = [
                (1. - luminance).powf(exponent),
                (1. - (2. * luminance - 1.).abs()).powf(exponent),
                luminance.powf(exponent),
            ];
            let sum = weights.iter().sum::<f32>().max(1e-6);
            for (index, [hue, saturation, brightness]) in settings.grading.iter().enumerate() {
                let weight = if index == 3 { 1. } else { weights[index] / sum };
                let tint = from_hsl(*hue, 1., 0.5);
                for channel in 0..3 {
                    rgb[channel] += weight
                        * ((tint[channel] - 0.5) * saturation / 100. * 0.3
                            + brightness / 100. * 0.2);
                }
            }
        }
        if settings.vignette != 0. {
            let radius = ((x * 2. - 1.).powi(2) + (y * 2. - 1.).powi(2)).sqrt() / 2f32.sqrt();
            let start = settings.vignette_midpoint / 100. * 0.8;
            let feather = (settings.vignette_feather / 100. * (1. - start)).max(0.01);
            let mask = smoothstep(((radius - start) / feather).clamp(0., 1.));
            let gain = 2f32.powf(settings.vignette / 100. * 2. * mask);
            rgb = rgb.map(|value| srgb(linear(value) * gain));
        }
        if settings.grain > 0. {
            let size = 1. + settings.grain_size * 0.2;
            let noise = grain(x * 4096. / size, y * 4096. / size);
            let rough = grain(x * 4096., y * 4096.);
            let noise = noise * (1. - settings.grain_roughness / 200.)
                + rough * settings.grain_roughness / 200.;
            let strength =
                settings.grain / 100. * 0.14 * (0.3 + (1. - (luma(rgb) * 2. - 1.).abs()) * 0.7);
            rgb = rgb.map(|value| value + noise * strength);
        }
        rgb
    }
}
fn black_point(input: f32, amount: f32) -> f32 {
    if amount == 0. || input <= 0. || input >= 1. {
        return input;
    }
    if amount < 0. {
        return (input + amount * 0.002 * (1. - srgb(input)).powi(4)).max(0.);
    }
    // shortcut: Fixed LRC ramp response; image-adaptive local contrast still differs from Adobe.
    const DATA: &[u8; 3 * 129 * 4] = include_bytes!("black_calibration.bin");
    let position = srgb(input) * 128.;
    let index = (position as usize).min(127);
    let fraction = position - index as f32;
    let setting = (amount / 50.).clamp(0., 2.);
    let row = (setting as usize).min(1);
    let sample = |row: usize| {
        let word = |column| {
            let offset = (row * 129 + column) * 4;
            f32::from_le_bytes(DATA[offset..offset + 4].try_into().unwrap())
        };
        word(index) * (1. - fraction) + word(index + 1) * fraction
    };
    linear(sample(row) * (1. - (setting - row as f32)) + sample(row + 1) * (setting - row as f32))
}
fn contrast(input: f32, amount: f32) -> f32 {
    if amount == 0. || input <= 0. || input >= 1. {
        return input;
    }
    // A bounded perceptual S-curve preserves black, middle gray and white.
    // The old unbounded power curve pulled exposed whites back toward gray.
    let display = srgb(input);
    let exponent = 2f32.powf(amount / 100.);
    let light = display.powf(exponent);
    let dark = (1. - display).powf(exponent);
    linear(light / (light + dark))
}
pub fn linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else if value <= 1. {
        static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            (0..=4096)
                .map(|i| {
                    let v = i as f32 / 4096.;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                })
                .collect()
        });
        transfer_sample(table, value * 4096.)
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
pub fn srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else if value <= 1. {
        static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            (0..=65536)
                .map(|i| {
                    let v = i as f32 / 65536.;
                    if v <= 0.0031308 {
                        v * 12.92
                    } else {
                        1.055 * v.powf(1. / 2.4) - 0.055
                    }
                })
                .collect()
        });
        transfer_sample(table, value * 65536.)
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}
fn transfer_sample(table: &[f32], position: f32) -> f32 {
    let index = (position as usize).min(table.len() - 2);
    let fraction = position - index as f32;
    table[index] + (table[index + 1] - table[index]) * fraction
}
pub fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}
fn curve(value: f32, points: &[f32; 5]) -> f32 {
    let position = value.clamp(0., 1.) * 4.;
    let index = (position as usize).min(3);
    let fraction = position - index as f32;
    points[index] * (1. - fraction) + points[index + 1] * fraction + (value - value.clamp(0., 1.))
}
fn circular_distance(first: f32, second: f32) -> f32 {
    ((first - second + 180.).rem_euclid(360.) - 180.).abs()
}
fn hsl(rgb: [f32; 3]) -> [f32; 3] {
    let max = rgb.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let min = rgb.into_iter().fold(f32::INFINITY, f32::min);
    let difference = max - min;
    let lightness = (max + min) * 0.5;
    if difference < 1e-8 {
        return [0., 0., lightness];
    }
    let hue = if max == rgb[0] {
        ((rgb[1] - rgb[2]) / difference).rem_euclid(6.)
    } else if max == rgb[1] {
        (rgb[2] - rgb[0]) / difference + 2.
    } else {
        (rgb[0] - rgb[1]) / difference + 4.
    } * 60.;
    [
        hue,
        difference / (1. - (2. * lightness - 1.).abs()).max(1e-8),
        lightness,
    ]
}
fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let hue = hue.rem_euclid(360.) / 60.;
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let second = chroma * (1. - (hue.rem_euclid(2.) - 1.).abs());
    let components = match hue as u32 {
        0 => [chroma, second, 0.],
        1 => [second, chroma, 0.],
        2 => [0., chroma, second],
        3 => [0., second, chroma],
        4 => [second, 0., chroma],
        _ => [chroma, 0., second],
    };
    components.map(|value| value + lightness - chroma * 0.5)
}
fn smoothstep(value: f32) -> f32 {
    value * value * (3. - 2. * value)
}
fn hash(x: i32, y: i32) -> f32 {
    let mut value = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    value = (value ^ (value >> 13)).wrapping_mul(1274126177);
    ((value ^ (value >> 16)) as f32 / u32::MAX as f32) * 2. - 1.
}
fn grain(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (smoothstep(x.fract()), smoothstep(y.fract()));
    let top = hash(ix, iy) * (1. - fx) + hash(ix + 1, iy) * fx;
    let bottom = hash(ix, iy + 1) * (1. - fx) + hash(ix + 1, iy + 1) * fx;
    top * (1. - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_tables_stay_below_sixteen_bit_rounding() {
        for i in 0..=1_000_000 {
            let value = i as f32 / 1_000_000.;
            let decoded = if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            };
            let encoded = if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.powf(1. / 2.4) - 0.055
            };
            assert!((linear(value) - decoded).abs() < 0.1 / 65535.);
            assert!((srgb(value) - encoded).abs() < 0.1 / 65535.);
        }
        for value in [-0.2, 1.1, 4., 16.] {
            assert!((linear(srgb(value)) - value).abs() < 1e-5);
        }
    }

    #[test]
    fn black_point_retains_tone_order_and_black_and_white_endpoints() {
        for amount in -100..=100 {
            let settings = Settings {
                blacks: amount as f32,
                ..Default::default()
            };
            for transform in [
                ColorTransform::new(&settings),
                ColorTransform::for_native_raw(&settings),
            ] {
                let mut previous = -1.;
                for i in 0..=4096 {
                    let input = i as f32 / 4096.;
                    let output = transform.apply([input; 3], &settings, 0.5, 0.5)[0];
                    assert!(
                        output + 1e-6 >= previous,
                        "inverted black ramp: {amount}, {input}"
                    );
                    previous = output;
                }
                assert_eq!(transform.apply([0.; 3], &settings, 0.5, 0.5), [0.; 3]);
                assert!((previous - 1.).abs() < 1e-6);
            }
        }
        assert_eq!(black_point(4., 100.), 4.);
    }

    #[test]
    fn extreme_tone_combinations_do_not_reverse_brightness() {
        for bits in 0..32 {
            let amount = |bit| if bits & (1 << bit) == 0 { -100. } else { 100. };
            let settings = Settings {
                contrast: amount(0),
                highlights: amount(1),
                shadows: amount(2),
                whites: amount(3),
                blacks: amount(4),
                ..Default::default()
            };
            for transform in [
                ColorTransform::new(&settings),
                ColorTransform::for_native_raw(&settings),
            ] {
                let mut previous = -1.;
                for i in 0..=16384 {
                    let input = i as f32 / 1024.;
                    let output = transform.apply([input; 3], &settings, 0.5, 0.5)[0];
                    assert!(
                        output.is_finite() && output + 1e-6 >= previous,
                        "inverted combined ramp: {bits}, {input}"
                    );
                    previous = output;
                }
            }
        }
    }

    #[test]
    fn native_raw_tone_preserves_neutrals_hue_and_recoverable_highlights() {
        let settings = Settings::default();
        let transform = ColorTransform::for_native_raw(&settings);
        let apply = |rgb| transform.apply(rgb, &settings, 0.5, 0.5).map(linear);
        let mut previous = -1.;
        for i in 0..=10000 {
            let value = i as f32 / 10000.;
            let rgb = apply([value; 3]);
            assert!(rgb.iter().all(|v| v.is_finite()));
            assert!(rgb[0] > previous, "tone flattened or reversed at {value}");
            assert!((rgb[0] - rgb[1]).abs() < 1e-6 && (rgb[1] - rgb[2]).abs() < 1e-6);
            previous = rgb[0];
        }
        assert!(apply([0.01; 3])[0] > 0.004);
        assert!(apply([0.18; 3])[0] > 0.25);
        for value in [1., 1.5, 4., 20.] {
            assert!((apply([value; 3])[0] - value).abs() < 1e-4);
        }
        for rgb in [
            [0.3, 0.15, 0.1],
            [0.1, 0.3, 0.15],
            [0.15, 0.1, 0.3],
            [-0.01, 0.2, 0.1],
        ] {
            let output = apply(rgb);
            let gain = output[1] / rgb[1];
            for channel in 0..3 {
                assert!((output[channel] - rgb[channel] * gain).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn straight_point_curves_do_not_change_highlight_colors() {
        let settings = Settings {
            saturation: -50.,
            ..Default::default()
        };
        let input = [2., 0.4, 0.1];
        let expected = ColorTransform::new(&settings).apply(input, &settings, 0.5, 0.5);
        let mut straight = settings.clone();
        straight.point_curves = std::array::from_fn(|_| vec![[0., 0.], [0.4, 0.4], [1., 1.]]);
        // Point curves take precedence over a previously saved five-point curve.
        straight.curves[0][2] = 0.8;
        let actual = ColorTransform::new(&straight).apply(input, &straight, 0.5, 0.5);
        assert_eq!(actual, expected);
    }

    #[test]
    fn channel_curves_match_lrc_srgb_swatch() {
        // LRC 15.6 / PV 15.4, embedded sRGB, all other adjustments zero, 16-bit TIFF.
        let input = [51., 102., 85.].map(|v| linear(v / 255.));
        for (channel, midpoint, expected) in [
            (1, 160., [0.429923, 0.379873, 0.332403]),
            (2, 96., [0.312016, 0.259113, 0.350149]),
            (3, 160., [0.101045, 0.399725, 0.458442]),
        ] {
            let mut settings = Settings::default();
            settings.point_curves[channel] =
                vec![[0., 0.], [128. / 255., midpoint / 255.], [1., 1.]];
            let actual = ColorTransform::new(&settings).apply(input, &settings, 0.5, 0.5);
            for (value, reference) in actual.into_iter().zip(expected) {
                assert!(
                    (value - reference).abs() < 0.002,
                    "channel {channel}: {actual:?}"
                );
            }
        }
    }
}
