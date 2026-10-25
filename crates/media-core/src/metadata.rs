//! Photo properties are read on selection, never during catalog scans or scrolling.
use anyhow::{Result, ensure};
use image::ImageDecoder;
use rawler::{
    exif::Exif,
    formats::tiff::{GenericTiffReader, Rational, reader::TiffReader},
};
use std::{
    collections::BTreeMap,
    io::{BufReader, Cursor},
    path::Path,
};
pub type PhotoMetadata = BTreeMap<String, String>;

pub fn read(path: &Path) -> Result<PhotoMetadata> {
    ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024 * 1024,
        "Metadata input exceeds 1 GiB"
    );
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (exif, make, model) = match ext.as_str() {
        ext if crate::RAW_EXTENSIONS.contains(&ext) => {
            let source = rawler::rawsource::RawSource::new_preview(path)?;
            let metadata =
                rawler::get_decoder(&source)?.raw_metadata(&source, &Default::default())?;
            (metadata.exif, metadata.make, metadata.model)
        }
        "tif" | "tiff" => {
            let mut file = BufReader::new(std::fs::File::open(path)?);
            let reader = GenericTiffReader::new(&mut file, 0, 0, Some(1), &[34665, 34853])?;
            from_tiff(&reader)?
        }
        "heic" | "heif" | "hif" | "avif" => {
            let stream = libheif_rs::StreamReader::new(
                std::fs::File::open(path)?,
                std::fs::metadata(path)?.len(),
            );
            let ctx = libheif_rs::HeifContext::read_from_reader(Box::new(stream))?;
            let handle = ctx.primary_image_handle()?;
            let mut ids = [0; 1];
            if handle.metadata_block_ids(&mut ids, b"Exif") == 0 {
                return Ok(PhotoMetadata::new());
            }
            ensure!(
                handle.metadata_size(ids[0]) <= 16 * 1024 * 1024,
                "EXIF exceeds 16 MiB"
            );
            let bytes = handle.metadata(ids[0])?;
            ensure!(bytes.len() >= 4, "Truncated HEIF EXIF");
            let offset = 4usize
                .checked_add(u32::from_be_bytes(bytes[..4].try_into()?) as usize)
                .ok_or_else(|| anyhow::anyhow!("Invalid HEIF EXIF offset"))?;
            parse_exif(
                bytes
                    .get(offset..)
                    .ok_or_else(|| anyhow::anyhow!("Invalid HEIF EXIF offset"))?,
            )?
        }
        "jpg" | "jpeg" | "png" => {
            let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
            reader.limits(image::Limits::default());
            let mut decoder = reader.into_decoder()?;
            let Some(bytes) = decoder.exif_metadata()? else {
                return Ok(PhotoMetadata::new());
            };
            parse_exif(&bytes)?
        }
        _ => return Ok(PhotoMetadata::new()),
    };
    Ok(properties(exif, make, model))
}

