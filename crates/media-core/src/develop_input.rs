//! Full sensor input for development, separate from embedded library previews.
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageBuffer, ImageDecoder, Rgb, metadata::Orientation};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Camera-space samples stay unbalanced until the editing recipe is applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawColor {
    pub camera: String,
    pub neutral: [f32; 3],
    pub xyz_to_camera: [[f32; 3]; 3],
    pub baseline_exposure: f32,
    pub calibration: [[[f32; 3]; 3]; 2],
    pub calibration_signature: String,
    pub analog_balance: [f32; 3],
    #[serde(default)]
    pub as_shot: [f32; 2],
    #[serde(default)]
    pub profile_signature: String,
}

impl RawColor {
    /// Reject damaged cache metadata before matrix inversion or pixel processing.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.neutral
                .iter()
                .chain(&self.analog_balance)
                .all(|v| v.is_finite() && *v > 0.),
            "Invalid RAW white balance"
        );
        ensure!(
            self.baseline_exposure.is_finite() && self.baseline_exposure.abs() <= 10.,
            "Invalid RAW baseline exposure"
        );
        for matrix in std::iter::once(&self.xyz_to_camera).chain(&self.calibration) {
            ensure!(
                matrix
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() <= 100.),
                "Invalid RAW color matrix"
            );
            let [a, b, c] = *matrix;
            let determinant = a[0] * (b[1] * c[2] - b[2] * c[1])
                - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
            ensure!(determinant.abs() > 1e-10, "Singular RAW color matrix");
        }
        Ok(())
    }
}

pub struct DecodedPhoto {
    pub image: DynamicImage,
    pub linear: bool,
    pub icc: Option<Vec<u8>>,
    pub raw: Option<RawColor>,
}

pub fn decode(path: &Path) -> Result<DecodedPhoto> {
    ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024 * 1024,
        "Photo exceeds 1 GiB"
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let photo = match extension.as_str() {
        ext if crate::RAW_EXTENSIONS.contains(&ext) => return decode_raw(path),
        "heic" | "heif" | "hif" | "avif" => decode_heif(path)?,
        "psd" | "psb" => super::photoshop_input::decode(path)?,
        "tif" | "tiff" => {
            // image 0.25.10 applies TIFF limits before metadata in ImageReader,
            // which can hide a valid ICC on small images. Read it before setting limits.
            let file = std::io::BufReader::new(std::fs::File::open(path)?);
            raster(image::codecs::tiff::TiffDecoder::new(file)?)?
        }
        "png" | "jpg" | "jpeg" => {
            let reader = image::ImageReader::open(path)?.with_guessed_format()?;
            raster(reader.into_decoder()?)?
        }
        _ => anyhow::bail!("Unsupported development input: {extension}"),
    };
    check_develop_dimensions(photo.image.width(), photo.image.height())?;
    Ok(photo)
}

fn raster(mut decoder: impl ImageDecoder) -> Result<DecodedPhoto> {
    let (width, height) = decoder.dimensions();
    check_develop_dimensions(width, height)?;
    let orientation = decoder.orientation()?;
    let linear = matches!(
        decoder.color_type(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    );
    let icc = decoder.icc_profile()?;
    ensure!(
        icc.as_ref()
            .is_none_or(|data| data.len() <= 16 * 1024 * 1024),
        "ICC exceeds 16 MiB"
    );
    decoder.set_limits(image::Limits::default())?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(DecodedPhoto {
        image,
        linear,
        icc,
        raw: None,
    })
}

fn check_develop_dimensions(width: u32, height: u32) -> Result<()> {
    // Bound the isolated worker's float working set to 64 MP, not an unbounded RAW allocation.
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 64_000_000,
        "Development input exceeds 64 megapixels or is empty"
    );
    Ok(())
}

