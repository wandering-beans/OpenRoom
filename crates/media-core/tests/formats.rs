use advanced_explorer_media::render;
use image::{ImageFormat, Rgba, RgbaImage};
use std::{fs, path::Path, time::Instant};

#[test]
fn removed_asset_formats_are_rejected_without_writing_output() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["pdf", "svg", "gif", "bmp", "webp", "hdr", "exr"] {
        let source = dir.path().join(format!("asset.{ext}"));
        // Valid TIFF bytes ensure rejection depends on the allowed formats, not corruption.
        RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]))
            .save_with_format(&source, ImageFormat::Tiff)
            .unwrap();
        let original = fs::read(&source).unwrap();
        let destination = dir.path().join("preview.png");
        assert!(
            render(&source, &destination, 384)
                .unwrap_err()
                .to_string()
                .contains("Unsupported")
        );
        assert!(!destination.exists());
        assert_eq!(fs::read(source).unwrap(), original);
    }
}

// Independently constructed Adobe PSD/PSB headers and planar composites.
fn photoshop(version: u16, depth: u16, rle: bool) -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend(version.to_be_bytes());
    bytes.extend([0; 6]);
    bytes.extend(3u16.to_be_bytes());
    bytes.extend(32u32.to_be_bytes());
    bytes.extend(64u32.to_be_bytes());
    bytes.extend(depth.to_be_bytes());
    bytes.extend(3u16.to_be_bytes());
    bytes.extend([0; 8]); // empty color data and image resources
    bytes.extend(vec![0; if version == 2 { 8 } else { 4 }]); // no layers
    bytes.extend(u16::from(rle).to_be_bytes());
    if rle {
        for _ in 0..3 * 32 {
            if version == 2 {
                bytes.extend(2u32.to_be_bytes())
            } else {
                bytes.extend(2u16.to_be_bytes())
            }
        }
        for value in [240, 30, 60] {
            for _ in 0..32 {
                bytes.extend([193, value]);
            }
        }
    } else {
        for value in [240u8, 30, 60] {
            for _ in 0..64 * 32 {
                match depth {
                    8 => bytes.push(value),
                    16 => bytes.extend((u16::from(value) * 257).to_be_bytes()),
                    32 => bytes.extend((f32::from(value) / 255.).to_be_bytes()),
                    _ => unreachable!(),
                }
            }
        }
    }
    bytes
}
fn check(source: &Path, output: &Path) {
    let before = fs::read(source).unwrap();
    for bound in [24, 384, 4096] {
        let dest = output.join(format!(
            "{}-{bound}.png",
            source.file_name().unwrap().to_string_lossy()
        ));
        let start = Instant::now();
        let info =
            render(source, &dest, bound).unwrap_or_else(|e| panic!("{}: {e:#}", source.display()));
        println!(
            "{} bound={bound} elapsed_ms={:.3}",
            source.file_name().unwrap().to_string_lossy(),
            start.elapsed().as_secs_f64() * 1000.
        );
        assert_eq!((info.width, info.height), (64, 32));
        assert_eq!(
            (info.render_width, info.render_height),
            (bound.min(64) as usize, bound.min(64) as usize / 2)
        );
        let decoded = image::open(&dest).unwrap().to_rgba8();
        assert_eq!(
            decoded.dimensions(),
            (info.render_width as u32, info.render_height as u32)
        );
        let pixel = decoded.get_pixel(decoded.width() - 1, decoded.height() / 2);
        assert!(
            u16::from(pixel[0]) > u16::from(pixel[1]) * 2,
            "{}: {pixel:?}",
            source.display()
        );
        let saved = fs::read(&dest).unwrap();
        assert!(render(source, &dest, bound).is_err());
        assert_eq!(saved, fs::read(&dest).unwrap());
    }
    assert!(render(source, source, 384).is_err());
    assert_eq!(before, fs::read(source).unwrap());
}
#[test]
fn photo_formats_pixels_sizes_alpha_and_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("日本語 assets");
    fs::create_dir(&assets).unwrap();
    let pixels = RgbaImage::from_fn(64, 32, |x, _| {
        Rgba([
            240,
            30,
            60,
            if x < 16 {
                0
            } else if x < 32 {
                128
            } else {
                255
            },
        ])
    });
    for (ext, format) in [("tif", ImageFormat::Tiff), ("tiff", ImageFormat::Tiff)] {
        pixels
            .save_with_format(assets.join(format!("sample.{ext}")), format)
            .unwrap();
    }
    fs::write(
        assets.join("sample.avif"),
        include_bytes!("fixtures/alpha.avif"),
    )
    .unwrap();
    for (version, ext) in [(1, "psd"), (2, "psb")] {
        for depth in [8, 16, 32] {
            fs::write(
                assets.join(format!("rgb-{depth}.{ext}")),
                photoshop(version, depth, false),
            )
            .unwrap();
        }
        fs::write(
            assets.join(format!("rle.{ext}")),
            photoshop(version, 8, true),
        )
        .unwrap();
    }
    for entry in fs::read_dir(&assets).unwrap() {
        check(&entry.unwrap().path(), temp.path());
    }
    for ext in ["tiff", "avif"] {
        let image = image::open(temp.path().join(format!("sample.{ext}-384.png")))
            .unwrap()
            .to_rgba8();
        assert_eq!(image.get_pixel(4, 16)[3], 0, "{ext}");
        let p = image.get_pixel(24, 16);
        assert!(
            (126..=129).contains(&p[3]) && (235..=245).contains(&p[0]),
            "{ext}: {p:?}"
        );
    }
    // Optional export makes the exact same corpus available to the desktop cache/worker tests.
    if let Ok(export) = std::env::var("AE_FORMAT_ASSETS") {
        fs::create_dir_all(&export).unwrap();
        for entry in fs::read_dir(assets).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), Path::new(&export).join(entry.file_name())).unwrap();
        }
    }
}
#[test]
fn malformed_truncated_and_oversized_photos() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["tif", "tiff", "avif", "psd", "psb"] {
        let source = dir.path().join(format!("bad.{ext}"));
        fs::write(&source, b"not an image").unwrap();
        assert!(render(&source, &dir.path().join("out"), 384).is_err());
        assert!(render(&source, &dir.path().join("out"), 4097).is_err());
    }
    let source = dir.path().join("bad.psb");
    let valid = photoshop(2, 8, false);
    for end in [0, 25, 30, 40, 100, valid.len() - 1] {
        fs::write(&source, &valid[..end]).unwrap();
        assert!(render(&source, &dir.path().join("out"), 384).is_err());
    }
    let mut huge = valid;
    huge[14..18].copy_from_slice(&300000u32.to_be_bytes());
    fs::write(&source, huge).unwrap();
    assert!(render(&source, &dir.path().join("out"), 384).is_err());
}

