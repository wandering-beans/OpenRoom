use advanced_explorer_develop::{self as engine, PreviewRenderer, Settings, WorkingImage};
use serde_json::json;

fn compare_slider_sequence(working: WorkingImage) {
    let mut renderer = PreviewRenderer::new(working.clone());
    let mut recipe = json!({"version": 1, "sharpen": 30, "noise_luma": 10});
    for patch in [
        json!({}),
        json!({"contrast": 18}),
        json!({"highlights": -30, "shadows": 20}),
        json!({"exposure": 0.7}),
        json!({"temperature": 12, "tint": -5}),
        json!({"raw_temperature": 4800, "raw_tint": 7}),
        json!({"raster_temperature": 5200, "raster_tint": -10}),
        json!({"sharpen_radius": 1.5, "sharpen_detail": 45, "sharpen_masking": 20}),
        json!({"noise_luma": 20, "noise_color": 15}),
        json!({"texture": 15, "clarity": 10}),
        json!({"saturation": 15, "vibrance": 20}),
        json!({"masks": [{
            "name": "local exposure", "enabled": true, "invert": false, "amount": 0.8,
            "adjustments": {"exposure": 0.5},
            "shapes": [{
                "kind": "linear", "operation": "add", "start": [0.2, 0.3], "end": [0.8, 0.7],
                "radius": 0.1, "feather": 0.5, "opacity": 1.0, "points": [],
                "color": [0.5, 0.5, 0.5], "range": [0.0, 1.0]
            }]
        }]}),
        json!({"contrast": -12, "point_curves": [[[0, 0], [0.5, 0.6], [1, 1]], [], [], []]}),
        json!({"quarter_turns": 1, "crop": [0.1, 0.1, 0.8, 0.7]}),
        json!({"monochrome": true, "vignette": -20, "grain": 10}),
        json!({"sharpen": 0, "noise_luma": 0, "noise_color": 0}),
        json!({"sharpen": 30, "noise_luma": 10, "noise_color": 0}),
    ] {
        recipe
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let settings: Settings = serde_json::from_value(recipe.clone()).unwrap();
        let expected = engine::render(&working, &settings, &|| false).unwrap();
        let actual = renderer.render(&settings, &|| false).unwrap();
        assert_eq!(actual.image, expected.image, "slider patch: {patch}");
        assert_eq!(actual.info.histogram, expected.info.histogram);
        assert_eq!(actual.info.clipped_shadows, expected.info.clipped_shadows);
        assert_eq!(
            actual.info.clipped_highlights,
            expected.info.clipped_highlights
        );
    }
    let settings: Settings = serde_json::from_value(recipe).unwrap();
    assert!(renderer.render(&settings, &|| true).is_err());
    assert!(
        renderer
            .render(
                &Settings {
                    contrast: 101.,
                    ..settings.clone()
                },
                &|| false
            )
            .is_err()
    );
    assert_eq!(
        renderer.render(&settings, &|| false).unwrap().image,
        engine::render(&working, &settings, &|| false)
            .unwrap()
            .image
    );
    assert_eq!(renderer.working().pixels, working.pixels);
}

#[test]
fn cached_previews_match_uncached_sliders_and_preserve_transparency() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("photo.png");
    image::RgbaImage::from_fn(96, 64, |x, y| {
        image::Rgba([
            (x * 2) as u8,
            (y * 3) as u8,
            ((x + y) % 255) as u8,
            (x % 4 * 85) as u8,
        ])
    })
    .save(&source)
    .unwrap();
    compare_slider_sequence(engine::prepare(&source, 1600).unwrap());

    let mut renderer = PreviewRenderer::new(engine::prepare(&source, 1600).unwrap());
    let settings = Settings {
        sharpen: 30.,
        ..Default::default()
    };
    for _ in 0..3 {
        renderer.render(&settings, &|| false).unwrap();
    }
    #[cfg(all(target_os = "macos", feature = "rtengine"))]
    {
        std::fs::remove_file(source).unwrap();
        assert!(
            renderer.render(&settings, &|| false).is_err(),
            "cached native stages must revalidate their source"
        );
    }
}

#[test]
#[ignore = "requires AE_PREVIEW_PHOTO pointing to a RAW sample"]
fn cached_raw_previews_match_uncached_sliders() {
    let source = std::env::var_os("AE_PREVIEW_PHOTO").unwrap();
    compare_slider_sequence(engine::prepare(std::path::Path::new(&source), 1600).unwrap());
}