fn decode_raw(path: &Path) -> Result<DecodedPhoto> {
    use rawler::{
        decoders::RawDecodeParams,
        imgop::develop::{Intermediate, ProcessingStep, RawDevelop},
        rawsource::RawSource,
    };
    let source = RawSource::new(path)?;
    let decoder = rawler::get_decoder(&source).context("RAW camera/encoding is not supported")?;
    let parameters = RawDecodeParams::default();
    let dimensions = decoder.raw_image(&source, &parameters, true)?;
    check_develop_dimensions(dimensions.width as u32, dimensions.height as u32)?;
    drop(dimensions);
    let raw = decoder.raw_image(&source, &parameters, false)?;
    check_develop_dimensions(raw.width as u32, raw.height as u32)?;
    let orientation = decoder
        .raw_metadata(&source, &parameters)
        .ok()
        .and_then(|metadata| metadata.exif.orientation)
        .and_then(|value| Orientation::from_exif(value as u8))
        .unwrap_or(Orientation::NoTransforms);
    let matrix = raw.color_matrix_find_first([
        rawler::imgop::xyz::Illuminant::D65,
        rawler::imgop::xyz::Illuminant::A,
        rawler::imgop::xyz::Illuminant::D50,
        rawler::imgop::xyz::Illuminant::D55,
    ]);
    // rawler does not populate RawImage::dng_tags when decoding a DNG.
    let mut calibration_tags = raw.dng_tags.clone();
    let is_dng = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dng"));
    if is_dng {
        use rawler::formats::tiff::{GenericTiffReader, reader::TiffReader};
        let tiff = GenericTiffReader::new_with_buffer(source.buf(), 0, 0, Some(1))?;
        for tag in [50723, 50724, 50727, 50730, 50931] {
            if let Some(entry) = tiff.root_ifd().entries.get(&tag) {
                calibration_tags.insert(tag, entry.value.clone());
            }
        }
    }
    let camera_calibration = |tag| {
        let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        calibration_tags
            .get(&tag)
            .and_then(|v| {
                let values: Option<Vec<f32>> = (0..9)
                    .map(|i| {
                        v.get_f32(i)
                            .ok()
                            .flatten()
                            .filter(|x| x.is_finite() && x.abs() <= 100.)
                    })
                    .collect();
                values.map(|values| {
                    std::array::from_fn(|i| std::array::from_fn(|j| values[i * 3 + j]))
                })
            })
            .unwrap_or_else(|| {
                if is_dng {
                    identity
                } else {
                    native_calibration(&raw.clean_make, &raw.clean_model)
                }
            })
    };
    // Four-channel sensors retain the library path until their profile model is supported.
    let raw_color = matrix
        .filter(|(_, values)| values.len() == 9)
        .map(|(_, values)| RawColor {
            as_shot: [0., 0.],
            profile_signature: String::new(),
            camera: format!("{} {}", raw.clean_make, raw.clean_model),
            neutral: std::array::from_fn(|i| {
                if raw.wb_coeffs[i].is_finite()
                    && raw.wb_coeffs[i] > 0.
                    && raw.wb_coeffs[1].is_finite()
                    && raw.wb_coeffs[1] > 0.
                {
                    raw.wb_coeffs[1] / raw.wb_coeffs[i]
                } else {
                    1.
                }
            }),
            xyz_to_camera: std::array::from_fn(|i| std::array::from_fn(|j| values[i * 3 + j])),
            calibration: [camera_calibration(50723), camera_calibration(50724)],
            calibration_signature: calibration_tags
                .get(&50931)
                .and_then(|v| v.as_string())
                .cloned()
                .unwrap_or_else(|| {
                    if is_dng {
                        String::new()
                    } else {
                        "com.adobe".into()
                    }
                }),
            analog_balance: std::array::from_fn(|i| {
                calibration_tags
                    .get(&50727)
                    .and_then(|v| v.get_f32(i).ok().flatten())
                    .filter(|v| v.is_finite() && *v > 0. && *v <= 100.)
                    .unwrap_or(1.)
            }),
            baseline_exposure: calibration_tags
                .get(&50730)
                .and_then(|v| v.get_f32(0).ok().flatten())
                .filter(|v| v.is_finite() && v.abs() <= 10.)
                .unwrap_or_else(|| {
                    if is_dng {
                        0.
                    } else {
                        native_baseline(&raw.clean_make, &raw.clean_model)
                    }
                }),
        });
    if let Some(color) = &raw_color {
        color.validate()?;
    }
    let mut develop = RawDevelop::default();
    develop.steps.retain(|step| {
        *step != ProcessingStep::SRgb && (raw_color.is_none() || *step != ProcessingStep::Calibrate)
    });
    let intermediate = develop.develop_intermediate(&raw)?;
    drop(raw);
    drop(decoder);
    drop(source);
    let dimensions = intermediate.dim();
    let pixels = match intermediate {
        // Reuse the owned RGB allocation instead of copying every float.
        Intermediate::ThreeColor(pixels) => pixels.into_inner().into_flattened(),
        Intermediate::Monochrome(pixels) => pixels
            .data
            .into_iter()
            .flat_map(|value| [value; 3])
            .collect(),
        Intermediate::FourColor(_) => anyhow::bail!("RAW calibration did not produce RGB"),
    };
    let image: ImageBuffer<Rgb<f32>, Vec<f32>> =
        ImageBuffer::from_raw(dimensions.w as u32, dimensions.h as u32, pixels)
            .context("Invalid RAW working dimensions")?;
    let mut image = DynamicImage::ImageRgb32F(image);
    image.apply_orientation(orientation);
    Ok(DecodedPhoto {
        image,
        linear: true,
        icc: None,
        raw: raw_color,
    })
}