#[test]
fn upstream_layered_photoshop_matches_reference_pixels() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["psd", "psb"] {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/layers.{ext}"));
        let reference = source.with_file_name(format!("layers-{ext}.png"));
        let dest = dir.path().join(format!("{ext}.png"));
        render(&source, &dest, 4096).unwrap();
        let actual = image::open(dest).unwrap().to_rgba8();
        let expected = image::open(reference).unwrap().to_rgba8();
        assert_eq!(actual.dimensions(), expected.dimensions());
        // Saved composites preserve Photoshop's appearance, including layer effects.
        assert_eq!(actual.as_raw(), expected.as_raw(), "{ext}");
        if let Ok(export) = std::env::var("AE_FORMAT_ASSETS") {
            fs::create_dir_all(&export).unwrap();
            fs::copy(&source, Path::new(&export).join(format!("layers.{ext}"))).unwrap();
        }
    }
}
#[test]
fn tiff_first_image_and_photoshop_zip() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let tif = dir.path().join("multipage.tif");
    let mut encoder = tiff::encoder::TiffEncoder::new(fs::File::create(&tif).unwrap()).unwrap();
    for color in [[65535u16, 0, 0], [0, 0, 65535]] {
        encoder
            .write_image::<tiff::encoder::colortype::RGB16>(64, 32, &color.repeat(64 * 32))
            .unwrap();
    }
    drop(encoder);
    {
        let source = &tif;
        let out = source.with_extension("png");
        let info = render(source, &out, 384).unwrap();
        assert_eq!((info.width, info.height), (64, 32));
        assert_eq!(
            *image::open(out).unwrap().to_rgba8().get_pixel(30, 16),
            Rgba([255, 0, 0, 255])
        );
    }
    for (version, ext) in [(1, "psd"), (2, "psb")] {
        let bytes = photoshop(version, 8, false);
        let header = if version == 1 { 38 } else { 42 };
        let mut zipped = bytes[..header].to_vec();
        zipped.extend(2u16.to_be_bytes());
        for channel in bytes[header + 2..].chunks(64 * 32) {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(channel).unwrap();
            zipped.extend(encoder.finish().unwrap());
        }
        let source = dir.path().join(format!("zip.{ext}"));
        fs::write(&source, zipped).unwrap();
        check(&source, dir.path());
    }
}

#[test]
fn two_megapixel_rasters_and_cache_fixture_export() {
    let temp = tempfile::tempdir().unwrap();
    let pixels = RgbaImage::from_fn(2048, 1024, |x, y| {
        Rgba([((x / 128) * 16) as u8, ((y / 64) * 16) as u8, 64, 255])
    });
    for (ext, format) in [("tif", ImageFormat::Tiff), ("tiff", ImageFormat::Tiff)] {
        let source = temp.path().join(format!("large.{ext}"));
        pixels.save_with_format(&source, format).unwrap();
        for bound in [384, 4096] {
            let started = Instant::now();
            let info = render(
                &source,
                &temp.path().join(format!("{ext}-{bound}.png")),
                bound,
            )
            .unwrap();
            assert_eq!((info.width, info.height), (2048, 1024));
            assert_eq!(info.render_width, bound.min(2048) as usize);
            println!(
                "2MP {ext} bound={bound} elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.
            );
        }
        if let Ok(export) = std::env::var("AE_FORMAT_ASSETS") {
            fs::create_dir_all(&export).unwrap();
            fs::copy(source, Path::new(&export).join(format!("large.{ext}"))).unwrap();
        }
    }
}
