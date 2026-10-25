//! Compare complete preview stages on the same real input before/after changes.
use advanced_explorer_develop::{self as engine, Settings};
use image::{
    ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    time::Instant,
};

fn measure<T>(count: usize, mut operation: impl FnMut() -> T) -> serde_json::Value {
    std::hint::black_box(operation());
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let start = Instant::now();
        std::hint::black_box(operation());
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    serde_json::json!({"median_ms": times[count/2], "p95_ms": times[(count*95/100).min(count-1)]})
}
fn main() -> anyhow::Result<()> {
    let source = PathBuf::from(std::env::args_os().nth(1).expect("input photo required"));
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()?
        .install(|| benchmark(source))
}

fn benchmark(source: PathBuf) -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let base = engine::prepare(&source, 4096)?;
    let path = directory.path().join("base.tif");
    let prepare = measure(5, || engine::prepare(&source, 4096).unwrap());
    let write = measure(5, || {
        let _ = std::fs::remove_file(&path);
        engine::write_base(&path, &base).unwrap();
    });
    let read = measure(5, || engine::read_base(&path, base.info.clone()).unwrap());
    let typical = Settings {
        exposure: 0.5,
        contrast: 12.,
        highlights: -25.,
        shadows: 20.,
        vibrance: 15.,
        ..Default::default()
    };
    let mut curve = typical.clone();
    curve.point_curves[0] = vec![[0., 0.], [0.25, 0.35], [0.75, 0.85], [1., 1.]];
    curve.point_curves[1] = vec![[0., 0.], [0.5, 0.6], [1., 1.]];
    let mut previews = Vec::new();
    for bound in [384, 1600, 4096] {
        let mut working = engine::read_base(&path, base.info.clone())?;
        working.pixels = engine::resize_linear(working.pixels, bound)?;
        working.info.width = working.pixels.width();
        working.info.height = working.pixels.height();
        let mut renders = Vec::new();
        for (name, settings) in [
            ("neutral", Settings::default()),
            ("tone", typical.clone()),
            ("point_curve", curve.clone()),
            (
                "detail",
                Settings {
                    sharpen: 40.,
                    texture: 15.,
                    clarity: 10.,
                    noise_luma: 15.,
                    ..typical.clone()
                },
            ),
            (
                "geometry",
                Settings {
                    angle: 8.,
                    perspective_h: 20.,
                    ..typical.clone()
                },
            ),
            (
                "grading",
                Settings {
                    grading: [[220., 40., 10.]; 4],
                    ..typical.clone()
                },
            ),
        ] {
            let output = engine::render(&working, &settings, &|| false)?;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            output.image.as_raw().hash(&mut hash);
            renders.push(serde_json::json!({"scenario":name,"render":measure(10,||engine::render(&working,&settings,&||false).unwrap()),"pixel_hash":format!("{:x}",hash.finish())}));
        }
        let output = engine::render(&working, &typical, &|| false)?;
        let mut encoding = Vec::new();
        for (name, compression, filter) in [
            ("default", CompressionType::Default, FilterType::Adaptive),
            ("fast", CompressionType::Fast, FilterType::Adaptive),
        ] {
            let encode = || {
                let mut bytes = Vec::new();
                PngEncoder::new_with_quality(&mut bytes, compression, filter)
                    .write_image(
                        output.image.as_raw(),
                        output.image.width(),
                        output.image.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .unwrap();
                bytes
            };
            let bytes = encode();
            assert_eq!(image::load_from_memory(&bytes)?.into_rgba8(), output.image);
            encoding.push(
                serde_json::json!({"mode":name,"timing":measure(10,encode),"bytes":bytes.len()}),
            );
        }
        let preview_path = directory.path().join("preview.png");
        let complete = measure(10, || {
            let rendered = engine::render(&working, &typical, &|| false).unwrap();
            rendered.image.save(&preview_path).unwrap();
        });
        let interactive = measure(20, || {
            let rendered = engine::render(&working, &curve, &|| false).unwrap();
            let file = std::io::BufWriter::new(std::fs::File::create(&preview_path).unwrap());
            PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Adaptive)
                .write_image(
                    rendered.image.as_raw(),
                    rendered.image.width(),
                    rendered.image.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
        });
        previews.push(serde_json::json!({"bound":bound,"dimensions":output.image.dimensions(),"renders":renders,"png":encoding,"render_and_default_png_write":complete,"curve_render_and_fast_png_write":interactive}));
    }
    let mut exports = Vec::new();
    for (format, extension) in [("jpeg", "jpg"), ("png", "png"), ("tiff", "tif")] {
        let target = directory.path().join(format!("export.{extension}"));
        let full = engine::prepare(&source, 0)?;
        let timing = measure(3, || {
            let _ = std::fs::remove_file(&target);
            engine::export(&full, &typical, &target, format).unwrap();
        });
        exports.push(serde_json::json!({"format":format,"timing":timing,"bytes":std::fs::metadata(target)?.len()}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"threads":4,"source_dimensions":[base.info.source_width,base.info.source_height],"cache_bytes":std::fs::metadata(&path)?.len(),"prepare":prepare,"base_write":write,"base_read":read,"previews":previews,"exports":exports})
        )?
    );
    Ok(())
}
