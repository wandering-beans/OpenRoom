use advanced_explorer_develop::{self as engine, Settings};
use image::{DynamicImage, ImageEncoder, Rgba, Rgba32FImage};

#[cfg(all(target_os = "macos", feature = "rtengine"))]
#[test]
fn native_raster_retains_display_tone_and_applies_adjustments_once() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("display.png");
    image::RgbaImage::from_fn(96, 64, |x, y| {
        Rgba([(x * 2 + 30) as u8, (y * 3 + 20) as u8, 90, (x * 2) as u8])
    })
    .save(&source)
    .unwrap();
    let native = engine::prepare(&source, 0).unwrap();
    let mut reference = native.clone();
    reference.info.native_source = None;
    let detail = advanced_explorer_rtengine::Adjustments {
        sharpen: 40.,
        noise_luma: 15.,
        ..Default::default()
    };
    let mut session =
        advanced_explorer_rtengine::Session::open(&source, Some(&native.pixels), 0).unwrap();
    let mut detailed = session.render(&detail).unwrap().pixels;
    for (pixel, input) in detailed.pixels_mut().zip(native.pixels.pixels()) {
        pixel[3] = input[3];
    }
    drop(session);
    for with_detail in [false, true] {
        reference.pixels = if with_detail {
            detailed.clone()
        } else {
            native.pixels.clone()
        };
        for kind in ["raster-display-linear", "raster-linear"] {
            let mut native = native.clone();
            native.info.source_kind = kind.into();
            reference.info.source_kind = kind.into();
            for recipe in [
                "{}",
                r#"{"saturation":300}"#,
                r#"{"exposure":1,"contrast":35,"temperature":30,"tint":-20}"#,
                r#"{"exposure":-2,"contrast":-50,"highlights":-60,"shadows":40}"#,
                r#"{"exposure":0.5,"raster_temperature":8000,"raster_reference_temperature":5200,"raster_tint":15,"temperature":30,"tint":20,"crop":[0.1,0.1,0.8,0.8],"quarter_turns":1}"#,
            ] {
                let mut settings: Settings = serde_json::from_str(recipe).unwrap();
                let expected = engine::render(&reference, &settings, &|| false)
                    .unwrap()
                    .image;
                if with_detail {
                    settings.sharpen = detail.sharpen;
                    settings.noise_luma = detail.noise_luma;
                }
                let actual = engine::render(&native, &settings, &|| false).unwrap().image;
                assert_eq!(actual.dimensions(), expected.dimensions());
                assert!(
                    actual
                        .as_raw()
                        .iter()
                        .zip(expected.as_raw())
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "{kind}: {recipe}"
                );
                let output = directory.path().join("export.png");
                engine::export(&native, &settings, &output, "png").unwrap();
                let exported = image::open(&output).unwrap().into_rgba16();
                assert!(
                    exported
                        .as_raw()
                        .iter()
                        .zip(actual.as_raw())
                        .all(|(a, b)| a.abs_diff(u16::from(*b) * 257) <= 129)
                );
                std::fs::remove_file(output).unwrap();
            }
        }
    }
}

#[test]
fn native_raw_initial_tone_is_shared_by_preview_and_export_only_once() {
    let mut base = engine::WorkingImage {
        pixels: Rgba32FImage::from_pixel(40, 30, Rgba([0.18, 0.18, 0.18, 0.5])),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 40,
            source_height: 30,
            width: 40,
            height: 30,
            source_kind: "rtengine-raw-linear".into(),
            color_note: String::new(),
        },
    };
    let settings = Settings::default();
    let preview = engine::render(&base, &settings, &|| false).unwrap().image;
    let pixel = preview.get_pixel(20, 15);
    assert!((140..=146).contains(&pixel[0]));
    assert_eq!([pixel[0]; 3], [pixel[0], pixel[1], pixel[2]]);
    assert_eq!(pixel[3], 128);
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("raw.png");
    engine::export(&base, &settings, &output, "png").unwrap();
    let exported = image::open(&output).unwrap().into_rgba16();
    assert!(exported.get_pixel(20, 15)[0].abs_diff(u16::from(pixel[0]) * 257) <= 129);
    assert_eq!(exported.get_pixel(20, 15)[3], 32768);
    base.info.source_kind = "rtengine-linear".into();
    let raster = engine::render(&base, &settings, &|| false).unwrap().image;
    assert_eq!(raster.get_pixel(20, 15)[0], 118);
    assert_eq!(base.pixels.get_pixel(20, 15)[0], 0.18);
}

