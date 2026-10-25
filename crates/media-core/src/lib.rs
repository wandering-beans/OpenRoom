pub mod develop_input;
pub mod metadata;
mod photoshop_input;
pub mod preview;
// Shared bounded media rendering; no desktop or OS UI dependencies.
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageEncoder, metadata::Orientation};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "arw", "sr2", "srf", "raf", "nef", "nrw", "rw2", "orf", "pef",
    "rwl", "srw", "3fr", "fff", "iiq",
];

pub fn is_raw(source: &Path) -> bool {
    source
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| RAW_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RenderInfo {
    pub width: usize,
    pub height: usize,
    pub render_width: usize,
    pub render_height: usize,
    pub source_kind: String,
}

pub fn render(source: &Path, destination: &Path, bound: u32) -> Result<RenderInfo> {
    ensure!((1..=4096).contains(&bound), "Invalid render size");
    ensure!(
        std::fs::metadata(source)?.len() <= 1024 * 1024 * 1024,
        "Media exceeds 1 GiB decoding limit"
    );
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (image, kind) = match ext.as_str() {
        ext if RAW_EXTENSIONS.contains(&ext) => raw(source, bound, true)?,
        "heic" | "heif" | "hif" | "avif" => {
            (heif(source, bound)?, "heif-avif-primary-or-thumbnail")
        }
        "tif" | "tiff" => (raster(source)?, "raster-first-image"),
        "psd" | "psb" => (photoshop(source)?, "photoshop-saved-composite"),
        _ => anyhow::bail!("Unsupported media format: {ext}"),
    };
    let (width, height) = (image.width(), image.height());
    let limit = bound.min(image.width().max(image.height()));
    let thumb = image.thumbnail(limit, limit).to_rgba8();
    write_preview(destination, &thumb)?;
    Ok(RenderInfo {
        width: width as usize,
        height: height as usize,
        render_width: thumb.width() as usize,
        render_height: thumb.height() as usize,
        source_kind: kind.into(),
    })
}

/// Lossless preview encoding favors interaction latency over compression ratio.
pub fn write_preview(destination: &Path, pixels: &image::RgbaImage) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut output = std::io::BufWriter::new(file);
    if destination.extension().is_some_and(|ext| ext == "tif") {
        // Library imports immediately decode this intermediate again; compression is wasted work.
        image::codecs::tiff::TiffEncoder::new(&mut output).write_image(
            pixels.as_raw(), pixels.width(), pixels.height(), image::ExtendedColorType::Rgba8,
        )?;
    } else {
        encode_preview(&mut output, pixels)?;
    }
    std::io::Write::flush(&mut output)?;
    Ok(())
}

/// The interactive preview uses the same lossless encoding without writing a cache file.
pub fn encode_preview(output: impl std::io::Write, pixels: &image::RgbaImage) -> Result<()> {
    image::codecs::png::PngEncoder::new_with_quality(
        output,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(
        pixels.as_raw(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(())
}

fn check_dimensions(width: u32, height: u32) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 150_000_000,
        "Image dimensions exceed 150 megapixels or are empty"
    );
    Ok(())
}

fn raster(path: &Path) -> Result<DynamicImage> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
    pool.install(|| raster_decode(path))
}

fn raster_decode(path: &Path) -> Result<DynamicImage> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(image::Limits::default()); // 512 MiB decoder allocation budget.
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height)?;
    ensure!(
        decoder.total_bytes() <= 512 * 1024 * 1024,
        "Decoded image exceeds 512 MiB"
    );
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn photoshop(path: &Path) -> Result<DynamicImage> {
    use std::io::Read;
    let mut header = [0u8; 26];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    ensure!(&header[..4] == b"8BPS", "Invalid Photoshop header");
    let height = u32::from_be_bytes(header[14..18].try_into()?);
    let width = u32::from_be_bytes(header[18..22].try_into()?);
    check_dimensions(width, height)?;
    let bytes = std::fs::read(path)?;
    validate_photoshop_composite(&bytes)?;
    let psd = ag_psd::read_psd(
        &bytes,
        &ag_psd::psd::ReadOptions {
            skip_layer_image_data: Some(true),
            skip_linked_files_data: Some(true),
            skip_thumbnail: Some(true),
            use_image_data: Some(true),
            total_memory_limit: Some(512 * 1024 * 1024),
            ..Default::default()
        },
    )
    .context("Cannot decode saved PSD/PSB composite")?;
    let pixels = psd
        .image_data
        .or(psd.canvas)
        .context("PSD/PSB has no saved composite; save with Maximize Compatibility")?;
    Ok(DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.data)
            .context("Invalid PSD/PSB composite dimensions")?,
    ))
}

