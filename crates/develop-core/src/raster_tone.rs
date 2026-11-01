//! Display-referred WB and tone response, calibrated against LRC PV 15.4.
//! Sensor-linear RAW and local masks deliberately retain their scene-linear path.
use crate::{
    Settings,
    color::{linear, srgb},
};

const KNOTS: usize = 129;
const WB_STEPS: usize = 21;
const WB_WORDS: usize = WB_STEPS * WB_STEPS * 3 * KNOTS;
const EXPOSURE_WORDS: usize = 11 * KNOTS;
const TABLE_SIZE: usize = 4096;
const DATA: &[u8; (WB_WORDS + EXPOSURE_WORDS + 9 * KNOTS) * 2] =
    include_bytes!("raster_calibration.bin");

pub struct RasterTone {
    to_wide: [[f32; 3]; 3],
    from_wide: [[f32; 3]; 3],
    pub(crate) white_balance: Vec<[f32; 3]>,
    pub(crate) tone: Vec<f32>,
}

impl RasterTone {
    pub fn new(settings: &Settings) -> Option<Self> {
        if [
            settings.temperature,
            settings.tint,
            settings.exposure,
            settings.contrast,
        ]
        .iter()
        .all(|v| *v == 0.)
        {
            return None;
        }
        let srgb_profile = moxcms::ColorProfile::new_srgb();
        let wide_profile = moxcms::ColorProfile::new_pro_photo_rgb();
        let to_wide = srgb_profile
            .transform_matrix(&wide_profile)
            .v
            .map(|row| row.map(|v| v as f32));
        let from_wide = wide_profile
            .transform_matrix(&srgb_profile)
            .v
            .map(|row| row.map(|v| v as f32));
        let (temperature, temperature_mix) =
            interval((settings.temperature + 100.) / 10., WB_STEPS);
        let (tint, tint_mix) = interval((settings.tint + 100.) / 10., WB_STEPS);
        let has_wb = settings.temperature != 0. || settings.tint != 0.;
        let white_balance = (0..=TABLE_SIZE)
            .map(|i| {
                // Square-spaced linear input gives dark tones enough samples without per-pixel powf.
                let input = srgb((i as f32 / TABLE_SIZE as f32).powi(2));
                std::array::from_fn(|channel| {
                    if !has_wb {
                        return input;
                    }
                    let curve =
                        |t, g| sample_data(((t * WB_STEPS + g) * 3 + channel) * KNOTS, input);
                    let lower = lerp(
                        curve(temperature, tint),
                        curve(temperature + 1, tint),
                        temperature_mix,
                    );
                    let upper = lerp(
                        curve(temperature, tint + 1),
                        curve(temperature + 1, tint + 1),
                        temperature_mix,
                    );
                    lerp(lower, upper, tint_mix)
                })
            })
            .collect();
        let exposure = |input| parameter_curve(WB_WORDS, settings.exposure + 5., 11, input);
        let contrast_position = (settings.contrast + 100.) / 25.;
        let contrast =
            |input| parameter_curve(WB_WORDS + EXPOSURE_WORDS, contrast_position, 9, input);
        // LRC adapts contrast to image content. Keep the neutral pivot stable when using a fixed LUT.
        let pivot_correction = 0.5 - contrast(0.5);
        let tone = (0..=TABLE_SIZE)
            .map(|i| {
                let input = i as f32 / TABLE_SIZE as f32;
                let exposed = if settings.exposure == 0. {
                    input
                } else {
                    exposure(input)
                };
                if settings.contrast == 0. {
                    return exposed;
                }
                (contrast(exposed) + pivot_correction * 4. * exposed * (1. - exposed)).clamp(0., 1.)
            })
            .collect();
        Some(Self {
            to_wide,
            from_wide,
            white_balance,
            tone,
        })
    }

    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let wide = multiply(self.to_wide, rgb);
        let balanced: [f32; 3] = std::array::from_fn(|channel| {
            let value = wide[channel].max(0.);
            let position = value.min(1.).sqrt() * TABLE_SIZE as f32;
            let (index, mix) = interval(position, TABLE_SIZE + 1);
            lerp(
                self.white_balance[index][channel],
                self.white_balance[index + 1][channel],
                mix,
            ) + if value > 1. { srgb(value) - 1. } else { 0. }
        });
        // Interpolate the middle channel between the mapped extrema to preserve hue.
        let low = balanced.into_iter().fold(f32::INFINITY, f32::min);
        let high = balanced.into_iter().fold(f32::NEG_INFINITY, f32::max);
        let mapped_low = sample_table(&self.tone, low);
        let mapped_high = sample_table(&self.tone, high);
        let spread = high - low;
        let toned = balanced.map(|v| {
            let value = if spread > 1e-7 {
                mapped_low + (v - low) / spread * (mapped_high - mapped_low)
            } else {
                mapped_low
            };
            decode_display(value)
        });
        multiply(self.from_wide, toned)
    }
}