#[test]
fn camera_saturation_is_shared_by_cached_preview_and_export_without_changing_rasters() {
    let directory = tempfile::tempdir().unwrap();
    let mut base = engine::WorkingImage {
        pixels: Rgba32FImage::from_fn(32, 16, |x, _| {
            Rgba(if x < 16 {
                [0.16, 0.24, 0.12, 0.5]
            } else {
                [0.18, 0.18, 0.18, 0.5]
            })
        }),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 32,
            source_height: 16,
            width: 32,
            height: 16,
            source_kind: "rtengine-raw-camera".into(),
            color_note: String::new(),
        },
    };
    let neutral = engine::render(&base, &Settings::default(), &|| false)
        .unwrap()
        .image;
    base.info.camera_saturation = Some(0.8);
    let reduced = engine::render(&base, &Settings::default(), &|| false)
        .unwrap()
        .image;
    assert!(reduced.get_pixel(0, 0)[1] < neutral.get_pixel(0, 0)[1]);
    assert!(reduced.get_pixel(0, 0)[2] > neutral.get_pixel(0, 0)[2]);
    assert_eq!(reduced.get_pixel(20, 0), neutral.get_pixel(20, 0));
    assert_eq!(reduced.get_pixel(0, 0)[3], 128);
    base.info.camera_saturation = Some(1.4);
    let cache = directory.path().join("base.tif");
    engine::write_base(&cache, &base).unwrap();
    let info = serde_json::from_slice(&serde_json::to_vec(&base.info).unwrap()).unwrap();
    let cached = engine::read_base(&cache, info).unwrap();
    let adjusted = engine::render(&cached, &Settings::default(), &|| false)
        .unwrap()
        .image;
    assert!(adjusted.get_pixel(0, 0)[1] > neutral.get_pixel(0, 0)[1]);
    assert!(adjusted.get_pixel(0, 0)[2] < neutral.get_pixel(0, 0)[2]);
    assert_eq!(adjusted.get_pixel(20, 0), neutral.get_pixel(20, 0));
    assert_eq!(adjusted.get_pixel(0, 0)[3], 128);
    for settings in [
        Settings::default(),
        Settings {
            saturation: -100.,
            ..Default::default()
        },
        Settings {
            monochrome: true,
            ..Default::default()
        },
    ] {
        let preview = engine::render(&cached, &settings, &|| false).unwrap().image;
        if settings.monochrome || settings.saturation == -100. {
            assert!(preview.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        }
        let output = directory.path().join(format!(
            "{}-{}.png",
            settings.saturation, settings.monochrome
        ));
        engine::export(&cached, &settings, &output, "png").unwrap();
        let output = image::open(output).unwrap().to_rgba16();
        assert!(
            output
                .as_raw()
                .iter()
                .zip(preview.as_raw())
                .all(|(a, b)| a.abs_diff(u16::from(*b) * 257) <= 129)
        );
    }
    base.info.source_kind = "raster-display-linear".into();
    base.info.camera_saturation = None;
    assert_eq!(
        engine::render(&base, &Settings::default(), &|| false)
            .unwrap()
            .image,
        neutral
    );
}

