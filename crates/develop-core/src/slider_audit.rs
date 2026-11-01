//! Per-control regression audit: active prerequisites, serialization, pixels and CPU/Metal parity.
use super::*;
use serde_json::{Value, json};

fn mask() -> Value {
    json!({"name":"Audit", "enabled":true, "invert":false,"amount":1,
        "adjustments":{},"shapes":[{"kind":"brush","operation":"add","start":[0.2,0.2],
        "end":[0.8,0.8],"radius":0.2,"feather":0.5,"opacity":1,
        "points":[[0.2,0.2],[0.5,0.5],[0.8,0.8]],"color":[0.5,0.5,0.5],"range":[0,1]}]})
}
fn cases() -> Vec<(String, Value, Value)> {
    let mut cases = Vec::new();
    let mut scalar = |key: &str, min: f32, max: f32, parent: Value| {
        for value in [min, max] {
            let mut after = parent.clone();
            after[key] = json!(value);
            cases.push((format!("{key}={value}"), parent.clone(), after));
        }
    };
    scalar("exposure", -5., 5., json!({}));
    scalar("temperature", -100., 100., json!({}));
    scalar("tint", -100., 100., json!({}));
    scalar(
        "raster_temperature",
        2000.,
        60000.,
        json!({"raster_temperature":6504}),
    );
    scalar(
        "raster_tint",
        -150.,
        150.,
        json!({"raster_temperature":6504}),
    );
    for key in [
        "contrast",
        "highlights",
        "shadows",
        "whites",
        "blacks",
        "texture",
        "clarity",
        "dehaze",
        "vibrance",
        "perspective_h",
        "perspective_v",
        "distortion",
        "aspect",
        "offset_x",
        "offset_y",
        "vignette",
        "ca_red",
        "ca_blue",
    ] {
        scalar(key, -100., 100., json!({}));
    }
    scalar("saturation", -100., 300., json!({}));
    scalar("angle", -45., 45., json!({}));
    scalar("scale", 50., 200., json!({}));
    scalar("sharpen", 0., 150., json!({"sharpen":50}));
    for key in ["noise_luma", "noise_color"] {
        scalar(key, 0., 100., json!({key:30}));
    }
    scalar("sharpen_radius", 0.5, 3., json!({"sharpen":100}));
    for key in ["sharpen_detail", "sharpen_masking"] {
        scalar(key, 0., 100., json!({"sharpen":100,key:50}));
    }
    scalar("defringe_radius", 0.5, 5., json!({"defringe":true}));
    scalar("defringe_threshold", 0., 100., json!({"defringe":true}));
    for key in ["vignette_midpoint", "vignette_feather"] {
        scalar(key, 0., 100., json!({"vignette":-60}));
    }
    scalar("grain", 0., 100., json!({"grain":40}));
    for key in ["grain_size", "grain_roughness"] {
        scalar(key, 0., 100., json!({"grain":50}));
    }
    let grades = json!({"grading":[[220,40,0],[100,10,0],[40,30,0],[0,0,0]]});
    scalar("grading_balance", -100., 100., grades.clone());
    scalar("grading_blending", 0., 100., grades);
    for group in ["mixer", "grading", "curves"] {
        let channels = if group == "mixer" { 8 } else { 4 };
        let components = if group == "curves" { 5 } else { 3 };
        for channel in 0..channels {
            for component in 0..components {
                let mut before = serde_json::to_value(Settings::default()).unwrap();
                let (min, max) = match group {
                    "grading" => {
                        before[group][channel] = json!([180, 35, 20]);
                        if component == 0 {
                            (0., 360.)
                        } else if component == 1 {
                            (0., 100.)
                        } else {
                            (-100., 100.)
                        }
                    }
                    "curves" => (0., 1.),
                    _ => (-100., 100.),
                };
                for value in [min, max] {
                    if before[group][channel][component].as_f64() == Some(value) {
                        continue;
                    }
                    let mut after = before.clone();
                    after[group][channel][component] = json!(value);
                    cases.push((
                        format!("{group}[{channel}][{component}]={value}"),
                        before.clone(),
                        after,
                    ));
                }
            }
        }
    }
    for component in 0..4 {
        let before = json!({"crop":[0.1,0.1,0.5,0.5]});
        for value in if component < 2 {
            [0., 0.5]
        } else {
            [0.01, 0.9]
        } {
            let mut after = before.clone();
            after["crop"][component] = json!(value);
            cases.push((format!("crop[{component}]={value}"), before.clone(), after));
        }
    }
    for key in masks::LOCAL_KEYS.iter().filter(|key| **key != "monochrome") {
        let before = json!({"masks":[mask()]});
        let value = if *key == "exposure" { 1.5 } else { 65. };
        let mut after = before.clone();
        after["masks"][0]["adjustments"][key] = json!(value);
        cases.push((format!("local-{key}"), before, after));
    }
    let mut before = json!({"masks":[mask()]});
    before["masks"][0]["adjustments"] = json!({"exposure":1.5});
    for (key, min, max) in [
        ("radius", 0.005, 0.5),
        ("feather", 0., 1.),
        ("amount", 0., 0.5),
    ] {
        for value in [min, max] {
            let mut after = before.clone();
            if key == "amount" {
                after["masks"][0][key] = json!(value);
            } else {
                after["masks"][0]["shapes"][0][key] = json!(value);
            }
            cases.push((format!("mask-{key}={value}"), before.clone(), after));
        }
    }
    cases
}