fn parse_exif(bytes: &[u8]) -> Result<(Exif, String, String)> {
    ensure!(bytes.len() <= 16 * 1024 * 1024, "EXIF exceeds 16 MiB");
    let bytes = bytes.strip_prefix(b"Exif\0\0").unwrap_or(bytes);
    ensure!(bytes.len() >= 8, "Truncated EXIF");
    let reader = GenericTiffReader::new(&mut Cursor::new(bytes), 0, 0, Some(1), &[34665, 34853])?;
    from_tiff(&reader)
}
fn from_tiff(reader: &GenericTiffReader) -> Result<(Exif, String, String)> {
    let ifd = reader.root_ifd();
    let text = |tag: u16| {
        ifd.get_entry(tag)
            .and_then(|e| e.value.as_string())
            .cloned()
            .unwrap_or_default()
    };
    Ok((Exif::new(ifd)?, text(271), text(272)))
}
fn number(r: Rational) -> Option<f64> {
    (r.d != 0).then(|| f64::from(r.n) / f64::from(r.d))
}
fn properties(exif: Exif, make: String, model: String) -> PhotoMetadata {
    let mut fields = PhotoMetadata::new();
    let mut add = |key: &str, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.trim().is_empty()) {
            fields.insert(
                key.into(),
                value
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect(),
            );
        }
    };
    add("メーカー", Some(make));
    add("カメラ", Some(model));
    add("レンズ", exif.lens_model);
    add("撮影日時", exif.date_time_original.or(exif.create_date));
    add("タイムゾーン", exif.offset_time_original);
    add(
        "ISO",
        exif.iso_speed
            .or(exif.iso_speed_ratings.map(u32::from))
            .map(|v| v.to_string()),
    );
    add(
        "絞り",
        exif.fnumber.and_then(number).map(|v| format!("f/{v:.1}")),
    );
    add(
        "焦点距離",
        exif.focal_length
            .and_then(number)
            .map(|v| format!("{v:.1} mm")),
    );
    add(
        "シャッター速度",
        exif.exposure_time.and_then(|r| {
            number(r).map(|v| {
                if v > 0. && v < 1. {
                    format!("1/{:.0} 秒", 1. / v)
                } else {
                    format!("{v:.3} 秒")
                }
            })
        }),
    );
    add("撮影者", exif.artist);
    add("著作権", exif.copyright);
    if let Some(gps) = exif.gps {
        let coordinate =
            |values: Option<[Rational; 3]>, reference: Option<String>| -> Option<String> {
                let [d, m, s] = values?;
                Some(format!(
                    "{:.6} {}",
                    number(d)? + number(m)? / 60. + number(s)? / 3600.,
                    reference.unwrap_or_default()
                ))
            };
        add(
            "GPS 緯度",
            coordinate(gps.gps_latitude, gps.gps_latitude_ref),
        );
        add(
            "GPS 経度",
            coordinate(gps.gps_longitude, gps.gps_longitude_ref),
        );
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn photo_properties_without_decoding_or_modifying_pixels() {
        use rawler::formats::tiff::TiffWriter;
        let mut data = Cursor::new(Vec::new());
        let mut writer = TiffWriter::new(&mut data).unwrap();
        let mut exif = writer.new_directory();
        exif.add_tag(36867u16, "2020:01:02 03:04:05");
        exif.add_tag(33437u16, Rational { n: 28, d: 10 });
        exif.add_tag(34855u16, 400u16);
        let offset = exif.build(&mut writer).unwrap();
        let mut root = writer.new_directory();
        root.add_tag(271u16, "Test camera maker");
        root.add_tag(272u16, "Test camera");
        root.add_tag(34665u16, offset);
        writer.build(root).unwrap();
        let bytes = data.into_inner();
        let (exif, make, model) = parse_exif(&bytes).unwrap();
        let values = properties(exif, make, model);
        assert_eq!(values["カメラ"], "Test camera");
        assert_eq!(values["ISO"], "400");
        assert_eq!(values["絞り"], "f/2.8");
        assert_eq!(values["撮影日時"], "2020:01:02 03:04:05");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.tiff");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read(&path).unwrap(), values);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        use image::ImageEncoder;
        let pixels = [32u8; 8 * 8 * 3];
        for extension in ["jpg", "png"] {
            let file = dir.path().join(format!("photo.{extension}"));
            let mut output = Vec::new();
            if extension == "jpg" {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut output);
                encoder.set_exif_metadata(bytes.clone()).unwrap();
                encoder
                    .write_image(&pixels, 8, 8, image::ExtendedColorType::Rgb8)
                    .unwrap();
            } else {
                let mut encoder = image::codecs::png::PngEncoder::new(&mut output);
                encoder.set_exif_metadata(bytes.clone()).unwrap();
                encoder
                    .write_image(&pixels, 8, 8, image::ExtendedColorType::Rgb8)
                    .unwrap();
            }
            std::fs::write(&file, &output).unwrap();
            assert_eq!(read(&file).unwrap(), values);
            assert_eq!(std::fs::read(&file).unwrap(), output);
        }

        assert!(parse_exif(b"invalid").is_err());
        assert!(number(Rational { n: 1, d: 0 }).is_none());
    }
}
