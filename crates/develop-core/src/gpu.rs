//! Native GPU compute accelerates the existing pointwise color transform; unsupported inputs stay on the CPU.
use super::*;
use image::Rgba32FImage;
use std::sync::OnceLock;

#[repr(C, align(16))]
struct Parameters {
    dimensions_flags: [u32; 4],
    gains: [f32; 4],
    saturation: [f32; 4],
    vignette: [f32; 4],
    grain: [f32; 4],
    grading_control: [f32; 4],
    white_balance: [[f32; 4]; 3],
    to_wide: [[f32; 4]; 3],
    from_wide: [[f32; 4]; 3],
    grading: [[f32; 4]; 4],
    mixer: [[f32; 4]; 8],
}
const _: () = assert!(std::mem::size_of::<Parameters>() == 27 * 16);
unsafe extern "C" {
    #[cfg_attr(target_os = "macos", link_name = "openroom_metal_color")]
    #[cfg_attr(target_os = "windows", link_name = "openroom_d3d_color")]
    fn openroom_gpu_color(
        pixels: *mut f32,
        pixel_bytes: usize,
        parameters: *const Parameters,
        parameter_bytes: usize,
        tone: *const f32,
        tone_bytes: usize,
        curves: *const f32,
        curve_bytes: usize,
        transfer: *const f32,
        transfer_bytes: usize,
        source: *const u8,
        source_bytes: usize,
    ) -> bool;
}
fn padded(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.]
}