// ag-psd's permissive metadata parser accepts unknown Photoshop records, but it
// zero-pads truncated raw pixel reads. Check composite boundaries before decoding.
fn validate_photoshop_composite(bytes: &[u8]) -> Result<()> {
    use std::io::{Cursor, Read, Seek, SeekFrom};
    fn integer(input: &mut Cursor<&[u8]>, size: usize) -> Result<u64> {
        let mut value = [0u8; 8];
        input.read_exact(&mut value[8 - size..])?;
        Ok(u64::from_be_bytes(value))
    }
    ensure!(
        bytes.len() >= 26 && &bytes[..4] == b"8BPS",
        "Truncated Photoshop header"
    );
    let version = u16::from_be_bytes(bytes[4..6].try_into()?);
    ensure!(matches!(version, 1 | 2), "Invalid Photoshop version");
    let channels = u16::from_be_bytes(bytes[12..14].try_into()?) as u64;
    let height = u32::from_be_bytes(bytes[14..18].try_into()?) as u64;
    let width = u32::from_be_bytes(bytes[18..22].try_into()?) as u64;
    let depth = u16::from_be_bytes(bytes[22..24].try_into()?) as u64;
    check_dimensions(width as u32, height as u32)?;
    ensure!(
        (1..=16).contains(&channels) && matches!(depth, 1 | 8 | 16 | 32),
        "Unsupported Photoshop channels/depth"
    );
    let mut cursor = Cursor::new(bytes);
    cursor.set_position(26);
    for length_size in [4, 4, if version == 2 { 8 } else { 4 }] {
        let length = integer(&mut cursor, length_size)?;
        ensure!(
            length <= bytes.len() as u64 - cursor.position(),
            "Truncated Photoshop section"
        );
        cursor.seek(SeekFrom::Current(length as i64))?;
    }
    let compression = integer(&mut cursor, 2)?;
    let required = match compression {
        0 => (width * depth).div_ceil(8) * height * channels,
        1 => {
            let mut sum = 0u64;
            for _ in 0..height * channels {
                sum += integer(&mut cursor, if version == 2 { 4 } else { 2 })?;
            }
            sum
        }
        // ZIP decoder itself verifies exact expanded size and stream termination.
        2 | 3 => 1,
        _ => anyhow::bail!("Unsupported Photoshop compression"),
    };
    ensure!(
        required <= bytes.len() as u64 - cursor.position(),
        "Truncated Photoshop composite"
    );
    Ok(())
}

/// Camera-rendered reference only; never substitute a sensor decode for a missing JPEG.
pub fn embedded_raw_preview(path: &Path) -> Result<DynamicImage> {
    Ok(raw(path, 1600, false)?.0.thumbnail(256, 256))
}

fn raw(path: &Path, bound: u32, allow_develop: bool) -> Result<(DynamicImage, &'static str)> {
    use rawler::{decoders::RawDecodeParams, imgop::develop::RawDevelop, rawsource::RawSource};
    let source = if bound <= 1600 { RawSource::new_preview(path)? } else { RawSource::new(path)? };
    let decoder = rawler::get_decoder(&source).context("RAW camera/encoding is not supported")?;
    let params = RawDecodeParams::default();
    let orientation = decoder
        .raw_metadata(&source, &params)
        .ok()
        .and_then(|m| m.exif.orientation)
        .and_then(|v| Orientation::from_exif(v as u8))
        .unwrap_or(Orientation::NoTransforms);
    // Embedded previews avoid sensor decoding during scrolling. Detail uses a larger embedded image.
    let mut image = if bound <= 384 {
        decoder.thumbnail_image(&source, &params).ok().flatten()
    } else {
        None
    };
    if image.is_none() {
        image = decoder.preview_image(&source, &params).ok().flatten();
    }
    if image.is_none() || bound > 384 {
        image = decoder
            .full_image(&source, &params)
            .ok()
            .flatten()
            .or(image);
    }
    // Fit-to-window uses the camera preview immediately. Explicit high-resolution
    // requests develop the RAW when the embedded image cannot supply enough pixels.
    if bound >= 4096
        && image
            .as_ref()
            .is_some_and(|i| i.width().max(i.height()) < bound)
    {
        image = None;
    }
    let kind;
    let mut image = if let Some(image) = image {
        kind = "raw-embedded";
        image
    } else {
        ensure!(allow_develop, "RAW has no embedded camera preview");
        kind = "raw-developed";
        // Keep each worker bounded instead of giving every thumbnail all CPU cores.
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
        pool.install(move || -> Result<DynamicImage> {
            let dimensions = decoder.raw_image(&source, &params, true)?;
            ensure!(
                dimensions.width.saturating_mul(dimensions.height) <= 150_000_000,
                "RAW exceeds 150 megapixels"
            );
            let raw = decoder.raw_image(&source, &params, false)?;
            ensure!(
                raw.width.saturating_mul(raw.height) <= 150_000_000,
                "RAW exceeds 150 megapixels"
            );
            RawDevelop::default()
                .develop_intermediate(&raw)?
                .to_dynamic_image()
                .context("RAW development failed")
        })?
    };
    image.apply_orientation(orientation);
    Ok((image, kind))
}

