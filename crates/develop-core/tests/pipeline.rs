use advanced_explorer_develop::{self as engine, BaseInfo, Settings, WorkingImage};
use image::{Rgba, Rgba32FImage};
fn fixture() -> WorkingImage {
    let pixels = Rgba32FImage::from_fn(40, 30, |x, y| {
        Rgba([
            x as f32 / 50.,
            y as f32 / 40.,
            ((x * 13 + y * 7) % 31) as f32 / 45.,
            1.,
        ])
    });
    WorkingImage {
        pixels,
        info: BaseInfo {
            auto_ca_supported: None,
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
            source_width: 40,
            source_height: 30,
            width: 40,
            height: 30,
            source_kind: "test".into(),
            color_note: "linear sRGB".into(),
        },
    }
}
#[test]
fn compact_catalog_recipes_restore_exact_settings_and_pixels() {
    let defaults = Settings::default();
    assert_eq!(defaults.catalog_recipe().unwrap(), r#"{"version":1}"#);
    let mut edited = Settings {
        exposure: 1.25,
        temperature: -18.,
        monochrome: true,
        crop: [0.1, 0.2, 0.8, 0.7],
        ..Default::default()
    };
    edited.curves[2][3] = 0.83;
    edited.mixer[6] = [12., -34., 56.];
    edited.grading[1] = [210., 15., -3.];
    let compact = edited.catalog_recipe().unwrap();
    let restored: Settings = serde_json::from_str(&compact).unwrap();
    let legacy: Settings = serde_json::from_str(&serde_json::to_string(&edited).unwrap()).unwrap();
    assert_eq!(restored, edited);
    assert_eq!(legacy, edited);
    assert!(compact.len() < serde_json::to_string(&edited).unwrap().len() / 2);
    assert_eq!(
        engine::render(&fixture(), &restored, &|| false)
            .unwrap()
            .image,
        engine::render(&fixture(), &edited, &|| false)
            .unwrap()
            .image
    );
    edited.exposure = f32::NAN;
    assert!(edited.catalog_recipe().is_err());
}
#[test]
fn lens_recipes_keep_legacy_defaults_and_validate_ranges() {
    let legacy: Settings = serde_json::from_str(r#"{"version":1,"exposure":1}"#).unwrap();
    assert!(!legacy.ca_auto && !legacy.defringe);
    assert_eq!((legacy.ca_red, legacy.ca_blue), (0., 0.));
    let edited: Settings = serde_json::from_value(serde_json::json!({
        "ca_auto":true,"ca_red":-40,"ca_blue":60,"defringe":true,"defringe_radius":3.2,"defringe_threshold":20
    })).unwrap();
    assert_eq!(
        serde_json::from_str::<Settings>(&edited.catalog_recipe().unwrap()).unwrap(),
        edited
    );
    for patch in [
        serde_json::json!({"ca_red":101}),
        serde_json::json!({"ca_blue":-101}),
        serde_json::json!({"defringe_radius":0}),
        serde_json::json!({"defringe_threshold":101}),
    ] {
        assert!(
            serde_json::from_value::<Settings>(patch)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
#[test]
fn extended_saturation_survives_recipes_and_increases_color_strength() {
    let mut base = fixture();
    base.pixels = Rgba32FImage::from_pixel(40, 30, Rgba([0.22, 0.2, 0.18, 1.]));
    let mut previous_spread = 0;
    for amount in [-100., 0., 100., 300.] {
        let settings = Settings {
            saturation: amount,
            ..Default::default()
        };
        let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, settings);
        let result = engine::render(&base, &restored, &|| false).unwrap().image;
        let pixel = result.get_pixel(0, 0);
        let spread = pixel[0] - pixel[2];
        if amount == -100. {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(spread, 0);
        } else {
            assert!(spread > previous_spread, "saturation {amount}: {pixel:?}");
        }
        previous_spread = spread;

        let mut mask = linear_mask();
        mask.adjustments = [("saturation".into(), amount)].into();
        let local = Settings {
            masks: vec![mask],
            ..Default::default()
        };
        let restored: Settings = serde_json::from_str(&local.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, local);
        let masked = engine::render(&base, &restored, &|| false).unwrap().image;
        let neutral = engine::render(&base, &Settings::default(), &|| false)
            .unwrap()
            .image;
        assert_eq!(masked.get_pixel(20, 25), neutral.get_pixel(20, 25));
        if amount != 0. {
            assert_ne!(masked.get_pixel(20, 2), neutral.get_pixel(20, 2));
        }
    }
    for amount in [-101., 301., f32::NAN, f32::INFINITY] {
        assert!(
            Settings {
                saturation: amount,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let mut mask = linear_mask();
        mask.adjustments = [("saturation".into(), amount)].into();
        assert!(mask.settings().is_err());
    }
}

#[test]
fn point_curves_change_pixels_and_reject_invalid_coordinates() {
    let mut settings = Settings::default();
    let neutral = engine::render(&fixture(), &settings, &|| false).unwrap();
    settings.point_curves[0] = vec![[0., 0.], [0.4, 0.7], [1., 1.]];
    let curved = engine::render(&fixture(), &settings, &|| false).unwrap();
    assert_ne!(neutral.image, curved.image);
    let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
    assert_eq!(restored, settings);
    assert_eq!(
        engine::render(&fixture(), &restored, &|| false)
            .unwrap()
            .image,
        curved.image
    );
    for points in [
        vec![[0., 0.], [0.4, 0.7], [0.4, 0.8], [1., 1.]],
        vec![[0., 0.], [0.5, f32::NAN], [1., 1.]],
        vec![[0.1, 0.], [1., 1.]],
        vec![[0., 0.], [1., 1.1]],
    ] {
        settings.point_curves[0] = points;
        assert!(settings.validate().is_err());
    }
}
#[test]
fn default_is_identity_and_exposure_is_linear() {
    let mut base = fixture();
    base.pixels = Rgba32FImage::from_pixel(40, 30, Rgba([0.1, 0.2, 0.3, 1.]));
    let neutral = engine::render(&base, &Settings::default(), &|| false).unwrap();
    assert_eq!(neutral.image.get_pixel(0, 0).0, [89, 124, 149, 255]);
    let plus = engine::render(
        &base,
        &Settings {
            exposure: 1.,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(plus.image.get_pixel(0, 0).0, [124, 170, 203, 255]);
    assert_eq!(neutral.info.histogram[0].iter().sum::<u32>(), 1200);
    assert_eq!(base.pixels.get_pixel(0, 0).0, [0.1, 0.2, 0.3, 1.]);
}
#[test]
fn raw_headroom_survives_working_cache_and_negative_exposure() {
    let mut base = fixture();
    base.info.source_kind = "raw-sensor-linear".into();
    base.pixels.put_pixel(10, 10, Rgba([4., 2., 1., 1.]));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("base.tif");
    engine::write_base(&path, &base).unwrap();
    let restored = engine::read_base(&path, base.info.clone()).unwrap();
    assert_eq!(restored.pixels.get_pixel(10, 10).0, [4., 2., 1., 1.]);
    let output = engine::render(
        &restored,
        &Settings {
            exposure: -3.,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(output.image.get_pixel(10, 10).0, [188, 137, 99, 255]);
}

#[test]
fn display_input_exposure_survives_cache_reload() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ramp.png");
    let input = [32, 64, 96, 128, 160, 192, 224];
    image::RgbImage::from_fn(7, 1, |x, _| image::Rgb([input[x as usize]; 3]))
        .save(&source)
        .unwrap();
    let base = engine::prepare(&source, 0).unwrap();
    assert_eq!(base.info.source_kind, "raster-display-linear");
    let cache = temp.path().join("base.tif");
    engine::write_base(&cache, &base).unwrap();
    let restored = engine::read_base(&cache, base.info.clone()).unwrap();
    let neutral = engine::render(&restored, &Settings::default(), &|| false).unwrap();
    let settings = Settings {
        exposure: 1.,
        ..Default::default()
    };
    let edited = engine::render(&restored, &settings, &|| false).unwrap();
    let expected = [55, 102, 146, 181, 207, 229, 244];
    for x in 0..7 {
        assert_eq!(neutral.image.get_pixel(x, 0)[0], input[x as usize]);
        assert!((edited.image.get_pixel(x, 0)[0] as i16 - expected[x as usize]).abs() <= 2);
    }
    assert_eq!(
        edited.image,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
}
#[test]
fn geometry_crop_and_monochrome_change_pixels_and_dimensions() {
    let base = fixture();
    let settings = Settings {
        quarter_turns: 1,
        crop: [0.1, 0.2, 0.5, 0.5],
        monochrome: true,
        ..Default::default()
    };
    let output = engine::render(&base, &settings, &|| false).unwrap();
    assert_eq!(output.image.dimensions(), (15, 20));
    assert!(output.image.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
    let flipped = engine::render(
        &base,
        &Settings {
            flip_h: true,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    let normal = engine::render(&base, &Settings::default(), &|| false).unwrap();
    assert_eq!(flipped.image.get_pixel(0, 0), normal.image.get_pixel(39, 0));
}
#[test]
fn adjustments_have_an_effect_and_are_deterministic() {
    assert_adjustments(&fixture());
}

#[test]
fn texture_changes_medium_detail_at_preview_and_full_size_without_tinting() {
    for scale in [1., 1600. / 6000., 384. / 6000.] {
        let period = (24_f32 * scale).max(4.);
        let mut base = fixture();
        base.pixels = Rgba32FImage::from_fn(128, 96, |x, _| {
            let value = 0.25 + 0.025 * (x as f32 * std::f32::consts::TAU / period).sin();
            Rgba([value, value, value, 0.5])
        });
        base.info.width = 128;
        base.info.height = 96;
        base.info.source_width = (128. / scale) as u32;
        base.info.source_height = (96. / scale) as u32;
        let contrast = |image: &image::RgbaImage| {
            let values: Vec<_> = (16..112)
                .map(|x| image.get_pixel(x, 48)[0] as f32)
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
        };
        let normal = engine::render(&base, &Settings::default(), &|| false)
            .unwrap()
            .image;
        for amount in [-100., 100.] {
            let settings = Settings {
                texture: amount,
                ..Default::default()
            };
            let output = engine::render(&base, &settings, &|| false).unwrap().image;
            let ratio = contrast(&output) / contrast(&normal);
            assert!(
                if amount > 0. {
                    ratio > 1.08
                } else {
                    ratio < 0.92
                },
                "texture {amount} at scale {scale}: contrast ratio {ratio}"
            );
            assert!(
                output
                    .pixels()
                    .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 128)
            );
        }
        base.pixels = Rgba32FImage::from_pixel(128, 96, Rgba([0.25, 0.25, 0.25, 0.5]));
        assert_eq!(
            engine::render(&base, &Settings::default(), &|| false)
                .unwrap()
                .image,
            engine::render(
                &base,
                &Settings {
                    texture: 100.,
                    ..Default::default()
                },
                &|| false
            )
            .unwrap()
            .image,
            "texture must leave flat regions unchanged"
        );
    }
}
#[cfg(all(target_os = "macos", feature = "rtengine"))]
#[test]
fn native_adjustments_have_an_effect_and_are_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("native.png");
    image::RgbaImage::from_fn(96, 64, |x, y| {
        image::Rgba([
            (30 + x * 2) as u8,
            (20 + y * 3) as u8,
            ((x * 13 + y * 7) % 180 + 20) as u8,
            255,
        ])
    })
    .save(&source)
    .unwrap();
    assert_adjustments(&engine::prepare(&source, 0).unwrap());
}
#[cfg(all(target_os = "macos", feature = "rtengine"))]
#[test]
fn lens_corrections_match_preview_and_export_without_changing_original() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lens.png");
    image::RgbaImage::from_fn(192, 128, |x, y| {
        image::Rgba([
            ((x * 17 + y * 11) % 230 + 10) as u8,
            ((x * 13 + y * 7) % 230 + 10) as u8,
            ((x * 19 + y * 3) % 230 + 10) as u8,
            180,
        ])
    })
    .save(&source)
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    let base = engine::prepare(&source, 0).unwrap();
    let neutral = engine::render(&base, &Settings::default(), &|| false)
        .unwrap()
        .image;
    for (name, settings) in [
        (
            "ca",
            Settings {
                ca_red: 80.,
                ca_blue: -70.,
                ..Default::default()
            },
        ),
        (
            "fringe",
            Settings {
                defringe: true,
                ..Default::default()
            },
        ),
        (
            "both",
            Settings {
                ca_red: 80.,
                defringe: true,
                noise_luma: 10.,
                sharpen: 30.,
                ..Default::default()
            },
        ),
    ] {
        let preview = engine::render(&base, &settings, &|| false).unwrap().image;
        assert_ne!(preview, neutral, "{name} must reach the rendering pipeline");
        let output = directory.path().join(format!("{name}.png"));
        engine::export_source_with_options(&source, &settings, &output, "png", &Default::default())
            .unwrap();
        assert_eq!(
            image::open(output).unwrap().to_rgba8(),
            preview,
            "{name}: preview/export differ"
        );
        assert!(preview.pixels().all(|p| p[3] == 180));
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert_eq!(
        engine::render(&base, &Settings::default(), &|| false)
            .unwrap()
            .image,
        neutral
    );
}
fn assert_adjustments(base: &WorkingImage) {
    let normal = engine::render(base, &Settings::default(), &|| false).unwrap();
    let keys = [
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
        "vibrance",
        "saturation",
        "sharpen",
        "noise_luma",
        "noise_color",
        "angle",
        "perspective_h",
        "perspective_v",
        "distortion",
        "vignette",
        "grain",
        "aspect",
        "offset_x",
        "offset_y",
        "scale",
    ];
    for key in keys {
        let value = if key == "exposure" {
            1.
        } else if key == "scale" {
            130.
        } else {
            30.
        };
        let settings: Settings = serde_json::from_value(serde_json::json!({key:value})).unwrap();
        let output = engine::render(base, &settings, &|| false).unwrap();
        assert_ne!(output.image, normal.image, "{key} did not affect output");
        assert_eq!(
            output.image,
            engine::render(base, &settings, &|| false).unwrap().image,
            "{key} is not deterministic"
        );
    }
    // Compare dependent controls against the same enabled parent, not against an unedited image.
    for (key, parent, value) in [
        ("sharpen_radius", serde_json::json!({"sharpen":100}), 3.),
        ("sharpen_detail", serde_json::json!({"sharpen":100}), 100.),
        ("sharpen_masking", serde_json::json!({"sharpen":100}), 100.),
        ("vignette_midpoint", serde_json::json!({"vignette":-70}), 0.),
        ("vignette_feather", serde_json::json!({"vignette":-70}), 0.),
        ("grain_size", serde_json::json!({"grain":70}), 100.),
        ("grain_roughness", serde_json::json!({"grain":70}), 100.),
        (
            "grading_balance",
            serde_json::json!({"grading":[[220,40,0],[100,10,0],[40,30,0],[0,0,0]]}),
            80.,
        ),
        (
            "grading_blending",
            serde_json::json!({"grading":[[220,40,0],[100,10,0],[40,30,0],[0,0,0]]}),
            100.,
        ),
    ] {
        let before: Settings = serde_json::from_value(parent.clone()).unwrap();
        let mut edited = parent;
        edited[key] = serde_json::json!(value);
        let after: Settings = serde_json::from_value(edited).unwrap();
        assert_ne!(
            engine::render(base, &before, &|| false).unwrap().image,
            engine::render(base, &after, &|| false).unwrap().image,
            "{key} did not affect output"
        );
    }
    for group in ["curves", "mixer", "grading"] {
        let channels = if group == "mixer" { 8 } else { 4 };
        let components = if group == "curves" { 5 } else { 3 };
        for channel in 0..channels {
            for component in 0..components {
                let mut before = Settings::default();
                if group == "grading" {
                    before.grading[channel] = [0., 40., 0.];
                }
                let mut after = before.clone();
                match group {
                    "curves" => {
                        after.curves[channel][component] = if component < 4 { 1. } else { 0. }
                    }
                    "mixer" => after.mixer[channel][component] = 70.,
                    _ => {
                        after.grading[channel][component] = if component == 0 { 220. } else { 70. }
                    }
                }
                assert_ne!(
                    engine::render(base, &before, &|| false).unwrap().image,
                    engine::render(base, &after, &|| false).unwrap().image,
                    "{group}[{channel}][{component}] did not affect output"
                );
            }
        }
    }
    for settings in [
        Settings {
            curves: [[0., 0.3, 0.6, 0.8, 1.]; 4],
            ..Default::default()
        },
        Settings {
            mixer: [[20., 30., 20.]; 8],
            ..Default::default()
        },
        Settings {
            grading: [[220., 40., 10.]; 4],
            ..Default::default()
        },
    ] {
        assert_ne!(
            engine::render(base, &settings, &|| false).unwrap().image,
            normal.image
        );
    }
}
#[test]
fn invalid_recipes_and_cancellation_are_rejected() {
    let base = fixture();
    for json in [
        r#"{"crop":[0.9,0,0.5,1]}"#,
        r#"{"exposure":6}"#,
        r#"{"version":2}"#,
        r#"{"sharpen_radius":0}"#,
        r#"{"quarter_turns":4}"#,
    ] {
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert!(engine::render(&base, &settings, &|| false).is_err());
    }
    assert!(
        Settings {
            exposure: f32::NAN,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(serde_json::from_str::<Settings>(r#"{"future_field":2}"#).is_err());
    assert!(engine::render(&base, &Settings::default(), &|| true).is_err());
}
#[test]
fn export_from_source_matches_prepared_export_and_refuses_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.png");
    image::RgbaImage::from_pixel(40, 30, image::Rgba([80, 120, 160, 100]))
        .save(&source)
        .unwrap();
    let settings = Settings {
        exposure: 0.5,
        texture: 15.,
        ..Default::default()
    };
    let prepared = engine::prepare(&source, 0).unwrap();
    let reference = directory.path().join("reference.png");
    let output = directory.path().join("output.png");
    engine::export(&prepared, &settings, &reference, "png").unwrap();
    engine::export_source_with_options(&source, &settings, &output, "png", &Default::default())
        .unwrap();
    assert_eq!(
        std::fs::read(&reference).unwrap(),
        std::fs::read(&output).unwrap()
    );
    assert!(
        engine::export_source_with_options(&source, &settings, &output, "png", &Default::default())
            .is_err()
    );
}

#[test]
fn exports_preserve_original_and_refuse_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.png");
    image::RgbImage::from_pixel(40, 30, image::Rgb([80, 120, 160]))
        .save(&source)
        .unwrap();
    let original = std::fs::read(&source).unwrap();
    let base = engine::prepare(&source, 1600).unwrap();
    assert!(engine::export(&base, &Settings::default(), &source, "png").is_err());
    for (format, extension) in [("jpeg", "jpg"), ("png", "png"), ("tiff", "tif")] {
        let output = dir.path().join(format!("edited.{extension}"));
        engine::export(
            &base,
            &Settings {
                exposure: 0.5,
                ..Default::default()
            },
            &output,
            format,
        )
        .unwrap();
        let decoded = image::open(&output).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 30));
        assert_eq!(
            decoded.color().bits_per_pixel(),
            if format == "jpeg" { 24 } else { 64 }
        );
        use image::ImageDecoder;
        if format == "tiff" {
            let mut decoder = image::codecs::tiff::TiffDecoder::new(std::io::BufReader::new(
                std::fs::File::open(&output).unwrap(),
            ))
            .unwrap();
            assert!(decoder.icc_profile().unwrap().is_some());
        } else {
            let mut decoder = image::ImageReader::open(&output)
                .unwrap()
                .into_decoder()
                .unwrap();
            assert!(
                decoder.icc_profile().unwrap().is_some(),
                "{format} ICC missing"
            );
        }
        assert!(
            engine::prepare(&output, 1600)
                .unwrap()
                .info
                .color_note
                .contains("ICCプロファイル")
        );
        let before = std::fs::read(&output).unwrap();
        assert!(engine::export(&base, &Settings::default(), &output, format).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), before);
    }
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn embedded_srgb_profile_is_converted_and_reported() {
    use image::ImageEncoder;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.png");
    let mut encoder = image::codecs::png::PngEncoder::new(std::fs::File::create(&path).unwrap());
    encoder
        .set_icc_profile(moxcms::ColorProfile::new_srgb().encode().unwrap())
        .unwrap();
    encoder
        .write_image(&[80, 120, 160], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    let base = engine::prepare(&path, 1600).unwrap();
    assert!(base.info.color_note.contains("ICCプロファイル"));
    let output = engine::render(&base, &Settings::default(), &|| false).unwrap();
    for (actual, expected) in output.image.get_pixel(0, 0).0[..3]
        .iter()
        .zip([80u8, 120, 160])
    {
        assert!(actual.abs_diff(expected) <= 1);
    }
}

#[test]
fn cache_rejects_bad_headers_and_empty_working_images() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.tif");
    let mut base = fixture();
    std::fs::write(&path, b"truncated TIFF").unwrap();
    assert!(engine::read_base(&path, base.info.clone()).is_err());
    // A valid but wrong format must not silently replace the linear float base.
    image::RgbaImage::new(40, 30).save(&path).unwrap();
    assert!(engine::read_base(&path, base.info.clone()).is_err());
    std::fs::remove_file(&path).unwrap();
    engine::write_base(&path, &base).unwrap();
    base.info.width = 41;
    assert!(engine::read_base(&path, base.info.clone()).is_err());
    base.pixels = Rgba32FImage::new(0, 0);
    assert!(engine::render(&base, &Settings::default(), &|| false).is_err());
}

#[test]
fn cancellation_during_render_preserves_working_pixels() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let base = fixture();
    let original = base.pixels.clone();
    let checks = AtomicUsize::new(0);
    let result = engine::render(
        &base,
        &Settings {
            sharpen: 30.,
            clarity: 20.,
            ..Default::default()
        },
        &|| checks.fetch_add(1, Ordering::SeqCst) > 2,
    );
    assert!(result.is_err());
    assert_eq!(base.pixels, original);
}

fn linear_mask() -> advanced_explorer_develop::masks::Mask {
    serde_json::from_value(serde_json::json!({"name":"空","enabled":true,"invert":false,"amount":1,"adjustments":{"exposure":1},"shapes":[{"kind":"linear","operation":"add","start":[0,0],"end":[0,0.5],"radius":0.1,"feather":0.5,"opacity":1,"points":[],"color":[0.5,0.5,0.5],"range":[0,1]}]})).unwrap()
}
#[test]
fn local_edits_preserve_unselected_pixels_and_crop_coordinates() {
    let base = fixture();
    let normal = engine::render(&base, &Settings::default(), &|| false)
        .unwrap()
        .image;
    let settings = Settings {
        masks: vec![linear_mask()],
        ..Default::default()
    };
    let masked = engine::render(&base, &settings, &|| false).unwrap().image;
    assert!(masked.get_pixel(20, 2)[0] > normal.get_pixel(20, 2)[0]);
    assert_eq!(masked.get_pixel(20, 25), normal.get_pixel(20, 25));
    let crop = Settings {
        crop: [0., 0.5, 1., 0.5],
        ..settings.clone()
    };
    let cropped = engine::render(&base, &crop, &|| false).unwrap().image;
    assert_eq!(
        cropped,
        image::imageops::crop_imm(&masked, 0, 15, 40, 15).to_image()
    );
    let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
    assert_eq!(settings, restored);
    let mut invalid = settings;
    invalid.masks[0].adjustments.insert("crop".into(), 1.);
    assert!(invalid.validate().is_err());
}

#[test]
fn mask_layers_share_basic_adjustments_without_changing_unselected_pixels() {
    let base = fixture();
    let settings = Settings {
        exposure: 0.5,
        ..Default::default()
    };
    let normal = engine::render(&base, &settings, &|| false).unwrap().image;
    for kind in ["brush", "linear", "radial"] {
        let mut mask = linear_mask();
        mask.shapes[0].kind = kind.into();
        mask.shapes[0].start = [0.3, 0.3];
        mask.shapes[0].end = [0.6, 0.6];
        mask.shapes[0].points = vec![[0.3, 0.3], [0.6, 0.4]];
        mask.shapes[0].radius = 0.2;
        let weights = mask.weights(&base.pixels, [0., 0., 1., 1.], 40. / 30.);
        for key in [
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
            "vibrance",
            "saturation",
            "monochrome",
        ] {
            mask.adjustments.clear();
            mask.adjustments.insert(
                key.into(),
                if ["exposure", "monochrome"].contains(&key) {
                    1.
                } else {
                    70.
                },
            );
            let edited = Settings {
                masks: vec![mask.clone()],
                ..settings.clone()
            };
            let output = engine::render(&base, &edited, &|| false).unwrap().image;
            assert_ne!(output, normal, "{kind}: {key} has no effect");
            for ((actual, expected), weight) in output.pixels().zip(normal.pixels()).zip(&weights) {
                assert_eq!(actual[3], expected[3]);
                if *weight == 0. {
                    assert_eq!(actual, expected, "{kind}: {key} leaked outside selection");
                }
            }
        }
        mask.adjustments.clear();
        for neutral in [mask.clone(), {
            let mut disabled = mask.clone();
            disabled.enabled = false;
            disabled.adjustments.insert("exposure".into(), 2.);
            disabled
        }] {
            assert_eq!(
                normal,
                engine::render(
                    &base,
                    &Settings {
                        masks: vec![neutral],
                        ..settings.clone()
                    },
                    &|| false
                )
                .unwrap()
                .image
            );
        }
    }
}

#[test]
fn mask_order_and_monochrome_survive_storage_and_export() {
    let base = fixture();
    let first = linear_mask();
    let mut second = first.clone();
    second.name = "二層目".into();
    second.adjustments.clear();
    second.adjustments.insert("contrast".into(), 70.);
    let mut settings = Settings {
        masks: vec![first, second],
        ..Default::default()
    };
    let ordered = engine::render(&base, &settings, &|| false).unwrap().image;
    settings.masks.reverse();
    assert_ne!(
        ordered,
        engine::render(&base, &settings, &|| false).unwrap().image
    );
    settings.masks[0]
        .adjustments
        .insert("monochrome".into(), 1.);
    let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
    assert_eq!(settings, restored);
    let preview = engine::render(&base, &restored, &|| false).unwrap().image;
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("masked.png");
    engine::export(&base, &restored, &output, "png").unwrap();
    let exported = image::open(output).unwrap().into_rgba16();
    assert!(
        exported
            .as_raw()
            .iter()
            .zip(preview.as_raw())
            .all(|(a, b)| a.abs_diff(u16::from(*b) * 257) <= 129)
    );
    settings.masks[0]
        .adjustments
        .insert("monochrome".into(), 0.5);
    assert!(settings.validate().is_err());
}
#[test]
fn brush_rasterization_matches_geometry_and_supports_erase() {
    let mut mask = linear_mask();
    let shape = &mut mask.shapes[0];
    shape.kind = "brush".into();
    shape.radius = 0.15;
    shape.points = vec![[0.1, 0.2], [0.9, 0.6]];
    let base = fixture();
    let weights = mask.weights(&base.pixels, [0., 0., 1., 1.], 40. / 30.);
    for y in 0..30 {
        for x in 0..40 {
            let expected = mask.weight(
                (x as f32 + 0.5) / 40.,
                (y as f32 + 0.5) / 30.,
                [0.; 3],
                40. / 30.,
            );
            assert!((weights[(y * 40 + x) as usize] - expected).abs() < 1e-5);
        }
    }
    let mut erased = mask.shapes[0].clone();
    erased.operation = "subtract".into();
    mask.shapes.push(erased);
    assert!(
        mask.weights(&base.pixels, [0., 0., 1., 1.], 40. / 30.)
            .iter()
            .all(|value| *value <= 0.251)
    );
}
#[test]
fn sharpening_detail_does_not_change_texture_when_sharpening_is_off() {
    let base = fixture();
    let settings = Settings {
        texture: 70.,
        clarity: 30.,
        ..Default::default()
    };
    let first = engine::render(&base, &settings, &|| false).unwrap().image;
    let second = engine::render(
        &base,
        &Settings {
            sharpen_detail: 100.,
            ..settings
        },
        &|| false,
    )
    .unwrap()
    .image;
    assert_eq!(first, second);
}
#[test]
fn constrain_crop_removes_transparent_transform_borders() {
    let output = engine::render(
        &fixture(),
        &Settings {
            angle: 30.,
            perspective_h: 40.,
            constrain_crop: true,
            ..Default::default()
        },
        &|| false,
    )
    .unwrap();
    assert!(output.image.width() < 40 || output.image.height() < 30);
    assert!(output.image.pixels().all(|pixel| pixel[3] == 255));
    assert!((output.image.width() as f32 - output.image.height() as f32 * 4. / 3.).abs() <= 1.);
}

#[test]
fn straightening_keeps_the_crop_dimensions_and_fills_every_corner() {
    for (width, height) in [(80, 120), (120, 80), (80, 80)] {
        let mut base = fixture();
        base.pixels = Rgba32FImage::from_pixel(width, height, Rgba([0.2, 0.3, 0.4, 1.]));
        for quarter_turns in [0, 1] {
            for crop in [[0., 0., 1., 1.], [0.1, 0.2, 0.75, 0.6]] {
                let settings = Settings {
                    quarter_turns,
                    crop,
                    ..Default::default()
                };
                let dimensions = engine::render(&base, &settings, &|| false)
                    .unwrap()
                    .image
                    .dimensions();
                for angle in [-45., -30., -0.01, 0., 0.01, 15., 45.] {
                    for constrain_crop in [false, true] {
                        let output = engine::render(
                            &base,
                            &Settings {
                                angle,
                                constrain_crop,
                                ..settings.clone()
                            },
                            &|| false,
                        )
                        .unwrap()
                        .image;
                        assert_eq!(
                            output.dimensions(),
                            dimensions,
                            "{width}x{height}, {angle}, {quarter_turns}, {crop:?}, {constrain_crop}"
                        );
                        assert!(
                            output.pixels().all(|pixel| pixel[3] == 255),
                            "transparent border at {width}x{height}, {angle}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn straightening_preserves_source_transparency_in_preview_and_export() {
    let mut base = fixture();
    for pixel in base.pixels.pixels_mut() {
        pixel[3] = 0.5;
    }
    let settings = Settings {
        angle: 45.,
        crop: [0.1, 0.2, 0.75, 0.6],
        ..Default::default()
    };
    let preview = engine::render(&base, &settings, &|| false).unwrap().image;
    assert!(preview.pixels().all(|pixel| pixel[3] == 128));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("straightened.png");
    engine::export(&base, &settings, &path, "png").unwrap();
    let exported = image::open(path).unwrap().to_rgba16();
    assert_eq!(exported.dimensions(), preview.dimensions());
    assert!(exported.pixels().all(|pixel| pixel[3] == 32768));
}
#[test]
fn full_resolution_cache_can_exceed_preview_bound() {
    let dir = tempfile::tempdir().unwrap();
    let mut base = fixture();
    base.pixels = Rgba32FImage::from_pixel(5000, 2, Rgba([0.2, 0.3, 0.4, 1.]));
    base.info.width = 5000;
    base.info.height = 2;
    base.info.source_width = 5000;
    base.info.source_height = 2;
    let path = dir.path().join("full.tif");
    engine::write_base(&path, &base).unwrap();
    let restored = engine::read_base(&path, base.info).unwrap();
    assert_eq!(restored.pixels.width(), 5000);
}
#[test]
fn export_size_and_quality_are_validated_without_upscaling() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("small.png");
    engine::export_with_options(
        &fixture(),
        &Settings::default(),
        &output,
        "png",
        &engine::ExportOptions {
            quality: 90,
            long_edge: 20,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(image::image_dimensions(&output).unwrap(), (20, 15));
    let invalid = dir.path().join("invalid.jpg");
    assert!(
        engine::export_with_options(
            &fixture(),
            &Settings::default(),
            &invalid,
            "jpeg",
            &engine::ExportOptions {
                quality: 0,
                long_edge: 20,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!invalid.exists());
}

#[test]
fn export_borders_preserve_the_photo_and_center_it_in_white_or_black() {
    let directory = tempfile::tempdir().unwrap();
    for quarter_turns in [0, 1] {
        let settings = Settings {
            quarter_turns,
            ..Default::default()
        };
        let reference = directory
            .path()
            .join(format!("reference-{quarter_turns}.png"));
        engine::export(&fixture(), &settings, &reference, "png").unwrap();
        let reference = image::open(reference).unwrap().to_rgba16();
        for ratio in [0., 1., 0.8, 16. / 9.] {
            for (color, value) in [
                (engine::BorderColor::White, 65535),
                (engine::BorderColor::Black, 0),
            ] {
                for format in ["jpeg", "png", "tiff"] {
                    let path = directory
                        .path()
                        .join(format!("{quarter_turns}-{ratio}-{value}.{format}"));
                    let options = engine::ExportOptions {
                        border: Some(engine::ExportBorder {
                            color,
                            aspect_ratio: ratio,
                        }),
                        ..Default::default()
                    };
                    engine::export_with_options(&fixture(), &settings, &path, format, &options)
                        .unwrap();
                    let output = image::open(path).unwrap().to_rgba16();
                    let left = (output.width() - reference.width()) / 2;
                    let top = (output.height() - reference.height()) / 2;
                    assert!(
                        left >= 2 && top >= 2,
                        "all four sides need a visible margin"
                    );
                    let expected_ratio = if ratio == 0. {
                        reference.width() as f64 / reference.height() as f64
                    } else {
                        ratio
                    };
                    assert!(
                        (output.width() as f64 - output.height() as f64 * expected_ratio).abs()
                            <= 2.
                    );
                    if format != "jpeg" {
                        assert_eq!(output.get_pixel(0, 0), &Rgba([value, value, value, 65535]));
                        assert_eq!(
                            output.get_pixel(output.width() - 1, output.height() - 1),
                            &Rgba([value, value, value, 65535])
                        );
                        assert_eq!(
                            image::imageops::crop_imm(
                                &output,
                                left,
                                top,
                                reference.width(),
                                reference.height()
                            )
                            .to_image(),
                            reference
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn export_border_limits_apply_to_finished_canvas_and_invalid_options_create_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let old: engine::ExportOptions =
        serde_json::from_str(r#"{"quality":92,"long_edge":0,"max_bytes":0}"#).unwrap();
    assert!(old.border.is_none());
    let mut options = engine::ExportOptions {
        long_edge: 20,
        border: Some(engine::ExportBorder {
            color: engine::BorderColor::Black,
            aspect_ratio: 1.,
        }),
        ..Default::default()
    };
    let output = directory.path().join("limited.png");
    engine::export_with_options(&fixture(), &Settings::default(), &output, "png", &options)
        .unwrap();
    assert_eq!(image::image_dimensions(&output).unwrap(), (20, 20));
    for ratio in [-1., 0.1, 3., f64::NAN, f64::INFINITY] {
        options.border.as_mut().unwrap().aspect_ratio = ratio;
        let invalid = directory.path().join("invalid.png");
        assert!(
            engine::export_with_options(
                &fixture(),
                &Settings::default(),
                &invalid,
                "png",
                &options
            )
            .is_err()
        );
        assert!(!invalid.exists());
    }
    options.border.as_mut().unwrap().aspect_ratio = 1.;
    options.long_edge = 2;
    assert!(
        engine::export_with_options(
            &fixture(),
            &Settings::default(),
            &directory.path().join("tiny.png"),
            "png",
            &options
        )
        .is_err()
    );
    options.long_edge = 20;
    options.max_bytes = 1;
    for format in ["jpeg", "png", "tiff"] {
        let path = directory.path().join(format!("oversized.{format}"));
        assert!(
            engine::export_with_options(&fixture(), &Settings::default(), &path, format, &options)
                .is_err()
        );
        assert!(!path.exists());
    }
}

#[test]
fn contrast_preserves_exposed_white_and_does_not_cap_hdr_luminance() {
    let mut base = fixture();
    // +5 EV makes the first patch white and the second exceed the tone LUT's range.
    base.pixels = Rgba32FImage::from_fn(2, 1, |x, _| {
        if x == 0 {
            Rgba([1. / 32., 1. / 32., 1. / 32., 1.])
        } else {
            Rgba([1., 0.8, 0.1, 1.])
        }
    });
    for amount in [-100., -50., 50., 100.] {
        let settings = Settings {
            exposure: 5.,
            contrast: amount,
            ..Default::default()
        };
        let output = engine::render(&base, &settings, &|| false).unwrap();
        let exposed = engine::render(
            &base,
            &Settings {
                contrast: 0.,
                ..settings
            },
            &|| false,
        )
        .unwrap();
        for x in 0..2 {
            assert_eq!(
                &output.image.get_pixel(x, 0).0[..3],
                &[255; 3],
                "contrast {amount}, patch {x}"
            );
            assert_eq!(output.image.get_pixel(x, 0), exposed.image.get_pixel(x, 0));
        }
    }
}

#[test]
fn contrast_is_monotonic_and_preserves_black_white_and_perceptual_midpoint() {
    let mut base = fixture();
    base.pixels = Rgba32FImage::from_fn(257, 1, |x, _| {
        let value = x as f32 / 256.;
        let linear = if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        };
        Rgba([linear, linear, linear, 1.])
    });
    for amount in [-100., -50., 0., 50., 100.] {
        let output = engine::render(
            &base,
            &Settings {
                contrast: amount,
                ..Default::default()
            },
            &|| false,
        )
        .unwrap()
        .image;
        assert_eq!(output.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(output.get_pixel(256, 0).0, [255; 4]);
        assert!((output.get_pixel(128, 0).0[0] as i16 - 128).abs() <= 1);
        for x in 1..257 {
            assert!(output.get_pixel(x, 0)[0] >= output.get_pixel(x - 1, 0)[0]);
        }
        if amount < 0. {
            assert!(output.get_pixel(64, 0)[0] > 64 && output.get_pixel(192, 0)[0] < 192);
        }
        if amount > 0. {
            assert!(output.get_pixel(64, 0)[0] < 64 && output.get_pixel(192, 0)[0] > 192);
        }
    }
}

#[test]
fn raster_metadata_reference_is_neutral_and_survives_recipe_storage() {
    let base = fixture();
    let original = engine::render(&base, &Settings::default(), &|| false)
        .unwrap()
        .image;
    for reference in [2800., 5200., 10000.] {
        let settings = Settings {
            raster_temperature: Some(reference),
            raster_reference_temperature: Some(reference),
            ..Default::default()
        };
        let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, settings);
        let neutral = engine::render(&base, &restored, &|| false).unwrap().image;
        for (actual, expected) in neutral.as_raw().iter().zip(original.as_raw()) {
            assert!(
                actual.abs_diff(*expected) <= 1,
                "Metadata reference reapplied WB"
            );
        }
        let changed = Settings {
            raster_temperature: Some(reference * 1.5),
            ..restored
        };
        assert_ne!(
            engine::render(&base, &changed, &|| false).unwrap().image,
            neutral
        );
    }
    for invalid in [f32::NAN, 1999., 60001.] {
        assert!(
            Settings {
                raster_reference_temperature: Some(invalid),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}

#[cfg(all(target_os = "macos", feature = "rtengine"))]
#[test]
fn jpeg_makernote_temperature_ignores_unused_auto_wb_values_and_bad_metadata() {
    use image::ImageEncoder;
    fn exif(mode: u16, kelvin: u16) -> Vec<u8> {
        let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
        fn entry(bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(kind.to_le_bytes());
            bytes.extend(count.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(2u16.to_le_bytes());
        entry(&mut bytes, 271, 2, 9, 38);
        entry(&mut bytes, 34665, 4, 1, 48);
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(b"FUJIFILM\0\0");
        bytes.extend(1u16.to_le_bytes());
        entry(&mut bytes, 37500, 7, 42, 66);
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(b"FUJIFILM");
        bytes.extend(12u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        entry(&mut bytes, 0x1002, 3, 1, u32::from(mode));
        entry(&mut bytes, 0x1005, 3, 1, u32::from(kelvin));
        bytes.extend(0u32.to_le_bytes());
        bytes
    }
    let directory = tempfile::tempdir().unwrap();
    for (name, metadata, expected) in [
        ("manual", exif(0xff0, 5200), Some(5200.)),
        ("auto", exif(0, 5200), None),
        ("invalid", exif(0xff0, 65535), None),
        ("damaged", b"broken exif".to_vec(), None),
    ] {
        let path = directory.path().join(format!("{name}.jpg"));
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new(std::fs::File::create(&path).unwrap());
        encoder.set_exif_metadata(metadata).unwrap();
        encoder
            .encode(&[128; 40 * 30 * 3], 40, 30, image::ExtendedColorType::Rgb8)
            .unwrap();
        assert_eq!(
            advanced_explorer_rtengine::raster_temperature(&path).unwrap(),
            expected
        );
        if name != "damaged" {
            let base = engine::prepare(&path, 1600).unwrap();
            assert_eq!(base.info.raster_wb_temperature, expected);
            let cache = directory.path().join(format!("{name}.tif"));
            engine::write_base(&cache, &base).unwrap();
            assert_eq!(
                engine::read_base(&cache, base.info)
                    .unwrap()
                    .info
                    .raster_wb_temperature,
                expected
            );
        }
    }
}

#[test]
fn raster_kelvin_is_wide_range_preserves_alpha_and_resets_to_baked_wb() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("neutral.png");
    image::RgbaImage::from_pixel(40, 30, Rgba([128, 128, 128, 128]))
        .save(&source)
        .unwrap();
    let base = engine::prepare(&source, 1600).unwrap();
    let render = |settings: &Settings| engine::render(&base, settings, &|| false).unwrap().image;
    let original = render(&Settings::default());
    let mut settings = Settings {
        raster_temperature: Some(6504.),
        ..Default::default()
    };
    let reference = render(&settings);
    for (actual, expected) in reference.as_raw().iter().zip(original.as_raw()) {
        assert!(actual.abs_diff(*expected) <= 1);
    }
    for kelvin in [2000., 60000.] {
        settings.raster_temperature = Some(kelvin);
        let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, settings);
        let corrected = render(&settings);
        assert_ne!(corrected, original);
        assert!(corrected.pixels().all(|pixel| pixel[3] == 128));
        let pixel = corrected.get_pixel(20, 15);
        if kelvin < 6504. {
            assert!(pixel[2] > pixel[0]);
        } else {
            assert!(pixel[0] > pixel[2]);
        }
        let legacy = Settings {
            temperature: 100.,
            tint: -100.,
            ..settings.clone()
        };
        assert_eq!(
            corrected,
            render(&legacy),
            "Kelvin replaces, not stacks with, relative WB"
        );
    }
    settings.raster_temperature = None;
    assert_eq!(render(&settings), original);
    for invalid in [f32::NAN, 1999., 60001.] {
        settings.raster_temperature = Some(invalid);
        assert!(settings.validate().is_err());
    }
    settings.raster_temperature = Some(6504.);
    settings.raster_tint = 151.;
    assert!(settings.validate().is_err());
}

#[test]
fn raw_kelvin_survives_catalog_and_float_cache_without_changing_sensor_samples() {
    let mut base = fixture();
    base.info.raw = Some(advanced_explorer_media::develop_input::RawColor {
        camera: "OpenRoom regression camera".into(),
        neutral: [0.5, 1., 0.7],
        xyz_to_camera: [[0.7, -0.1, -0.1], [-0.4, 1.2, 0.3], [-0.1, 0.2, 0.7]],
        baseline_exposure: 0.,
        calibration: [[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]; 2],
        calibration_signature: String::new(),
        analog_balance: [1.; 3],
        as_shot: [5000., 10.],
        profile_signature: String::new(),
    });
    base.info.source_kind = "raw-camera-linear".into();
    let settings = Settings {
        raw_temperature: Some(4200.),
        raw_tint: 22.,
        exposure: 0.7,
        contrast: -30.,
        ..Default::default()
    };
    let restored_settings: Settings =
        serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
    assert_eq!(restored_settings, settings);
    let pixels = base.pixels.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.tif");
    engine::write_base(&path, &base).unwrap();
    let restored = engine::read_base(&path, base.info.clone()).unwrap();
    assert_eq!(restored.pixels, pixels);
    let mut damaged = base.info.clone();
    damaged.raw.as_mut().unwrap().calibration[0] = [[0.; 3]; 3];
    assert!(engine::read_base(&path, damaged).is_err());
    let mut damaged = base.info.clone();
    damaged.raw.as_mut().unwrap().neutral[0] = f32::NAN;
    assert!(engine::read_base(&path, damaged).is_err());
    assert_eq!(
        engine::render(&base, &settings, &|| false).unwrap().image,
        engine::render(&restored, &restored_settings, &|| false)
            .unwrap()
            .image
    );
    assert_eq!(base.pixels, pixels);
    let raster_settings = Settings {
        raster_temperature: Some(2000.),
        ..settings.clone()
    };
    assert_eq!(
        engine::render(&base, &settings, &|| false).unwrap().image,
        engine::render(&base, &raster_settings, &|| false)
            .unwrap()
            .image,
        "Raster reference WB must not affect RAW"
    );
    assert_eq!(Settings::default().raw_temperature, None);
    for kelvin in [2000., 60000.] {
        let settings = Settings {
            raw_temperature: Some(kelvin),
            ..Default::default()
        };
        let restored: Settings = serde_json::from_str(&settings.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, settings);
    }
    for invalid in [f32::NAN, 1999., 60001.] {
        assert!(
            Settings {
                raw_temperature: Some(invalid),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Settings {
            raw_tint: 151.,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn export_byte_limit_reduces_jpeg_quality_and_removes_oversized_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let mut base = fixture();
    base.pixels = Rgba32FImage::from_fn(64, 48, |x, y| {
        Rgba([
            ((x * 13 + y * 31) % 101) as f32 / 100.,
            ((x * 47 + y * 7) % 101) as f32 / 100.,
            ((x * 3 + y * 59) % 101) as f32 / 100.,
            1.,
        ])
    });
    let reference = dir.path().join("reference.jpg");
    let mut options = engine::ExportOptions {
        quality: 30,
        ..Default::default()
    };
    engine::export_with_options(&base, &Settings::default(), &reference, "jpeg", &options).unwrap();
    options.max_bytes = std::fs::metadata(&reference).unwrap().len();
    options.quality = 95;
    let limited = dir.path().join("limited.jpg");
    engine::export_with_options(&base, &Settings::default(), &limited, "jpeg", &options).unwrap();
    assert!(std::fs::metadata(&limited).unwrap().len() <= options.max_bytes);
    assert_eq!(image::image_dimensions(&limited).unwrap(), (64, 48));
    for format in ["jpeg", "png", "tiff"] {
        let output = dir.path().join(format!("too-small.{format}"));
        options.max_bytes = 1;
        options.quality = 1;
        assert!(
            engine::export_with_options(&base, &Settings::default(), &output, format, &options)
                .is_err()
        );
        assert!(!output.exists(), "oversized {format} must not remain");
        std::fs::write(&output, b"original").unwrap();
        assert!(
            engine::export_with_options(&base, &Settings::default(), &output, format, &options)
                .is_err()
        );
        assert_eq!(std::fs::read(output).unwrap(), b"original");
    }
}

#[test]
fn detail_pixels_match_with_four_and_eight_render_threads() {
    let mut working = fixture();
    working.pixels = Rgba32FImage::from_fn(137, 1301, |x, y| {
        Rgba([x as f32 / 150., (y % 97) as f32 / 100., 0.2, 0.7])
    });
    working.info.width = 137;
    working.info.height = 1301;
    working.info.source_width = 137;
    working.info.source_height = 1301;
    let settings = Settings {
        texture: 15.,
        clarity: 10.,
        ..Default::default()
    };
    let render = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                engine::render(&working, &settings, &|| false)
                    .unwrap()
                    .image
            })
    };
    assert_eq!(render(4), render(8));
}
