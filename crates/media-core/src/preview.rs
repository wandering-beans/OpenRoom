//! Compact sRGB library previews. Development and export retain their original inputs.
use anyhow::{Result, ensure};
use image::{ImageDecoder, ImageEncoder, RgbImage};
use libheif_rs::{
    Channel, ColorSpace, CompressionFormat, EncoderParameterValue, EncoderQuality, HeifContext,
    LibHeif, RgbChroma,
};

pub fn avif_available() -> bool {
    LibHeif::new()
        .encoder_for_format(CompressionFormat::Av1)
        .is_ok()
}

pub fn read_srgb(path: &std::path::Path) -> Result<RgbImage> {
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let icc = decoder.icc_profile()?;
    let pixels = image::DynamicImage::from_decoder(decoder)?;
    ensure!(
        pixels.width().max(pixels.height()) <= 1600,
        "Library input exceeds 1600px"
    );
    let pixels = if let Some(icc) = icc {
        let pixels = pixels.to_rgb32f();
        let source = moxcms::ColorProfile::new_from_slice(&icc)?;
        let transform = source.create_transform_f32(
            moxcms::Layout::Rgb,
            &moxcms::ColorProfile::new_srgb(),
            moxcms::Layout::Rgb,
            Default::default(),
        )?;
        let mut converted = vec![0.; pixels.as_raw().len()];
        transform.transform(pixels.as_raw(), &mut converted)?;
        image::Rgb32FImage::from_raw(pixels.width(), pixels.height(), converted)
            .ok_or_else(|| anyhow::anyhow!("Invalid sRGB image"))?
    } else {
        return Ok(pixels.to_rgb8());
    };
    Ok(image::DynamicImage::ImageRgb32F(pixels).to_rgb8())
}

pub fn encode(pixels: &RgbImage, thumbnail: bool, avif: bool) -> Result<Vec<u8>> {
    ensure!(
        pixels.width() > 0 && pixels.height() > 0 && pixels.width().max(pixels.height()) <= 1600,
        "Invalid library preview dimensions"
    );
    let profile = moxcms::ColorProfile::new_srgb().encode()?;
    if !avif {
        let mut output = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut output,
            if thumbnail { 75 } else { 82 },
        );
        encoder.set_icc_profile(profile)?;
        encoder.write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        return Ok(output);
    }
    let lib = LibHeif::new();
    let mut encoder = lib.encoder_for_format(CompressionFormat::Av1)?;
    encoder.set_quality(EncoderQuality::Lossy(if thumbnail { 55 } else { 65 }))?;
    for (name, value) in [("speed", 6), ("threads", 2)] {
        if encoder.parameter(name)?.is_some() {
            encoder.set_parameter_value(name, EncoderParameterValue::Int(value))?;
        }
    }
    let mut image = libheif_rs::Image::new(
        pixels.width(),
        pixels.height(),
        ColorSpace::Rgb(RgbChroma::Rgb),
    )?;
    image.set_color_profile_raw(&libheif_rs::ColorProfileRaw::new(
        libheif_rs::color_profile_types::PROF,
        profile,
    ))?;
    image.create_plane(Channel::Interleaved, pixels.width(), pixels.height(), 8)?;
    let plane = image
        .planes_mut()
        .interleaved
        .ok_or_else(|| anyhow::anyhow!("Missing RGB plane"))?;
    let row_bytes = pixels.width() as usize * 3;
    for (source, destination) in pixels
        .as_raw()
        .chunks_exact(row_bytes)
        .zip(plane.data.chunks_mut(plane.stride))
    {
        destination[..row_bytes].copy_from_slice(source);
    }
    let mut context = HeifContext::new()?;
    context.encode_image(&image, &mut encoder, None)?;
    Ok(context.write_to_bytes()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncompressed_intermediate_preserves_pixels_and_originals() {
        let dir = tempfile::tempdir().unwrap();
        let pixels = image::RgbaImage::from_fn(257, 193, |x, y| {
            image::Rgba([x as u8, y as u8, (x + y) as u8, (x * y) as u8])
        });
        let png = dir.path().join("reference.png");
        let tiff = dir.path().join("intermediate.tif");
        crate::write_preview(&png, &pixels).unwrap();
        crate::write_preview(&tiff, &pixels).unwrap();
        assert_eq!(image::open(&png).unwrap().to_rgba8(), pixels);
        assert_eq!(image::open(&tiff).unwrap().to_rgba8(), pixels);
        assert_eq!(read_srgb(&png).unwrap(), read_srgb(&tiff).unwrap());
        let before = std::fs::read(&tiff).unwrap();
        assert!(crate::write_preview(&tiff, &pixels).is_err());
        assert_eq!(std::fs::read(tiff).unwrap(), before);
    }

    #[test]
    fn unprofiled_sixteen_bit_conversion_matches_float_path() {
        let dir = tempfile::tempdir().unwrap();
        let pixels = image::ImageBuffer::from_fn(256, 256, |x, y| {
            let value = (y * 256 + x) as u16;
            image::Rgb([value, value / 2, u16::MAX - value])
        });
        let image = image::DynamicImage::ImageRgb16(pixels);
        let expected = image::DynamicImage::ImageRgb32F(image.to_rgb32f()).to_rgb8();
        for extension in ["png", "tif"] {
            let path = dir.path().join(format!("sixteen-bit.{extension}"));
            image.save(&path).unwrap();
            assert_eq!(read_srgb(&path).unwrap(), expected);
        }
    }

    #[test]
    fn compact_preview_encodes_avif_and_jpeg_without_changing_pixels() {
        let pixels = RgbImage::from_fn(180, 120, |x, y| image::Rgb([x as u8, y as u8, 90]));
        let before = pixels.clone();
        for thumbnail in [false, true] {
            let jpeg = encode(&pixels, thumbnail, false).unwrap();
            assert_eq!(image::load_from_memory(&jpeg).unwrap().width(), 180);
            assert!(jpeg.len() < 180 * 120 * 3);
            assert!(
                avif_available(),
                "AV1 encoder must be available in the test/package environment"
            );
            let avif = encode(&pixels, thumbnail, true).unwrap();
            assert!(avif.windows(4).any(|v| v == b"avif"));
            let context = HeifContext::read_from_bytes(&avif).unwrap();
            let handle = context.primary_image_handle().unwrap();
            assert_eq!((handle.width(), handle.height()), (180, 120));
        }
        assert_eq!(pixels, before);
    }
}
