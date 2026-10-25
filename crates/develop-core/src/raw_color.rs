//! Scene-referred RAW processing following the DNG camera-profile model.
use crate::{
    Settings,
    camera_profile::{self, HsvTable, Matrix},
    color::{linear, srgb},
};
use advanced_explorer_media::develop_input::RawColor;

const XYZ_TO_WIDE: Matrix = [
    [1.3459433, -0.2556075, -0.0511118],
    [-0.5445989, 1.5081673, 0.0205351],
    [0., 0., 1.2118128],
];
const D50: [f32; 3] = [0.96422, 1., 0.82521];
const BRADFORD: Matrix = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

pub struct RawTransform {
    matrix: Matrix,
    to_srgb: Matrix,
    to_wide: Matrix,
    hue_sat: Option<HsvTable>,
    look: Option<HsvTable>,
    curve: Vec<f32>,
    contrast: Option<crate::raster_tone::RasterTone>,
}
impl RawTransform {
    pub fn new(raw: &RawColor, settings: &Settings) -> Self {
        let profile = camera_profile::installed(&raw.camera);
        let matrices = profile
            .as_ref()
            .map(|p| p.color)
            .unwrap_or([raw.xyz_to_camera; 2]);
        let temperatures = profile
            .as_ref()
            .map(|p| p.temperatures)
            .unwrap_or([2856., 6504.]);
        let calibration = matching_calibration(raw, profile.as_ref());
        let (as_shot_temperature, as_shot_tint) =
            as_shot(raw.neutral, matrices, calibration, temperatures);
        let kelvin = settings.raw_temperature.unwrap_or(as_shot_temperature);
        let tint = if settings.raw_temperature.is_some() {
            settings.raw_tint
        } else {
            as_shot_tint
        };
        let weight = weight(kelvin, temperatures);
        let calibration = blend(calibration, weight);
        let color = product(calibration, blend(matrices, weight));
        let white = white_xyz(kelvin, tint);
        let mut neutral = if settings.raw_temperature.is_none() {
            raw.neutral
        } else {
            mul(color, white)
        };
        let maximum = neutral.into_iter().fold(0., f32::max).max(1e-6);
        neutral = neutral.map(|v| (v / maximum).max(1e-6));
        let camera_to_xyz = if let Some(forward) = profile.as_ref().and_then(|p| p.forward) {
            let forward = blend(forward, weight);
            let inverse_calibration =
                inverse(calibration).unwrap_or([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
            let reference_neutral = mul(inverse_calibration, neutral);
            let balanced = std::array::from_fn(|i| {
                std::array::from_fn(|j| forward[i][j] / reference_neutral[j].max(1e-6))
            });
            product(balanced, inverse_calibration)
        } else {
            let inverse = inverse(color).unwrap_or([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
            let source_white = mul(inverse, neutral);
            product(adapt(source_white, D50), inverse)
        };
        let exposure = 2f32.powf(
            settings.exposure
                + raw.baseline_exposure
                + profile.as_ref().map(|p| p.exposure_offset).unwrap_or(0.),
        );
        let mut matrix = product(XYZ_TO_WIDE, camera_to_xyz).map(|row| row.map(|v| v * exposure));
        // Catalogs written before Kelvin controls retain their relative WB adjustments.
        if settings.raw_temperature.is_none() {
            let gains = [
                2f32.powf(settings.temperature * 0.004),
                2f32.powf(-settings.tint * 0.004),
                2f32.powf(-settings.temperature * 0.004),
            ];
            for row in &mut matrix {
                for i in 0..3 {
                    row[i] *= gains[i];
                }
            }
        }
        let hue_sat = profile
            .as_ref()
            .and_then(|p| match (&p.hue_sat[0], &p.hue_sat[1]) {
                (Some(a), Some(b))
                    if a.dimensions == b.dimensions && a.srgb_encoding == b.srgb_encoding =>
                {
                    let mut table = a.clone();
                    for (x, y) in table.values.iter_mut().zip(&b.values) {
                        for i in 0..3 {
                            x[i] = x[i] * weight + y[i] * (1. - weight);
                        }
                    }
                    Some(table)
                }
                (Some(a), _) => Some(a.clone()),
                (_, Some(b)) => Some(b.clone()),
                _ => None,
            });
        let curve = (0..=4096)
            .map(|i| {
                let x = i as f32 / 4096.;
                if let Some(p) = &profile
                    && p.curve.len() >= 2
                {
                    return crate::curve::sample(x, &p.curve);
                }
                default_tone(x)
            })
            .collect();
        let to_srgb = moxcms::ColorProfile::new_pro_photo_rgb()
            .transform_matrix(&moxcms::ColorProfile::new_srgb())
            .v
            .map(|r| r.map(|v| v as f32));
        let contrast = crate::raster_tone::RasterTone::new(&Settings {
            contrast: settings.contrast,
            ..Default::default()
        });
        let to_wide = inverse(to_srgb).unwrap();
        Self {
            matrix,
            to_srgb,
            to_wide,
            hue_sat,
            look: profile.and_then(|p| p.look),
            curve,
            contrast,
        }
    }
    pub fn input(&self, camera: [f32; 3]) -> [f32; 3] {
        mul(self.to_srgb, mul(self.matrix, camera))
    }
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut wide = mul(self.to_wide, rgb).map(|v| v.max(0.));
        if let Some(table) = &self.hue_sat {
            wide = table.apply(wide);
        }
        if let Some(table) = &self.look {
            wide = table.apply(wide);
        }
        let low = wide.into_iter().fold(f32::INFINITY, f32::min);
        let high = wide.into_iter().fold(0., f32::max);
        let a = sample(&self.curve, low);
        let b = sample(&self.curve, high);
        let mapped = wide.map(|v| {
            if high - low > 1e-7 {
                a + (v - low) / (high - low) * (b - a)
            } else {
                a
            }
        });
        let rgb = mul(self.to_srgb, mapped);
        self.contrast.as_ref().map(|c| c.apply(rgb)).unwrap_or(rgb)
    }
}
pub(crate) fn default_tone(x: f32) -> f32 {
    // Measured neutral linear-DNG response, independent of the validation photographs.
    const DATA: &[u8; 4097 * 4] = include_bytes!("raw_tone.bin");
    let p = x.clamp(0., 1.) * 4096.;
    let i = (p as usize).min(4095);
    let word = |n: usize| f32::from_le_bytes(DATA[n * 4..n * 4 + 4].try_into().unwrap());
    word(i) + (word(i + 1) - word(i)) * (p - i as f32)
}
fn sample(table: &[f32], x: f32) -> f32 {
    let p = x.clamp(0., 1.) * (table.len() - 1) as f32;
    let i = (p as usize).min(table.len() - 2);
    // Keep highlight headroom available to later recovery controls instead of flattening it at white.
    table[i] + (table[i + 1] - table[i]) * (p - i as f32) + (x - 1.).max(0.)
}

impl HsvTable {
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max = rgb.into_iter().fold(0., f32::max);
        let min = rgb.into_iter().fold(f32::INFINITY, f32::min);
        if max <= 1e-10 {
            return rgb;
        }
        let delta = max - min;
        let saturation = delta / max;
        let hue = if delta <= 1e-10 {
            0.
        } else if max == rgb[0] {
            ((rgb[1] - rgb[2]) / delta).rem_euclid(6.)
        } else if max == rgb[1] {
            (rgb[2] - rgb[0]) / delta + 2.
        } else {
            (rgb[0] - rgb[1]) / delta + 4.
        };
        let value = if self.srgb_encoding { srgb(max) } else { max };
        let [h, s, v] = self.dimensions;
        let pos = [
            hue * h as f32 / 6.,
            saturation * (s - 1) as f32,
            value.min(1.) * (v - 1) as f32,
        ];
        let index = [
            pos[0] as usize % h,
            (pos[1] as usize).min(s - 2),
            (pos[2] as usize).min(v.saturating_sub(2)),
        ];
        let fraction = std::array::from_fn::<_, 3, _>(|i| pos[i] - index[i] as f32);
        let mut correction = [0.; 3];
        for dh in 0..2 {
            for ds in 0..2 {
                for dv in 0..2 {
                    let weight = (if dh == 0 {
                        1. - fraction[0]
                    } else {
                        fraction[0]
                    }) * (if ds == 0 {
                        1. - fraction[1]
                    } else {
                        fraction[1]
                    }) * (if dv == 0 {
                        1. - fraction[2]
                    } else {
                        fraction[2]
                    });
                    let entry = self.values[((index[2] + dv).min(v - 1) * h + (index[0] + dh) % h)
                        * s
                        + index[1]
                        + ds];
                    for i in 0..3 {
                        correction[i] += entry[i] * weight;
                    }
                }
            }
        }
        let hue = (hue + correction[0] / 60.).rem_euclid(6.);
        let sat = (saturation * correction[1]).clamp(0., 1.);
        // Keep scene headroom until exposure and the output tone curve have been applied.
        let value = value * correction[2];
        let value = if self.srgb_encoding {
            linear(value)
        } else {
            value
        };
        let chroma = value * sat;
        let x = chroma * (1. - (hue % 2. - 1.).abs());
        let m = value - chroma;
        let rgb = match hue as usize {
            0 => [chroma, x, 0.],
            1 => [x, chroma, 0.],
            2 => [0., chroma, x],
            3 => [0., x, chroma],
            4 => [x, 0., chroma],
            _ => [chroma, 0., x],
        };
        rgb.map(|v| v + m)
    }
}

fn weight(k: f32, t: [f32; 2]) -> f32 {
    let span = 1. / t[0] - 1. / t[1];
    if span.abs() < 1e-9 {
        1.
    } else {
        ((1. / k - 1. / t[1]) / span).clamp(0., 1.)
    }
}
fn blend(m: [Matrix; 2], w: f32) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| m[0][i][j] * w + m[1][i][j] * (1. - w)))
}
fn mul(m: Matrix, v: [f32; 3]) -> [f32; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}
fn product(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
fn inverse(m: Matrix) -> Option<Matrix> {
    let [a, b, c] = m;
    let cof = [
        [
            b[1] * c[2] - b[2] * c[1],
            a[2] * c[1] - a[1] * c[2],
            a[1] * b[2] - a[2] * b[1],
        ],
        [
            b[2] * c[0] - b[0] * c[2],
            a[0] * c[2] - a[2] * c[0],
            a[2] * b[0] - a[0] * b[2],
        ],
        [
            b[0] * c[1] - b[1] * c[0],
            a[1] * c[0] - a[0] * c[1],
            a[0] * b[1] - a[1] * b[0],
        ],
    ];
    let det = a[0] * cof[0][0] + a[1] * cof[1][0] + a[2] * cof[2][0];
    (det.abs() > 1e-10).then(|| cof.map(|r| r.map(|v| v / det)))
}
fn adapt(from: [f32; 3], to: [f32; 3]) -> Matrix {
    let a = mul(BRADFORD, from);
    let b = mul(BRADFORD, to);
    let scaled = std::array::from_fn(|i| BRADFORD[i].map(|v| v * b[i] / a[i].max(1e-6)));
    product(inverse(BRADFORD).unwrap(), scaled)
}
// Robertson (1968) CIE 1960 isotemperature lines, Wyszecki & Stiles table 1(3.11).
// Reciprocal kelvin interpolation covers the entire 2000–60000 K editing range.
const ISOTEMPERATURE_LINES: [[f32; 4]; 31] = [
    [0., 0.18006, 0.26352, -0.24341],
    [10., 0.18066, 0.26589, -0.25479],
    [20., 0.18133, 0.26846, -0.26876],
    [30., 0.18208, 0.27119, -0.28539],
    [40., 0.18293, 0.27407, -0.30470],
    [50., 0.18388, 0.27709, -0.32675],
    [60., 0.18494, 0.28021, -0.35156],
    [70., 0.18611, 0.28342, -0.37915],
    [80., 0.18740, 0.28668, -0.40955],
    [90., 0.18880, 0.28997, -0.44278],
    [100., 0.19032, 0.29326, -0.47888],
    [125., 0.19462, 0.30141, -0.58204],
    [150., 0.19962, 0.30921, -0.70471],
    [175., 0.20525, 0.31647, -0.84901],
    [200., 0.21142, 0.32312, -1.0182],
    [225., 0.21807, 0.32909, -1.2168],
    [250., 0.22511, 0.33439, -1.4512],
    [275., 0.23247, 0.33904, -1.7298],
    [300., 0.24010, 0.34308, -2.0637],
    [325., 0.24792, 0.34655, -2.4681],
    [350., 0.25591, 0.34951, -2.9641],
    [375., 0.26400, 0.35200, -3.5814],
    [400., 0.27218, 0.35407, -4.3633],
    [425., 0.28039, 0.35577, -5.3762],
    [450., 0.28863, 0.35714, -6.7262],
    [475., 0.29685, 0.35823, -8.5955],
    [500., 0.30505, 0.35907, -11.324],
    [525., 0.31320, 0.35968, -15.628],
    [550., 0.32129, 0.36011, -23.325],
    [575., 0.32931, 0.36038, -40.770],
    [600., 0.33724, 0.36051, -116.45],
];
fn temperature_line(k: f32) -> ([f32; 2], [f32; 2]) {
    let reciprocal = 1e6 / k.clamp(2000., 60000.);
    let index = ISOTEMPERATURE_LINES
        .partition_point(|row| row[0] < reciprocal)
        .clamp(1, ISOTEMPERATURE_LINES.len() - 1);
    let a = ISOTEMPERATURE_LINES[index - 1];
    let b = ISOTEMPERATURE_LINES[index];
    let w = (reciprocal - a[0]) / (b[0] - a[0]);
    let uv = [a[1] + w * (b[1] - a[1]), a[2] + w * (b[2] - a[2])];
    let direction = |slope: f32| {
        let length = 1f32.hypot(slope);
        [-1. / length, -slope / length]
    };
    let na = direction(a[3]);
    let nb = direction(b[3]);
    let normal = [na[0] + w * (nb[0] - na[0]), na[1] + w * (nb[1] - na[1])];
    let length = normal[0].hypot(normal[1]);
    (uv, normal.map(|v| v / length))
}
fn white_xyz(k: f32, tint: f32) -> [f32; 3] {
    let (uv, n) = temperature_line(k);
    let u = uv[0] + n[0] * tint / 3000.;
    let v = uv[1] + n[1] * tint / 3000.;
    [1.5 * u / v, 1., (2. - 0.5 * u - 5. * v) / v]
}
pub(crate) fn raster_white_balance(kelvin: f32, tint: f32, reference_kelvin: f32) -> Matrix {
    let to_xyz = moxcms::ColorProfile::new_srgb()
        .colorant_matrix()
        .v
        .map(|row| row.map(|v| v as f32));
    // Baked images use an explicit reference white, never an invented camera temperature.
    let reference_to_d50 = adapt(white_xyz(reference_kelvin, 0.), D50);
    let source_white = mul(reference_to_d50, white_xyz(kelvin, tint));
    product(
        inverse(to_xyz).unwrap(),
        product(adapt(source_white, D50), to_xyz),
    )
}
fn temperature_tint(xyz: [f32; 3]) -> (f32, f32) {
    let d = xyz[0] + 15. * xyz[1] + 3. * xyz[2];
    let uv = [4. * xyz[0] / d, 6. * xyz[1] / d];
    let (mut lo, mut hi) = (2000., 60000.);
    for _ in 0..32 {
        let mid = (lo + hi) / 2.;
        let (p, n) = temperature_line(mid);
        let cross = (uv[0] - p[0]) * n[1] - (uv[1] - p[1]) * n[0];
        if cross > 0. {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let t = (lo + hi) / 2.;
    let (p, n) = temperature_line(t);
    (t, 3000. * ((uv[0] - p[0]) * n[0] + (uv[1] - p[1]) * n[1]))
}
fn as_shot(
    neutral: [f32; 3],
    matrices: [Matrix; 2],
    calibration: [Matrix; 2],
    temps: [f32; 2],
) -> (f32, f32) {
    let mut k = 5000.;
    let mut tint = 0.;
    for _ in 0..12 {
        let w = weight(k, temps);
        if let Some(inv) = inverse(product(blend(calibration, w), blend(matrices, w))) {
            (k, tint) = temperature_tint(mul(inv, neutral));
        }
    }
    (k, tint)
}

pub fn describe(raw: &mut RawColor) -> String {
    let profile = camera_profile::installed(&raw.camera);
    let matrices = profile
        .as_ref()
        .map(|p| p.color)
        .unwrap_or([raw.xyz_to_camera; 2]);
    let temperatures = profile
        .as_ref()
        .map(|p| p.temperatures)
        .unwrap_or([2856., 6504.]);
    let calibration = matching_calibration(raw, profile.as_ref());
    let (kelvin, tint) = as_shot(raw.neutral, matrices, calibration, temperatures);
    raw.as_shot = [kelvin, tint];
    raw.profile_signature = camera_profile::signature(&raw.camera);
    format!(
        "{} / {}",
        raw.camera,
        profile
            .map(|p| p.name)
            .unwrap_or_else(|| "カメラ行列（DCPなし）".into())
    )
}

fn matching_calibration(
    raw: &RawColor,
    profile: Option<&camera_profile::CameraProfile>,
) -> [Matrix; 2] {
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let calibration =
        if profile.is_some_and(|p| p.calibration_signature == raw.calibration_signature) {
            raw.calibration
        } else {
            [identity; 2]
        };
    calibration.map(|matrix| std::array::from_fn(|i| matrix[i].map(|v| v * raw.analog_balance[i])))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_camera() -> RawColor {
        RawColor {
            camera: "OpenRoom test XYZ sensor".into(),
            neutral: D50,
            xyz_to_camera: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            baseline_exposure: 0.,
            calibration: [[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]; 2],
            calibration_signature: String::new(),
            analog_balance: [1.; 3],
            as_shot: [0., 0.],
            profile_signature: String::new(),
        }
    }
    #[test]
    fn kelvin_and_tint_match_d50_and_roundtrip() {
        let (kelvin, tint) = temperature_tint(D50);
        assert!((kelvin - 5003.).abs() < 10., "{kelvin}/{tint}");
        assert!((tint - 10.).abs() < 1., "{kelvin}/{tint}");
        let (uv, _) = temperature_line(5000.);
        assert_eq!(uv, [0.21142, 0.32312]);
        for k in [2500., 3500., 5000., 7500., 15000., 25000., 50000., 60000.] {
            for tint in [-30., 0., 30.] {
                let (actual_k, actual_tint) = temperature_tint(white_xyz(k, tint));
                assert!(
                    (actual_k - k).abs() / k < 0.002,
                    "{k}/{tint}: {actual_k}/{actual_tint}"
                );
                assert!((actual_tint - tint).abs() < 0.1);
            }
        }
    }
    #[test]
    fn camera_conversion_keeps_highlights_until_exposure_and_tone() {
        let raw = synthetic_camera();
        let neutral = RawTransform::new(&raw, &Settings::default());
        let bright = D50.map(|v| v * 2.);
        assert!(neutral.input(bright).iter().all(|v| *v > 1.9));
        let reduced = RawTransform::new(
            &raw,
            &Settings {
                exposure: -2.,
                ..Default::default()
            },
        );
        let output = reduced.apply(reduced.input(bright));
        for value in output {
            assert!((value - default_tone(0.5)).abs() < 0.001);
        }
        for k in [2000., 60000.] {
            for tint in [-150., 150.] {
                let transform = RawTransform::new(
                    &raw,
                    &Settings {
                        raw_temperature: Some(k),
                        raw_tint: tint,
                        exposure: 5.,
                        ..Default::default()
                    },
                );
                assert!(
                    transform
                        .apply(transform.input([0.2, 0.4, 0.1]))
                        .iter()
                        .all(|v| v.is_finite())
                );
            }
        }
    }
    #[test]
    fn hsv_interpolation_preserves_neutrals_wraps_hue_and_retains_headroom() {
        let identity = HsvTable {
            dimensions: [2, 2, 1],
            values: vec![[0., 1., 1.]; 4],
            srgb_encoding: false,
        };
        for rgb in [[0.; 3], [0.2; 3], [0.6, 0.1, 0.4], [3., 0.1, 0.2]] {
            let actual = identity.apply(rgb);
            assert!(actual.iter().zip(rgb).all(|(a, b)| (*a - b).abs() < 1e-5));
        }
        let shift = HsvTable {
            values: vec![[360., 1., 1.]; 4],
            ..identity
        };
        let actual = shift.apply([0.8, 0.1, 0.2]);
        assert!(
            actual
                .iter()
                .zip([0.8, 0.1, 0.2])
                .all(|(a, b)| (*a - b).abs() < 1e-5)
        );
    }
    #[test]
    fn measured_raw_curve_is_monotonic_and_not_the_raster_transfer() {
        let mut previous = 0.;
        for i in 0..=4096 {
            let value = default_tone(i as f32 / 4096.);
            assert!(value >= previous);
            previous = value;
        }
        assert_eq!(default_tone(0.), 0.);
        assert_eq!(default_tone(1.), 1.);
        assert!((default_tone(0.1) - 0.186).abs() < 0.002);
        assert!((default_tone(0.5) - 0.804).abs() < 0.002);
    }
}