#[test]
#[ignore = "runs the complete slider pixel audit including the native engine and Metal"]
fn every_slider_changes_pixels_round_trips_and_matches_cpu() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("audit.png");
    // Colored gradients, sharp edges and deterministic noise exercise every mixer band and detail control.
    image::RgbaImage::from_fn(768, 512, |x, y| {
        let n = (x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263)).wrapping_mul(1274126177);
        image::Rgba([(x % 256) as u8, (y % 256) as u8, (n >> 16) as u8, 255])
    })
    .save(&source)
    .unwrap();
    let working = prepare(&source, 0).unwrap();
    let original = working.pixels.clone();
    if let Some(report) = std::env::var_os("AE_SLIDER_CASES") {
        std::fs::write(report, serde_json::to_vec_pretty(&cases()).unwrap()).unwrap();
    }
    let mut failures = Vec::new();
    let mut count = 0;
    for (name, before, after) in cases() {
        count += 1;
        let before: Settings = serde_json::from_value(before).unwrap();
        let after: Settings = serde_json::from_value(after).unwrap();
        before.validate().unwrap();
        after.validate().unwrap();
        let restored: Settings = serde_json::from_str(&after.catalog_recipe().unwrap()).unwrap();
        assert_eq!(restored, after, "{name}: saved recipe changed");
        let baseline =
            preview_pixels(&process(false, Cow::Borrowed(&working), &before, &|| false).unwrap());
        let cpu =
            preview_pixels(&process(false, Cow::Borrowed(&working), &after, &|| false).unwrap());
        let gpu_float = process(true, Cow::Borrowed(&working), &after, &|| false).unwrap();
        assert!(
            gpu_float.as_raw().iter().all(|v| v.is_finite()),
            "{name}: non-finite result"
        );
        let gpu = preview_pixels(&gpu_float);
        assert_eq!(cpu.dimensions(), gpu.dimensions(), "{name}");
        let error = cpu
            .as_raw()
            .iter()
            .zip(gpu.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        if cpu == baseline {
            failures.push(format!("{name}: no pixel effect"));
        }
        if error > 1 {
            failures.push(format!("{name}: CPU/Metal error {error}/255"));
        }
        println!("{name}: changed={} max_gpu_error={error}", cpu != baseline);
    }
    assert_eq!(working.pixels, original, "the source buffer changed");
    assert!(
        failures.is_empty(),
        "{} failures in {count} slider cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!("{count} slider pixel/serialization/parity cases passed");
}
