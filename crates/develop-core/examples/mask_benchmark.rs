//! Deterministic mask workload: compare release builds, including output hashes.
use advanced_explorer_develop::{
    self as engine, BaseInfo, Settings, WorkingImage,
    masks::{Mask, Shape},
};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    time::Instant,
};

fn measure<T>(mut operation: impl FnMut() -> T) -> serde_json::Value {
    std::hint::black_box(operation());
    let mut times = Vec::new();
    for _ in 0..11 {
        let start = Instant::now();
        std::hint::black_box(operation());
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    serde_json::json!({"median_ms":times[5], "p95_ms":times[10]})
}

fn main() -> anyhow::Result<()> {
    let single_case = std::env::var_os("AE_MASK_SINGLE_CASE").is_some();
    let benchmark = || -> anyhow::Result<()> {
        let mut results = Vec::new();
        for (width, height) in [(1600, 1067), (6000, 4000)] {
            if single_case && width != 6000 {
                continue;
            }
            let base = WorkingImage {
                pixels: image::Rgba32FImage::from_fn(width, height, |x, y| {
                    image::Rgba([
                        (x % 193) as f32 / 200.,
                        (y % 191) as f32 / 200.,
                        ((x + y) % 197) as f32 / 200.,
                        0.8,
                    ])
                }),
                info: BaseInfo {
                    auto_ca_supported: None,
                    camera_saturation: None,
                    native_source: None,
                    raw: None,
                    raster_wb_temperature: None,
                    source_width: 6000,
                    source_height: 4000,
                    width,
                    height,
                    source_kind: "synthetic".into(),
                    color_note: "linear sRGB".into(),
                },
            };
            for kind in ["linear", "radial", "brush"] {
                if single_case && kind != "linear" {
                    continue;
                }
                let shape = Shape {
                    kind: kind.into(),
                    operation: "add".into(),
                    start: [0.3, 0.3],
                    end: [0.7, 0.7],
                    radius: 0.06,
                    feather: 0.5,
                    opacity: 0.8,
                    points: vec![[0.2, 0.3], [0.8, 0.5]],
                    color: [0.5; 3],
                    range: [0.2, 0.8],
                };
                let mask = Mask {
                    name: kind.into(),
                    enabled: true,
                    invert: false,
                    amount: 0.8,
                    shapes: vec![shape],
                    adjustments: BTreeMap::from([("exposure".into(), 0.7)]),
                };
                let weights = measure(|| {
                    mask.weights(&base.pixels, [0., 0., 1., 1.], width as f32 / height as f32)
                });
                for detail in [false, true] {
                    if single_case && detail {
                        continue;
                    }
                    let mut settings = Settings {
                        masks: vec![mask.clone()],
                        ..Default::default()
                    };
                    if detail {
                        settings.masks[0].adjustments.insert("texture".into(), 30.);
                    }
                    let render = measure(|| engine::render(&base, &settings, &|| false).unwrap());
                    let frame = engine::render(&base, &settings, &|| false)?;
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    frame.image.as_raw().hash(&mut hash);
                    results.push(serde_json::json!({"dimensions":[width,height],"mask":kind,"detail":detail,
                        "weights":weights,"render":render,"pixel_hash":format!("{:016x}",hash.finish())}));
                }
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"threads":4,"samples":11,"results":results})
            )?
        );
        Ok(())
    };
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()?
        .install(benchmark)
}
