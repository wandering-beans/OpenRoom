//! Synthetic inputs for the independent LRC quality and dynamic-range comparison.
use anyhow::{Result, ensure};
use image::{DynamicImage, ImageEncoder, Rgba, Rgba32FImage};
use std::path::PathBuf;

fn main() -> Result<()> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("output directory required"),
    );
    ensure!(!directory.exists(), "Fixture directory already exists");
    std::fs::create_dir_all(&directory)?;
    let hdr = Rgba32FImage::from_fn(1024, 128, |x, _| {
        let value = 2f32.powf(x as f32 / 1023. * 15. - 10.);
        Rgba([value, value, value, 1.])
    });
    DynamicImage::ImageRgba32F(hdr.clone()).save(directory.join("linear-hdr.tif"))?;
    let mut linear = moxcms::ColorProfile::new_srgb();
    let transfer = moxcms::ToneReprCurve::Lut(Vec::new());
    linear.red_trc = Some(transfer.clone());
    linear.green_trc = Some(transfer.clone());
    linear.blue_trc = Some(transfer);
    linear.cicp = None;
    let mut encoder = image::codecs::tiff::TiffEncoder::new(std::fs::File::create(
        directory.join("linear-hdr-icc.tif"),
    )?);
    encoder.set_icc_profile(linear.encode()?)?;
    DynamicImage::ImageRgba32F(hdr).write_with_encoder(encoder)?;
    let p3 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(1024, 128, |x, y| {
        let ramp = (x as f32 / 1023. * 65535.).round() as u16;
        image::Rgb(match y / 16 {
            0 => [ramp, 0, 0],
            1 => [0, ramp, 0],
            2 => [0, 0, ramp],
            3 => [ramp, ramp, 0],
            4 => [ramp, 0, ramp],
            5 => [0, ramp, ramp],
            6 => [ramp, ramp / 2, ramp / 4],
            _ => [ramp; 3],
        })
    });
    let mut encoder = image::codecs::png::PngEncoder::new(std::fs::File::create(
        directory.join("display-p3.png"),
    )?);
    encoder.set_icc_profile(moxcms::ColorProfile::new_display_p3().encode()?)?;
    DynamicImage::ImageRgb16(p3).write_with_encoder(encoder)?;
    image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(4096, 128, |x, _| {
        image::Rgb([8192 + x as u16; 3])
    })
    .save(directory.join("fine-gradient.png"))?;
    Ok(())
}
