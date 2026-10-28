use advanced_explorer_develop::{self as engine, BaseInfo, Settings, WorkingImage};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    rayon::ThreadPoolBuilder::new().num_threads(4).build()?.install(|| -> anyhow::Result<()> {
        let mut base = WorkingImage {
            pixels: image::Rgba32FImage::from_fn(1600, 1067, |x, y| {
                image::Rgba([x as f32 / 2000., y as f32 / 1500., 0.2, 1.])
            }),
            info: BaseInfo {
            auto_ca_supported: None,
            camera_saturation: None,
            native_source: None,
            raw: None,
            raster_wb_temperature: None,
                width: 1600, height: 1067, source_width: 1600, source_height: 1067,
                source_kind: "synthetic".into(), color_note: "linear sRGB".into(),
            },
        };
        let mut results = Vec::new();
        for (label, kind) in [("previous_math", "synthetic"), ("calibrated_raster", "raster-display-linear")] {
            base.info.source_kind = kind.into();
            for (name, settings) in [
                ("neutral", Settings::default()),
                ("wb_tone", Settings { temperature: 22., tint: 18., exposure: 1.4, contrast: -65., ..Default::default() }),
            ] {
                let mut samples = Vec::new();
                for iteration in 0..21 {
                    let start = Instant::now();
                    std::hint::black_box(engine::render(&base, &settings, &|| false)?);
                    if iteration > 0 { samples.push(start.elapsed().as_secs_f64() * 1000.); }
                }
                samples.sort_by(f64::total_cmp);
                results.push(serde_json::json!({"path": label, "settings": name, "median_ms": samples[10], "p95_ms": samples[19]}));
            }
        }
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({"threads":4, "dimensions":[1600,1067], "samples":20, "results":results}))?);
        Ok(())
    })
}
