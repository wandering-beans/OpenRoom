//! Compare preview cache I/O and lossless PNG filters using one supplied photo.
use advanced_explorer_develop as engine;
use image::{
    ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use std::{path::Path, time::Instant};

fn median_ms(mut run: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<f64> {
    run()?;
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        run()?;
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[3])
}

fn main() -> anyhow::Result<()> {
    let source = std::env::args().nth(1).expect("Pass a sample photo path");
    let temporary = tempfile::tempdir()?;
    let mut cache_results = Vec::new();
    for bound in [4096, 1600, 384] {
        let base = engine::prepare(Path::new(&source), bound)?;
        let cache = temporary.path().join(format!("{bound}.tif"));
        engine::write_base(&cache, &base)?;
        let elapsed = median_ms(|| {
            let loaded = engine::read_base(&cache, base.info.clone())?;
            std::hint::black_box(engine::resize_linear(loaded.pixels, 1600)?);
            Ok(())
        })?;
        cache_results.push(serde_json::json!({
            "bound": bound, "bytes": std::fs::metadata(cache)?.len(),
            "read_and_resize_median_ms": elapsed
        }));
    }
    let base = engine::prepare(Path::new(&source), 1600)?;
    let pixels = engine::render(&base, &Default::default(), &|| false)?.image;
    let mut encoders = Vec::new();
    for filter in [FilterType::Adaptive, FilterType::Sub, FilterType::Paeth] {
        let mut output = Vec::new();
        let elapsed = median_ms(|| {
            output.clear();
            PngEncoder::new_with_quality(&mut output, CompressionType::Fast, filter).write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(())
        })?;
        assert_eq!(image::load_from_memory(&output)?.to_rgba8(), pixels);
        encoders.push(serde_json::json!({
            "filter": format!("{filter:?}"), "bytes": output.len(), "median_ms": elapsed,
            "pixels_equal": true
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "trials": 7, "warmup": 1, "os_caches_flushed": false,
            "preview": [pixels.width(), pixels.height()], "cache": cache_results, "png": encoders
        }))?
    );
    Ok(())
}