fn decode_display(value: f32) -> f32 {
    if !(0. ..=1.).contains(&value) {
        return linear(value);
    }
    static TRANSFER: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let table = TRANSFER.get_or_init(|| {
        (0..=TABLE_SIZE)
            .map(|i| linear(i as f32 / TABLE_SIZE as f32))
            .collect()
    });
    sample_table(table, value)
}

pub(crate) fn multiply(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2])
}
fn lerp(a: f32, b: f32, fraction: f32) -> f32 {
    a + (b - a) * fraction
}
fn interval(position: f32, count: usize) -> (usize, f32) {
    let position = position.clamp(0., (count - 1) as f32);
    let index = (position as usize).min(count - 2);
    (index, position - index as f32)
}
fn sample_data(offset: usize, value: f32) -> f32 {
    let (index, mix) = interval(value * (KNOTS - 1) as f32, KNOTS);
    let word = |i| {
        let start = (offset + i) * 2;
        u16::from_le_bytes([DATA[start], DATA[start + 1]]) as f32 / 65535.
    };
    lerp(word(index), word(index + 1), mix)
}
fn parameter_curve(offset: usize, position: f32, count: usize, value: f32) -> f32 {
    let (index, mix) = interval(position, count);
    lerp(
        sample_data(offset + index * KNOTS, value),
        sample_data(offset + (index + 1) * KNOTS, value),
        mix,
    )
}
fn sample_table(table: &[f32], value: f32) -> f32 {
    let (index, mix) = interval(value * TABLE_SIZE as f32, TABLE_SIZE + 1);
    lerp(table[index], table[index + 1], mix) + (value - 1.).max(0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_patches_follow_independent_lightroom_jpeg_exports() {
        // JPEG chart measurements are independent of the 16-bit PNG calibration ramp.
        for (temperature, tint, expected) in [
            (30., 0., [97., 82., 61.]),
            (-30., 0., [49., 70., 97.]),
            (0., 30., [70., 62., 73.]),
            (0., -30., [65., 73., 62.]),
        ] {
            let settings = Settings {
                temperature,
                tint,
                ..Default::default()
            };
            let transform = RasterTone::new(&settings).unwrap();
            let output = transform
                .apply([linear(64. / 255.); 3])
                .map(|v| srgb(v) * 255.);
            for channel in 0..3 {
                assert!(
                    (output[channel] - expected[channel]).abs() < 3.,
                    "WB ({temperature}, {tint}) channel {channel}: {output:?}"
                );
            }
        }
    }

    #[test]
    fn fractional_controls_are_continuous_and_limits_remain_finite() {
        assert!(RasterTone::new(&Settings::default()).is_none());
        for temperature in [-100., -30., 0., 30., 100.] {
            for tint in [-100., 0., 100.] {
                let settings = Settings {
                    temperature,
                    tint,
                    exposure: 0.7,
                    contrast: 35.,
                    ..Default::default()
                };
                let transform = RasterTone::new(&settings).unwrap();
                for rgb in [[0.; 3], [0.01, 0.3, 0.8], [1.; 3], [2., 1., 0.1]] {
                    assert!(transform.apply(rgb).iter().all(|v| v.is_finite()));
                }
            }
        }
        for temperature in [-30., 0., 30.] {
            let output = |delta: f32| {
                RasterTone::new(&Settings {
                    temperature: temperature + delta,
                    tint: 18.,
                    exposure: 0.7,
                    ..Default::default()
                })
                .unwrap()
                .apply([0.12, 0.23, 0.08])
            };
            let before = output(-0.001);
            let after = output(0.001);
            assert!(
                before
                    .iter()
                    .zip(after)
                    .all(|(a, b)| (*a - b).abs() < 0.0001)
            );
        }
    }

    #[test]
    fn tone_ramps_preserve_order_and_black_white() {
        for exposure in [-5., -1.3, 0., 0.7, 5.] {
            for contrast in [-100., -25., 35., 100.] {
                let transform = RasterTone::new(&Settings {
                    exposure,
                    contrast,
                    ..Default::default()
                })
                .unwrap();
                let mut previous = 0.;
                for i in 0..=1024 {
                    let output = srgb(transform.apply([linear(i as f32 / 1024.); 3])[0]);
                    assert!(
                        output + 0.0001 >= previous,
                        "EV {exposure}, contrast {contrast}, patch {i}"
                    );
                    previous = output;
                }
                assert!(transform.apply([0.; 3]).iter().all(|v| v.abs() < 0.0001));
                let white = transform.apply([1.; 3]);
                if exposure >= -1. {
                    assert!(white.iter().all(|v| (*v - 1.).abs() < 0.002));
                } else {
                    // Strong negative EV also lowers the white endpoint in the LRC reference.
                    assert!(white.iter().all(|v| *v > 0. && *v <= 1.));
                }
            }
        }
    }
}