// BaselineExposure from LRC 15.4 DNG exports of the six documented RAW fixtures.
// An embedded DNG value takes precedence; unmeasured native cameras retain 0 EV.
fn native_calibration(make: &str, model: &str) -> [[f32; 3]; 3] {
    let gains = match (make, model) {
        ("Canon", "EOS 350D") => [1.0379, 1., 0.9987],
        ("Canon", "EOS M50") => [1.0253, 1., 0.9712],
        ("Fujifilm", "X-T2") => [1.0362, 1., 1.043],
        ("Sony", "NEX-6") => [0.9449, 1., 1.0397],
        ("Sony", "DSC-R1") => [0.9403, 1., 1.0081],
        _ => [1.; 3],
    };
    std::array::from_fn(|i| std::array::from_fn(|j| if i == j { gains[i] } else { 0. }))
}
fn native_baseline(make: &str, model: &str) -> f32 {
    match (make, model) {
        ("Canon", "EOS 350D") => 0.25,
        ("Canon", "EOS D30") => -0.5,
        ("Canon", "EOS M50") => 0.28,
        ("Fujifilm", "X-T2") => 0.15,
        ("Sony", "NEX-6") => 0.35,
        _ => 0.,
    }
}

fn nclx_profile(
    primaries: libheif_rs::ColorPrimaries,
    transfer: libheif_rs::TransferCharacteristics,
) -> Result<Option<Vec<u8>>> {
    use libheif_rs::{ColorPrimaries, TransferCharacteristics};
    ensure!(
        !matches!(
            transfer,
            TransferCharacteristics::ITU_R_BT_2100_0_PQ
                | TransferCharacteristics::ITU_R_BT_2100_0_HLG
        ),
        "HDRのHEIF/AVIFは現像に対応していません。HDR対応アプリで現像するか、16bit TIFFに変換してください。"
    );
    if primaries == ColorPrimaries::Unspecified && transfer == TransferCharacteristics::Unspecified
    {
        return Ok(None);
    }
    let primaries = moxcms::CicpColorPrimaries::try_from(primaries as u8)
        .context("Unknown HEIF color primaries")?;
    let transfer = moxcms::TransferCharacteristics::try_from(transfer as u8)
        .context("Unknown HEIF transfer function")?;
    let cicp = moxcms::CicpProfile {
        color_primaries: primaries,
        transfer_characteristics: transfer,
        matrix_coefficients: moxcms::MatrixCoefficients::Identity,
        full_range: true,
    };
    let _: moxcms::ColorPrimaries = primaries.try_into().context("Unsupported HEIF primaries")?;
    let _: moxcms::ToneReprCurve = transfer
        .try_into()
        .context("Unsupported HEIF transfer function")?;
    let profile = moxcms::ColorProfile::new_from_cicp(cicp);
    Ok(Some(profile.encode()?))
}

