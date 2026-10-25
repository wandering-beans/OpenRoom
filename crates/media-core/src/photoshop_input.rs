//! Precision-preserving saved composites. Layers remain the external editor's responsibility.
use crate::develop_input::DecodedPhoto;
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, Rgba, Rgba32FImage};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
fn integer(reader: &mut Cursor<&[u8]>, bytes: usize) -> Result<u64> {
    let mut value = [0; 8];
    reader.read_exact(&mut value[8 - bytes..])?;
    Ok(u64::from_be_bytes(value))
}
fn skip_section(reader: &mut Cursor<&[u8]>, bytes: usize) -> Result<()> {
    let length = integer(reader, bytes)?;
    let end = reader
        .position()
        .checked_add(length)
        .context("PSD section overflow")?;
    ensure!(
        end <= reader.get_ref().len() as u64,
        "Truncated PSD section"
    );
    reader.set_position(end);
    Ok(())
}
pub fn decode(path: &Path) -> Result<DecodedPhoto> {
    let bytes = std::fs::read(path)?;
    super::validate_photoshop_composite(&bytes)?;
    let mut reader = Cursor::new(bytes.as_slice());
    reader.set_position(4);
    let version = integer(&mut reader, 2)?;
    reader.set_position(12);
    let channels = integer(&mut reader, 2)? as usize;
    let height = integer(&mut reader, 4)? as u32;
    let width = integer(&mut reader, 4)? as u32;
    let depth = integer(&mut reader, 2)? as usize;
    let mode = integer(&mut reader, 2)?;
    ensure!(
        u64::from(width) * u64::from(height) <= 64_000_000,
        "PSD exceeds 64 megapixels"
    );
    ensure!(
        [8, 16, 32].contains(&depth) && [1, 3].contains(&mode),
        "現像はRGB・グレースケールPSDの8/16/32bitに対応します"
    );
    let color_channels = if mode == 3 { 3 } else { 1 };
    ensure!(channels >= color_channels, "Missing PSD channels");
    skip_section(&mut reader, 4)?;
    let resources = integer(&mut reader, 4)?;
    let resource_end = reader
        .position()
        .checked_add(resources)
        .context("PSD resource overflow")?;
    ensure!(resource_end <= bytes.len() as u64, "Truncated resources");
    let mut icc = None;
    while reader.position() < resource_end {
        let mut signature = [0; 4];
        reader.read_exact(&mut signature)?;
        ensure!(&signature == b"8BIM", "Invalid resource signature");
        let id = integer(&mut reader, 2)?;
        let name_len = integer(&mut reader, 1)?;
        reader.seek(SeekFrom::Current(((name_len + 2) & !1) as i64 - 1))?;
        let size = integer(&mut reader, 4)?;
        ensure!(
            reader.position() + size + (size % 2) <= resource_end,
            "Truncated resource"
        );
        if id == 1039 {
            ensure!(size <= 16 * 1024 * 1024, "ICC exceeds 16 MiB");
            let mut data = vec![0; size as usize];
            reader.read_exact(&mut data)?;
            icc = Some(data);
        } else {
            reader.seek(SeekFrom::Current(size as i64))?;
        }
        reader.seek(SeekFrom::Current((size % 2) as i64))?;
    }
    skip_section(&mut reader, if version == 2 { 8 } else { 4 })?;
    let compression = integer(&mut reader, 2)?;
    let sample_bytes = depth / 8;
    let row_bytes = width as usize * sample_bytes;
    let expected = row_bytes
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(channels))
        .context("PSD allocation overflow")?;
    ensure!(
        expected <= 1024 * 1024 * 1024,
        "PSD composite exceeds 1 GiB"
    );
    let mut planes = Vec::with_capacity(expected);
    match compression {
        0 => {
            planes.resize(expected, 0);
            reader.read_exact(&mut planes)?;
        }
        1 => {
            let lengths = (0..channels * height as usize)
                .map(|_| integer(&mut reader, if version == 2 { 4 } else { 2 }))
                .collect::<Result<Vec<_>>>()?;
            for length in lengths {
                let end = reader
                    .position()
                    .checked_add(length)
                    .context("PSD RLE overflow")?;
                ensure!(end <= bytes.len() as u64, "Truncated PSD RLE");
                let first = planes.len();
                while reader.position() < end {
                    let control = integer(&mut reader, 1)? as u8 as i8;
                    if control >= 0 {
                        let count = control as usize + 1;
                        ensure!(
                            reader.position() + count as u64 <= end
                                && planes.len() + count <= first + row_bytes,
                            "Invalid RLE literal"
                        );
                        let offset = planes.len();
                        planes.resize(offset + count, 0);
                        reader.read_exact(&mut planes[offset..])?;
                    } else if control != -128 {
                        let count = (1 - i16::from(control)) as usize;
                        ensure!(
                            reader.position() < end && planes.len() + count <= first + row_bytes,
                            "Invalid RLE run"
                        );
                        let value = integer(&mut reader, 1)? as u8;
                        planes.resize(planes.len() + count, value);
                    }
                }
                ensure!(planes.len() == first + row_bytes, "Invalid RLE row size");
            }
        }
        2 | 3 => {
            let decoder = flate2::read::ZlibDecoder::new(&bytes[reader.position() as usize..]);
            decoder.take(expected as u64 + 1).read_to_end(&mut planes)?;
            ensure!(planes.len() == expected, "Invalid ZIP composite size");
            if compression == 3 {
                for row in planes.chunks_mut(row_bytes) {
                    if depth == 16 {
                        let mut previous = 0u16;
                        for pair in row.as_chunks_mut::<2>().0 {
                            let value =
                                u16::from_be_bytes([pair[0], pair[1]]).wrapping_add(previous);
                            pair.copy_from_slice(&value.to_be_bytes());
                            previous = value;
                        }
                    } else {
                        for i in 1..row.len() {
                            row[i] = row[i].wrapping_add(row[i - 1]);
                        }
                        if depth == 32 {
                            let shuffled = row.to_vec();
                            for x in 0..width as usize {
                                for byte in 0..4 {
                                    row[x * 4 + byte] = shuffled[byte * width as usize + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => anyhow::bail!("Unsupported PSD compression"),
    }
    let count = width as usize * height as usize;
    let sample = |channel: usize, index: usize| -> f32 {
        let offset = (channel * count + index) * sample_bytes;
        match depth {
            8 => planes[offset] as f32 / 255.,
            16 => u16::from_be_bytes([planes[offset], planes[offset + 1]]) as f32 / 65535.,
            _ => f32::from_be_bytes(planes[offset..offset + 4].try_into().unwrap()),
        }
    };
    let pixels = Rgba32FImage::from_fn(width, height, |x, y| {
        let i = y as usize * width as usize + x as usize;
        let rgb = if mode == 3 {
            [sample(0, i), sample(1, i), sample(2, i)]
        } else {
            [sample(0, i); 3]
        };
        Rgba([
            rgb[0],
            rgb[1],
            rgb[2],
            if channels > color_channels {
                sample(color_channels, i)
            } else {
                1.
            },
        ])
    });
    Ok(DecodedPhoto {
        image: DynamicImage::ImageRgba32F(pixels),
        raw: None,
        linear: depth == 32,
        icc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(depth: u16, compression: u16, values: &[u8]) -> Vec<u8> {
        let mut bytes = b"8BPS".to_vec();
        bytes.extend(1u16.to_be_bytes());
        bytes.extend([0; 6]);
        bytes.extend(3u16.to_be_bytes());
        bytes.extend(1u32.to_be_bytes());
        bytes.extend(2u32.to_be_bytes());
        bytes.extend(depth.to_be_bytes());
        bytes.extend(3u16.to_be_bytes());
        bytes.extend([0; 12]);
        bytes.extend(compression.to_be_bytes());
        bytes.extend(values);
        bytes
    }
    #[test]
    fn composites_keep_sub_8bit_values_and_hdr_headroom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("precision.psd");
        let values: Vec<u8> = [10000u16, 10001, 20000, 20001, 30000, 30001]
            .into_iter()
            .flat_map(u16::to_be_bytes)
            .collect();
        for compression in [0, 2, 3] {
            let mut raw = values.clone();
            if compression == 3 {
                for row in raw.as_chunks_mut::<4>().0 {
                    let a = u16::from_be_bytes([row[0], row[1]]);
                    let b = u16::from_be_bytes([row[2], row[3]]);
                    row[2..].copy_from_slice(&b.wrapping_sub(a).to_be_bytes());
                }
            }
            if compression > 0 {
                use std::io::Write;
                let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
                encoder.write_all(&raw).unwrap();
                raw = encoder.finish().unwrap();
            }
            std::fs::write(&path, fixture(16, compression, &raw)).unwrap();
            let decoded = decode(&path).unwrap().image.into_rgba32f();
            assert!(decoded.get_pixel(1, 0)[0] > decoded.get_pixel(0, 0)[0]);
            assert!((decoded.get_pixel(0, 0)[0] - 10000. / 65535.).abs() < 1e-7);
        }
        let values: Vec<u8> = [0.5f32, 2., 0.4, 3., 0.3, 4.]
            .into_iter()
            .flat_map(f32::to_be_bytes)
            .collect();
        std::fs::write(&path, fixture(32, 0, &values)).unwrap();
        let decoded = decode(&path).unwrap();
        assert!(decoded.linear);
        assert_eq!(decoded.image.into_rgba32f().get_pixel(1, 0)[2], 4.);
        let mut broken = fixture(16, 0, &[0; 3]);
        std::fs::write(&path, &broken).unwrap();
        assert!(decode(&path).is_err());
        broken[24..26].copy_from_slice(&4u16.to_be_bytes());
        std::fs::write(&path, broken).unwrap();
        assert!(decode(&path).is_err());
    }
}