impl ColorTransform {
    pub(crate) fn apply_gpu(&self, pixels: &mut Rgba32FImage, settings: &Settings) -> bool {
        // Small drag previews are already faster on CPU than a GPU submission and readback.
        // Legacy sensor calibration remains on CPU until it has a verified GPU implementation.
        static DISABLED: OnceLock<bool> = OnceLock::new();
        if pixels.len() / 4 < 512 * 512
            || self.raw.is_some()
            || *DISABLED.get_or_init(|| std::env::var("OPENROOM_DISABLE_GPU").as_deref() == Ok("1"))
        {
            return false;
        }
        let flags = u32::from(self.has_tone)
            | (u32::from(self.raster_wb.is_some()) << 1)
            | (u32::from(settings.monochrome) << 2)
            | self
                .has_curves
                .iter()
                .enumerate()
                .fold(0, |bits, (i, active)| {
                    bits | (u32::from(*active) << (4 + i))
                })
            | (u32::from(self.has_mixer) << 8)
            | (u32::from(self.has_grading) << 9)
            | (u32::from(self.raster.is_some()) << 10);
        let params = Parameters {
            dimensions_flags: [pixels.width(), pixels.height(), flags, 0],
            gains: padded(self.gains),
            saturation: [
                self.camera_saturation,
                settings.saturation,
                settings.vibrance,
                0.,
            ],
            vignette: [
                settings.vignette,
                settings.vignette_midpoint,
                settings.vignette_feather,
                0.,
            ],
            grain: [
                settings.grain,
                settings.grain_size,
                settings.grain_roughness,
                0.,
            ],
            grading_control: [settings.grading_balance, settings.grading_blending, 0., 0.],
            white_balance: self.raster_wb.unwrap_or_default().map(padded),
            to_wide: self.curve_to_wide.map(padded),
            from_wide: self.curve_from_wide.map(padded),
            grading: settings.grading.map(|[hue, saturation, brightness]| {
                padded(
                    from_hsl(hue, 1., 0.5)
                        .map(|v| (v - 0.5) * saturation / 100. * 0.3 + brightness / 100. * 0.2),
                )
            }),
            mixer: std::array::from_fn(|i| {
                let [h, s, l] = settings.mixer[i];
                [h, s, l, [0., 30., 60., 120., 180., 240., 270., 300.][i]]
            }),
        };
        let mut curves: Vec<f32> = (0..4)
            .flat_map(|channel| {
                (0..=4096).map(move |i| self.map_curve(i as f32 / 4096., channel, settings))
            })
            .collect();
        if let Some(raster) = &self.raster {
            // Preserve the calibrated JPEG response: reuse its exact CPU samples in ProPhoto RGB.
            for channel in 0..3 {
                curves.extend(raster.white_balance.iter().map(|rgb| rgb[channel]));
            }
            curves.extend_from_slice(&raster.tone);
        }
        // Reuse the CPU transfer samples, including their interpolation and extended-range behavior.
        static TRANSFER: OnceLock<Vec<f32>> = OnceLock::new();
        let transfer = TRANSFER.get_or_init(|| {
            let samples = (0..=4096)
                .map(|i| linear(i as f32 / 4096.))
                .chain((0..=65536).map(|i| srgb(i as f32 / 65536.)))
                .collect::<Vec<_>>();
            #[cfg(target_os = "windows")]
            {
                // D3D's approximate pow loses color precision when HDR channels nearly cancel.
                // Sample the CPU transfer above white as well; keep the Metal table unchanged.
                let mut samples = samples;
                samples.extend((0..=65536).map(|i| linear(1. + i as f32 * (63. / 65536.))));
                samples.extend((0..=65536).map(|i| srgb(1. + (i as f32 / 1024.).powi(2))));
                samples
            }
            #[cfg(target_os = "macos")]
            samples
        });
        let tone = if self.has_tone {
            self.tone.as_slice()
        } else {
            &[0.]
        };
        #[cfg(target_os = "macos")]
        let shader = include_str!("../native/color.metal");
        #[cfg(target_os = "windows")]
        let shader = include_str!("../native/color.hlsl");
        // SAFETY: the bridge is synchronous, validates allocation sizes, and commits only a completed
        // command. The mutable pixel slice is exclusive; every other pointer lives through the call.
        unsafe {
            openroom_gpu_color(
                pixels.as_mut_ptr(),
                pixels.len() * 4,
                &params,
                std::mem::size_of::<Parameters>(),
                tone.as_ptr(),
                std::mem::size_of_val(tone),
                curves.as_ptr(),
                curves.len() * 4,
                transfer.as_ptr(),
                transfer.len() * 4,
                shader.as_ptr(),
                shader.len(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use rayon::prelude::*;
    use std::time::Instant;

    #[test]
    fn small_drag_preview_keeps_input_for_cpu_fallback() {
        let settings = Settings {
            exposure: 2.,
            ..Default::default()
        };
        let mut pixels = Rgba32FImage::from_pixel(384, 256, Rgba([0.2, 0.4, 0.6, 1.]));
        let original = pixels.clone();
        assert!(!ColorTransform::new(&settings).apply_gpu(&mut pixels, &settings));
        assert_eq!(pixels, original);
    }

    #[test]
    fn explicit_gpu_disable_preserves_input_for_cpu_fallback() {
        if std::env::var("OPENROOM_DISABLE_GPU").as_deref() != Ok("1") {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "color::gpu::tests::explicit_gpu_disable_preserves_input_for_cpu_fallback",
                ])
                .env("OPENROOM_DISABLE_GPU", "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let settings = Settings {
            exposure: 2.,
            ..Default::default()
        };
        let mut pixels = Rgba32FImage::from_pixel(1021, 683, Rgba([0.2, 0.4, 0.6, 0.5]));
        let original = pixels.clone();
        assert!(!ColorTransform::new(&settings).apply_gpu(&mut pixels, &settings));
        assert_eq!(pixels, original);
    }

    #[test]
    #[ignore = "requires a real GPU and a full-resolution allocation"]
    fn gpu_full_resolution_dispatch_covers_rows_and_releases_large_buffers() {
        let settings = Settings {
            exposure: 0.7,
            saturation: 15.,
            vignette: -20.,
            ..Default::default()
        };
        let transform = ColorTransform::new(&settings);
        // Cross the D3D11 limit of 65535 groups in one dispatch dimension.
        let (width, height) = (4103, 4093);
        let source = |x: u32, y: u32| {
            Rgba([
                x as f32 / width as f32,
                y as f32 / height as f32,
                0.18,
                (x % 4) as f32 / 3.,
            ])
        };
        let mut pixels = Rgba32FImage::from_fn(width, height, source);
        assert!(transform.apply_gpu(&mut pixels, &settings));
        let count = width * height;
        for i in (0..count)
            .step_by(104729)
            .chain([65535 * 256 - 1, 65535 * 256, count - 1])
        {
            let (x, y) = (i % width, i / width);
            let input = source(x, y);
            let expected = transform.apply(
                [input[0], input[1], input[2]],
                &settings,
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let actual = pixels.get_pixel(x, y);
            for c in 0..3 {
                assert!((actual[c] - expected[c]).abs() <= 0.0002 * expected[c].abs().max(1.));
            }
            assert_eq!(actual[3], input[3]);
        }
        drop(pixels);
        // A small frame after the transient large allocation must still use the GPU.
        let mut preview = Rgba32FImage::from_pixel(1021, 683, source(0, 0));
        assert!(transform.apply_gpu(&mut preview, &settings));
    }

    #[test]
    #[ignore = "requires a real GPU device; compares the GPU with the authoritative CPU path"]
    fn gpu_colors_match_cpu_and_measure_transfer_inclusive_cost() {
        let input = Rgba32FImage::from_fn(1021, 683, |x, y| {
            let n = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
            let n = n.wrapping_mul(1274126177);
            Rgba([
                f32::from(n as u8) / 255. * 2. - 0.1,
                f32::from((n >> 8) as u8) / 255.,
                f32::from((n >> 16) as u8) / 255. * 1.5,
                (x % 4) as f32 / 3.,
            ])
        });
        let mut cases = vec![Settings::default()];
        for exposure in [-5., 2., 5.] {
            cases.push(Settings {
                exposure,
                contrast: 35.,
                highlights: -70.,
                shadows: 45.,
                whites: 20.,
                blacks: 25.,
                dehaze: 15.,
                saturation: 60.,
                vibrance: 45.,
                ..Default::default()
            });
        }
        let mut complex = Settings {
            temperature: 25.,
            tint: -15.,
            monochrome: true,
            vignette: -65.,
            grain: 40.,
            ..Default::default()
        };
        complex.curves[0] = [0.02, 0.2, 0.55, 0.8, 0.98];
        complex.point_curves[1] = vec![[0., 0.], [0.35, 0.2], [0.7, 0.8], [1., 1.]];
        complex.mixer[0] = [25., -35., 20.];
        complex.mixer[5] = [-40., 30., -20.];
        complex.grading = [
            [30., 35., -15.],
            [180., 20., 10.],
            [275., 50., 20.],
            [90., 5., 2.],
        ];
        cases.push(complex.clone());
        complex.monochrome = false;
        complex.raster_temperature = Some(4300.);
        complex.raster_tint = 20.;
        cases.push(complex);
        for (case, settings) in cases.into_iter().enumerate() {
            for mode in ["linear", "native_raw", "raster"] {
                let transform = match mode {
                    "native_raw" => ColorTransform::for_native_raw(&settings),
                    "raster" => ColorTransform::for_raster(&settings),
                    _ => ColorTransform::new(&settings),
                }
                .with_camera_saturation(1.15);
                let mut gpu = input.clone();
                assert!(
                    transform.apply_gpu(&mut gpu, &settings),
                    "GPU must run, not silently fall back"
                );
                let mut timed_input = input.clone();
                let started = Instant::now();
                assert!(transform.apply_gpu(&mut timed_input, &settings));
                let gpu_ms = started.elapsed().as_secs_f64() * 1000.;
                let mut cpu = input.clone();
                let started = Instant::now();
                cpu.as_mut()
                    .par_chunks_mut(4)
                    .enumerate()
                    .for_each(|(i, pixel)| {
                        let rgb = transform.apply(
                            [pixel[0], pixel[1], pixel[2]],
                            &settings,
                            ((i % 1021) as f32 + 0.5) / 1021.,
                            ((i / 1021) as f32 + 0.5) / 683.,
                        );
                        pixel[..3].copy_from_slice(&rgb);
                    });
                let cpu_ms = started.elapsed().as_secs_f64() * 1000.;
                let mut max_error = 0f32;
                for (i, (actual, expected)) in gpu.as_raw().iter().zip(cpu.as_raw()).enumerate() {
                    if i % 4 == 3 {
                        assert_eq!(actual, expected, "alpha changed");
                    }
                    let error = (actual - expected).abs();
                    max_error = max_error.max(error);
                    assert!(
                        actual.is_finite() && error <= 0.0002 * expected.abs().max(1.),
                        "case {case}, mode {mode}, sample {i}: GPU {actual}, CPU {expected}"
                    );
                }
                let gpu_bytes = crate::preview_pixels(&gpu);
                let cpu_bytes = crate::preview_pixels(&cpu);
                assert!(
                    gpu_bytes
                        .as_raw()
                        .iter()
                        .zip(cpu_bytes.as_raw())
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "8-bit preview differs by more than one level"
                );
                println!(
                    "case={case} mode={mode} cpu_ms={cpu_ms:.2} gpu_ms={gpu_ms:.2} max_float_error={max_error}"
                );
            }
        }
    }
}