fn heif(path: &Path, bound: u32) -> Result<DynamicImage> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let lib = LibHeif::new();
    // File-backed reader supports Unicode paths on Windows without narrow C filename APIs.
    let reader =
        libheif_rs::StreamReader::new(std::fs::File::open(path)?, std::fs::metadata(path)?.len());
    let mut ctx = HeifContext::read_from_reader(Box::new(reader))?;
    ctx.set_max_decoding_threads(2);
    let primary = ctx.primary_image_handle()?;
    ensure!(
        (primary.width() as u64) * (primary.height() as u64) <= 150_000_000,
        "HEIF exceeds 150 megapixels"
    );
    let mut ids = vec![0; primary.number_of_thumbnails()];
    primary.thumbnail_ids(&mut ids);
    let thumbnail = if bound <= 384 {
        ids.into_iter().find_map(|id| primary.thumbnail(id).ok())
    } else {
        None
    };
    let handle = thumbnail.as_ref().unwrap_or(&primary);
    let decoded = lib
        .decode(handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .context("HEIF/AVIF decoding failed; libheif requires HEVC/AV1 decoder plugins")?;
    let planes = decoded.planes();
    let plane = planes.interleaved.context("HEIF RGBA plane missing")?;
    let row = decoded.width() as usize * 4;
    let mut pixels = Vec::with_capacity(row * decoded.height() as usize);
    for y in 0..decoded.height() as usize {
        pixels.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + row]);
    }
    Ok(DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(decoded.width(), decoded.height(), pixels)
            .context("Invalid HEIF dimensions")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires AE_RAW_ASSETS containing the public camera fixtures"]
    fn camera_references_use_embedded_images_across_manufacturers() {
        let directory = std::path::PathBuf::from(std::env::var_os("AE_RAW_ASSETS").unwrap());
        for (name, dimensions) in [
            (
                "Fujifilm - X-T2 - 14bit 14bit compressed (3_2).RAF",
                (1920, 1280),
            ),
            ("Panasonic - DMC-FZ28 - 16_9.RW2", (1920, 1080)),
            ("Olympus - E-3 - 16bit (4_3).ORF", (1600, 1200)),
            (
                "Samsung - NX500 - 12bit 12bit normal compression (3_2).SRW",
                (6480, 4320),
            ),
        ] {
            let path = directory.join(name);
            let (reference, kind) = raw(&path, 1600, false).unwrap();
            assert_eq!(kind, "raw-embedded", "{name}");
            assert_eq!(
                (reference.width(), reference.height()),
                dimensions,
                "{name}"
            );
            assert!(embedded_raw_preview(&path).unwrap().width() <= 256);
        }
    }

    #[test]
    fn corrupt_media_and_invalid_requests_fail() {
        let dir = tempfile::tempdir().unwrap();
        for ext in RAW_EXTENSIONS
            .iter()
            .copied()
            .chain(["heic", "heif", "hif"])
        {
            let source = dir.path().join(format!("broken.{ext}"));
            std::fs::write(&source, b"not an image").unwrap();
            assert!(render(&source, &dir.path().join("out.png"), 384).is_err());
            assert!(render(&source, &dir.path().join("out.png"), 0).is_err());
        }
    }
    #[test]
    #[ignore = "requires AE_RAW_ASSETS containing real RAW/HEIF samples"]
    fn real_samples() {
        let dir = std::env::var("AE_RAW_ASSETS").unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut formats = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .unwrap()
                .to_str()
                .unwrap()
                .to_ascii_lowercase();
            if !RAW_EXTENSIONS.contains(&ext.as_str())
                && !["heic", "heif", "hif"].contains(&ext.as_str())
            {
                continue;
            }
            let before = std::fs::read(&path).unwrap();
            for size in [384, 4096] {
                let start = std::time::Instant::now();
                let result = render(&path, &out.path().join(format!("{ext}-{size}.png")), size)
                    .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
                assert!(
                    result.render_width <= size as usize && result.render_height <= size as usize
                );
                assert!(result.render_width > 0 && result.render_height > 0);
                assert!(
                    result.render_width <= result.width && result.render_height <= result.height
                );
                println!(
                    "{} size={} elapsed_ms={:.1} {:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    size,
                    start.elapsed().as_secs_f64() * 1000.,
                    result
                );
            }
            assert!(
                render(&path, &path, 384).is_err(),
                "Original must never be an output"
            );
            assert_eq!(before, std::fs::read(&path).unwrap());
            formats.insert(ext);
        }
        for ext in RAW_EXTENSIONS
            .iter()
            .copied()
            .filter(|ext| *ext != "dng")
            .chain(["heic"])
        {
            assert!(formats.contains(ext), "Missing sample for {ext}");
        }
    }
}