fn decode_heif(path: &Path) -> Result<DecodedPhoto> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let reader =
        libheif_rs::StreamReader::new(std::fs::File::open(path)?, std::fs::metadata(path)?.len());
    let mut context = HeifContext::read_from_reader(Box::new(reader))?;
    context.set_max_decoding_threads(2);
    let handle = context.primary_image_handle()?;
    check_develop_dimensions(handle.width(), handle.height())?;
    let nclx = handle.color_profile_nclx();
    let icc = handle.color_profile_raw().map(|profile| profile.data);
    let inferred = nclx
        .as_ref()
        .filter(|profile| {
            icc.is_none()
                || matches!(
                    profile.transfer_characteristics(),
                    libheif_rs::TransferCharacteristics::ITU_R_BT_2100_0_PQ
                        | libheif_rs::TransferCharacteristics::ITU_R_BT_2100_0_HLG
                )
        })
        .map(|profile| {
            nclx_profile(
                profile.color_primaries(),
                profile.transfer_characteristics(),
            )
        })
        .transpose()?
        .flatten();
    let icc = icc.or(inferred);
    ensure!(
        icc.as_ref()
            .is_none_or(|data| data.len() <= 16 * 1024 * 1024),
        "ICC exceeds 16 MiB"
    );
    let depth = handle.luma_bits_per_pixel();
    let chroma = if depth > 8 {
        RgbChroma::HdrRgbaLe
    } else {
        RgbChroma::Rgba
    };
    let decoded = LibHeif::new().decode(&handle, ColorSpace::Rgb(chroma), None)?;
    let planes = decoded.planes();
    let plane = planes.interleaved.context("Missing HEIF pixel plane")?;
    let (width, height) = (decoded.width(), decoded.height());
    let row_samples = width as usize * 4;
    let image = if depth > 8 {
        ensure!(
            (9..=16).contains(&plane.bits_per_pixel),
            "Invalid HEIF bit depth"
        );
        let maximum = (1u32 << plane.bits_per_pixel) - 1;
        let mut pixels = Vec::with_capacity(row_samples * height as usize);
        for row in plane.data.chunks(plane.stride).take(height as usize) {
            for sample in row[..row_samples * 2].as_chunks::<2>().0 {
                let value = u16::from_le_bytes([sample[0], sample[1]]);
                pixels.push(((u32::from(value) * 65535 + maximum / 2) / maximum).min(65535) as u16);
            }
        }
        DynamicImage::ImageRgba16(
            ImageBuffer::from_raw(width, height, pixels).context("Invalid HEIF dimensions")?,
        )
    } else {
        let mut pixels = Vec::with_capacity(row_samples * height as usize);
        for row in plane.data.chunks(plane.stride).take(height as usize) {
            pixels.extend_from_slice(&row[..row_samples]);
        }
        DynamicImage::ImageRgba8(
            ImageBuffer::from_raw(width, height, pixels).context("Invalid HEIF dimensions")?,
        )
    };
    Ok(DecodedPhoto {
        image,
        linear: nclx.is_some_and(|profile| {
            profile.transfer_characteristics() == libheif_rs::TransferCharacteristics::Linear
        }),
        icc,
        raw: None,
    })
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    #[test]
    fn nclx_preserves_display_p3_primaries_and_rejects_unsupported_hdr() {
        use libheif_rs::{ColorPrimaries, TransferCharacteristics};
        let data = nclx_profile(
            ColorPrimaries::SMPTE_EG_432_1,
            TransferCharacteristics::IEC_61966_2_1,
        )
        .unwrap()
        .unwrap();
        let profile = moxcms::ColorProfile::new_from_slice(&data).unwrap();
        let matrix = profile.transform_matrix(&moxcms::ColorProfile::new_srgb());
        assert!(matrix.v[0][0] > 1.2 && matrix.v[1][0] < -0.03);
        for transfer in [
            TransferCharacteristics::ITU_R_BT_2100_0_PQ,
            TransferCharacteristics::ITU_R_BT_2100_0_HLG,
        ] {
            assert!(
                nclx_profile(ColorPrimaries::ITU_R_BT_2020_2_and_2100_0, transfer)
                    .unwrap_err()
                    .to_string()
                    .contains("HDR")
            );
        }
        assert!(
            nclx_profile(
                ColorPrimaries::Unspecified,
                TransferCharacteristics::Unspecified
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    #[ignore = "requires a libheif AV1 encoder"]
    fn ten_bit_avif_keeps_more_than_eight_bit_precision() {
        use libheif_rs::{
            Channel, ColorSpace, CompressionFormat, EncoderQuality, HeifContext, Image, LibHeif,
            RgbChroma,
        };
        let mut image = Image::new(32, 32, ColorSpace::Rgb(RgbChroma::HdrRgbLe)).unwrap();
        image
            .create_plane(Channel::Interleaved, 32, 32, 10)
            .unwrap();
        {
            let planes = image.planes_mut();
            let plane = planes.interleaved.unwrap();
            for y in 0..32usize {
                for x in 0..32usize {
                    for channel in 0..3usize {
                        let value = (300 + x as u16 + channel as u16 * 100).to_le_bytes();
                        let offset = y * plane.stride + x * 6 + channel * 2;
                        plane.data[offset..offset + 2].copy_from_slice(&value);
                    }
                }
            }
        }
        let lib = LibHeif::new();
        let mut encoder = lib.encoder_for_format(CompressionFormat::Av1).unwrap();
        encoder.set_quality(EncoderQuality::LossLess).unwrap();
        let mut color_profile = libheif_rs::ColorProfileNCLX::new().unwrap();
        color_profile.set_color_primaries(libheif_rs::ColorPrimaries::SMPTE_EG_432_1);
        image.set_color_profile_nclx(&color_profile).unwrap();
        let mut context = HeifContext::new().unwrap();
        let mut options = libheif_rs::EncodingOptions::new().unwrap();
        options.set_mac_os_compatibility_workaround_no_nclx_profile(false);
        context
            .encode_image(&image, &mut encoder, Some(options))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("precision.avif");
        let mut encoded = context.write_to_bytes().unwrap();
        let nclx = encoded
            .windows(4)
            .position(|bytes| bytes == b"nclx")
            .expect("AVIF nclx box");
        encoded[nclx + 4..nclx + 6].copy_from_slice(&12u16.to_be_bytes());
        encoded[nclx + 6..nclx + 8].copy_from_slice(&13u16.to_be_bytes());
        std::fs::write(&path, &encoded).unwrap();
        let decoded = decode_heif(&path).unwrap();
        let profile = moxcms::ColorProfile::new_from_slice(decoded.icc.as_ref().unwrap()).unwrap();
        assert!(
            profile
                .transform_matrix(&moxcms::ColorProfile::new_srgb())
                .v[0][0]
                > 1.2
        );
        assert_eq!(decoded.image.color(), image::ColorType::Rgba16);
        let pixels = decoded.image.into_rgba16();
        let values: std::collections::HashSet<_> =
            (0..32).map(|x| pixels.get_pixel(x, 0)[0]).collect();
        assert!(
            values.len() > 16,
            "10-bit gradient should not be reduced to 8-bit steps: {} distinct values",
            values.len()
        );
        for transfer in [16u16, 18] {
            encoded[nclx + 6..nclx + 8].copy_from_slice(&transfer.to_be_bytes());
            std::fs::write(&path, &encoded).unwrap();
            assert!(
                decode_heif(&path)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("HDR")
            );
        }
    }
}
