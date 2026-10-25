//! Compare lossless preview encodings on a real image, including decoding and byte identity.
use image::{
    ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let source = std::env::args_os()
        .nth(1)
        .expect("input JPEG/PNG/TIFF required");
    let original = image::open(source)?.into_rgba8();
    let mut results = Vec::new();
    for bound in [1600, original.width().max(original.height())] {
        let pixels = image::DynamicImage::ImageRgba8(original.clone())
            .thumbnail(bound, bound)
            .into_rgba8();
        for (name, compression, filter) in [
            ("fast_sub", CompressionType::Fast, FilterType::Sub),
            ("fast_none", CompressionType::Fast, FilterType::NoFilter),
            (
                "uncompressed",
                CompressionType::Uncompressed,
                FilterType::NoFilter,
            ),
        ] {
            let mut samples = Vec::new();
            let mut bytes = Vec::new();
            for iteration in 0..11 {
                bytes.clear();
                let started = Instant::now();
                PngEncoder::new_with_quality(&mut bytes, compression, filter).write_image(
                    pixels.as_raw(),
                    pixels.width(),
                    pixels.height(),
                    image::ExtendedColorType::Rgba8,
                )?;
                let encoded_ms = started.elapsed().as_secs_f64() * 1000.;
                let decoded = image::load_from_memory(&bytes)?.into_rgba8();
                let total_ms = started.elapsed().as_secs_f64() * 1000.;
                assert_eq!(decoded, pixels);
                if iteration > 0 {
                    samples.push((encoded_ms, total_ms));
                }
            }
            samples.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut roundtrips = samples.iter().map(|sample| sample.1).collect::<Vec<_>>();
            roundtrips.sort_by(f64::total_cmp);
            results.push(serde_json::json!({"mode":name,"bound":bound,"bytes":bytes.len(),
                "encode_median_ms":(samples[4].0 + samples[5].0)/2.,"roundtrip_median_ms":(roundtrips[4] + roundtrips[5])/2.}));
        }
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