#[test]
fn raw_profile_keeps_distinct_highlights_available_for_recovery() {
    let pixels = Rgba32FImage::from_fn(3, 1, |x, _| {
        Rgba([
            0.65 + x as f32 * 0.1,
            0.65 + x as f32 * 0.1,
            0.65 + x as f32 * 0.1,
            1.,
        ])
    });
    let base = engine::WorkingImage {
        pixels: pixels.clone(),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raster_wb_temperature: None,
            source_width: 3,
            source_height: 1,
            width: 3,
            height: 1,
            source_kind: "raw-camera-linear".into(),
            color_note: String::new(),
            raw: Some(advanced_explorer_media::develop_input::RawColor {
                camera: "OpenRoom linear ProPhoto regression camera".into(),
                neutral: [1.; 3],
                xyz_to_camera: [
                    [1.3459433, -0.2556075, -0.0511118],
                    [-0.5445989, 1.5081673, 0.0205351],
                    [0., 0., 1.2118128],
                ],
                baseline_exposure: 1.,
                calibration: [[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]; 2],
                calibration_signature: String::new(),
                analog_balance: [1.; 3],
                as_shot: [5000., 0.],
                profile_signature: String::new(),
            }),
        },
    };
    let recovered = engine::render(
        &base,
        &Settings {
            highlights: -100.,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    let levels: std::collections::HashSet<_> =
        recovered.image.pixels().map(|pixel| pixel[0]).collect();
    assert_eq!(
        levels.len(),
        3,
        "profile clipping flattened recoverable RAW highlights"
    );
    assert!(recovered.image.pixels().all(|pixel| pixel[0] < 255));
    let whites = engine::render(
        &base,
        &Settings {
            whites: -100.,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    assert!(
        whites
            .image
            .pixels()
            .map(|pixel| pixel[0])
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1
    );
    assert_eq!(pixels, base.pixels);
}

#[test]
fn damaged_float_cache_is_rejected_instead_of_rendering_invalid_samples() {
    let directory = tempfile::tempdir().unwrap();
    for value in [f32::NAN, f32::INFINITY] {
        let path = directory.path().join(format!("{value}.tif"));
        let base = engine::WorkingImage {
            pixels: Rgba32FImage::from_pixel(1, 1, Rgba([value, 0.5, 0.5, 1.])),
            info: engine::BaseInfo {
                camera_saturation: None,
                native_source: None,
                raw: None,
                raster_wb_temperature: None,
                source_width: 1,
                source_height: 1,
                width: 1,
                height: 1,
                source_kind: "raster-linear".into(),
                color_note: String::new(),
            },
        };
        engine::write_base(&path, &base).unwrap();
        assert!(engine::read_base(&path, base.info).is_err());
    }
}

#[test]
fn editing_an_unrelated_hue_does_not_clip_highlights_before_vignetting() {
    let base = engine::WorkingImage {
        pixels: Rgba32FImage::from_pixel(3, 3, Rgba([0.1, 0.1, 4., 1.])),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 3,
            source_height: 3,
            width: 3,
            height: 3,
            source_kind: "raster-linear".into(),
            color_note: String::new(),
        },
    };
    let mut settings = Settings {
        vignette: -100.,
        ..Default::default()
    };
    let normal = engine::render(&base, &settings, &|| false).unwrap().image;
    settings.mixer[0] = [20., 0., 0.];
    assert_eq!(
        normal,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
}

#[test]
fn hue_edits_keep_highlight_brightness_and_leave_neutral_highlights_unchanged() {
    let mut base = engine::WorkingImage {
        pixels: Rgba32FImage::from_pixel(3, 3, Rgba([4., 0.1, 0.1, 1.])),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 3,
            source_height: 3,
            width: 3,
            height: 3,
            source_kind: "raster-linear".into(),
            color_note: String::new(),
        },
    };
    let mut settings = Settings {
        vignette: -100.,
        ..Default::default()
    };
    let normal = engine::render(&base, &settings, &|| false).unwrap().image;
    settings.mixer[0] = [20., 0., 0.];
    let edited = engine::render(&base, &settings, &|| false).unwrap().image;
    assert_eq!(normal.get_pixel(0, 0)[0], edited.get_pixel(0, 0)[0]);
    assert!(edited.get_pixel(0, 0)[0] > 200);
    base.pixels = Rgba32FImage::from_pixel(3, 3, Rgba([4., 4., 4., 1.]));
    settings.mixer[0] = [0., 0., 50.];
    let adjusted = engine::render(&base, &settings, &|| false).unwrap().image;
    settings.mixer = [[0.; 3]; 8];
    assert_eq!(
        adjusted,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
}

#[test]
fn midtone_curves_do_not_discard_highlight_headroom_before_vignetting() {
    let base = engine::WorkingImage {
        pixels: Rgba32FImage::from_pixel(3, 3, Rgba([4., 4., 4., 1.])),
        info: engine::BaseInfo {
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 3,
            source_height: 3,
            width: 3,
            height: 3,
            source_kind: "raster-linear".into(),
            color_note: String::new(),
        },
    };
    let mut settings = Settings {
        vignette: -100.,
        ..Default::default()
    };
    let normal = engine::render(&base, &settings, &|| false).unwrap().image;
    settings.curves[0][2] = 0.6;
    assert_eq!(
        normal,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
    settings.point_curves[0] = vec![[0., 0.], [0.5, 0.6], [1., 1.]];
    assert_eq!(
        normal,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
}

#[test]
fn float_tiff_retains_linear_highlights_through_cache_and_export() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("linear-hdr.tif");
    let levels = [0.5, 1., 2., 4., 8., 16., 32.];
    DynamicImage::ImageRgba32F(Rgba32FImage::from_fn(7, 1, |x, _| {
        Rgba([
            levels[x as usize],
            levels[x as usize],
            levels[x as usize],
            1.,
        ])
    }))
    .save(&source)
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    let base = engine::prepare(&source, 0).unwrap();
    assert_eq!(base.info.source_kind, "raster-linear");
    for (x, level) in levels.into_iter().enumerate() {
        assert!((base.pixels.get_pixel(x as u32, 0)[0] - level).abs() < 1e-5);
    }
    let cache = directory.path().join("base.tif");
    engine::write_base(&cache, &base).unwrap();
    let restored = engine::read_base(&cache, base.info.clone()).unwrap();
    let destination = directory.path().join("recovered.png");
    engine::export(
        &restored,
        &Settings {
            exposure: -5.,
            ..Default::default()
        },
        &destination,
        "png",
    )
    .unwrap();
    let output = image::open(&destination).unwrap().into_rgba16();
    for (x, level) in levels.into_iter().enumerate() {
        let value = level / 32.;
        let encoded = if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1. / 2.4) - 0.055
        };
        assert!((output.get_pixel(x as u32, 0)[0] as f32 / 65535. - encoded).abs() < 2. / 65535.);
    }
    assert_eq!(original, std::fs::read(source).unwrap());
}

#[test]
fn wide_gamut_colors_are_not_clipped_before_monochrome_conversion() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("p3.png");
    let profile = moxcms::ColorProfile::new_display_p3();
    let file = std::fs::File::create(&source).unwrap();
    let mut encoder = image::codecs::png::PngEncoder::new(file);
    encoder.set_icc_profile(profile.encode().unwrap()).unwrap();
    DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(
        1,
        1,
        image::Rgb([65535u16, 0, 0]),
    ))
    .write_with_encoder(encoder)
    .unwrap();
    let base = engine::prepare(&source, 0).unwrap();
    let expected = profile
        .transform_matrix(&moxcms::ColorProfile::new_srgb())
        .v
        .map(|row| row[0] as f32);
    for (value, expected) in base.pixels.get_pixel(0, 0).0[..3].iter().zip(expected) {
        assert!(
            (*value - expected).abs() < 0.001,
            "ICC input lost gamut: {value} != {expected}"
        );
    }
    let luminance = 0.2126 * expected[0] + 0.7152 * expected[1] + 0.0722 * expected[2];
    let gray = ((1.055 * luminance.powf(1. / 2.4) - 0.055) * 255.).round() as i16;
    let result = engine::render(
        &base,
        &Settings {
            monochrome: true,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    for channel in &result.image.get_pixel(0, 0).0[..3] {
        assert!(
            (*channel as i16 - gray).abs() <= 1,
            "early gamut clipping changed gray: {channel} != {gray}"
        );
    }
    let desaturated = engine::render(
        &base,
        &Settings {
            saturation: -100.,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(
        desaturated.image, result.image,
        "desaturation must retain the luminance of extended-gamut red"
    );
}

#[test]
fn neutral_full_resolution_export_does_not_blur_or_downsample_fine_detail() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("detail.png");
    let input = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(1701, 17, |x, y| {
        image::Rgb([
            if x % 2 == 0 { 0 } else { 65535 },
            if y % 2 == 0 { 65535 } else { 0 },
            ((x * 173 + y * 997) % 65536) as u16,
        ])
    });
    input.save(&source).unwrap();
    let base = engine::prepare(&source, 0).unwrap();
    let destination = directory.path().join("detail.tif");
    engine::export(&base, &Settings::default(), &destination, "tiff").unwrap();
    let output = image::open(destination).unwrap().into_rgb16();
    assert_eq!(output.dimensions(), input.dimensions());
    assert!(
        output
            .as_raw()
            .iter()
            .zip(input.as_raw())
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
}

#[test]
fn sixteen_bit_lossless_exports_keep_fine_gradient_levels() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("gradient.png");
    let input = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(4096, 1, |x, _| {
        image::Rgb([8192 + x as u16; 3])
    });
    for icc in [false, true] {
        if icc {
            let mut encoder =
                image::codecs::png::PngEncoder::new(std::fs::File::create(&source).unwrap());
            encoder
                .set_icc_profile(moxcms::ColorProfile::new_srgb().encode().unwrap())
                .unwrap();
            DynamicImage::ImageRgb16(input.clone())
                .write_with_encoder(encoder)
                .unwrap();
        } else {
            input.save(&source).unwrap();
        }
        let base = engine::prepare(&source, 0).unwrap();
        for format in ["png", "tiff"] {
            let destination = directory.path().join(format!("export-{icc}.{format}"));
            engine::export(&base, &Settings::default(), &destination, format).unwrap();
            let decoded = image::open(destination).unwrap();
            assert_eq!(decoded.color(), image::ColorType::Rgba16);
            let output = decoded.into_rgba16();
            let distinct: std::collections::HashSet<_> =
                output.pixels().map(|pixel| pixel[0]).collect();
            assert_eq!(distinct.len(), 4096, "{format} gradient was quantized");
            for x in 0..4096 {
                assert!((i32::from(output.get_pixel(x, 0)[0]) - (8192 + x as i32)).abs() <= 1);
            }
        }
    }
}
