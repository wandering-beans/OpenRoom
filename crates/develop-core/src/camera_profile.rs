//! Bounded DCP reader. Profiles are read from the user's installation, never bundled.
use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub type Matrix = [[f32; 3]; 3];
#[derive(Clone, Debug)]
pub struct HsvTable {
    pub dimensions: [usize; 3],
    pub values: Vec<[f32; 3]>,
    pub srgb_encoding: bool,
}
#[derive(Clone, Debug)]
pub struct CameraProfile {
    pub name: String,
    pub calibration_signature: String,
    pub color: [Matrix; 2],
    pub forward: Option<[Matrix; 2]>,
    pub temperatures: [f32; 2],
    pub hue_sat: [Option<HsvTable>; 2],
    pub look: Option<HsvTable>,
    pub curve: Vec<[f32; 2]>,
    pub exposure_offset: f32,
}

fn candidates(camera: &str) -> Vec<PathBuf> {
    if camera.contains(['/', '\\']) {
        return Vec::new();
    }
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("OPENROOM_CAMERA_PROFILES") {
        roots.push(PathBuf::from(path));
    }
    if cfg!(target_os = "macos") {
        roots.push(PathBuf::from(
            "/Library/Application Support/Adobe/CameraRaw/CameraProfiles/Adobe Standard",
        ));
    }
    if let Some(path) = std::env::var_os("PROGRAMDATA") {
        roots.push(PathBuf::from(path).join("Adobe/CameraRaw/CameraProfiles/Adobe Standard"));
    }
    if let Some(path) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(path).join(".local/share/OpenRoom/CameraProfiles"));
    }
    roots
        .into_iter()
        .map(|root| root.join(format!("{camera} Adobe Standard.dcp")))
        .collect()
}
pub fn installed(camera: &str) -> Option<CameraProfile> {
    candidates(camera)
        .into_iter()
        .find_map(|path| read(&path).ok())
}
pub fn signature(camera: &str) -> String {
    candidates(camera)
        .into_iter()
        .filter_map(|path| {
            let metadata = std::fs::metadata(&path).ok()?;
            Some(format!(
                "{}:{}:{:?}",
                path.display(),
                metadata.len(),
                metadata.modified().ok()?
            ))
        })
        .collect::<Vec<_>>()
        .join(";")
}

pub fn read(path: &Path) -> Result<CameraProfile> {
    ensure!(
        std::fs::metadata(path)?.len() <= 4 * 1024 * 1024,
        "DCP exceeds 4 MiB"
    );
    parse(&std::fs::read(path)?)
}
fn parse(data: &[u8]) -> Result<CameraProfile> {
    ensure!(data.len() >= 8, "Truncated DCP header");
    let little = &data[..2] == b"II";
    ensure!(little || &data[..2] == b"MM", "Invalid DCP byte order");
    let word = |offset: usize| -> Result<u16> {
        let bytes: [u8; 2] = data
            .get(offset..offset + 2)
            .ok_or_else(|| anyhow::anyhow!("Truncated DCP"))?
            .try_into()?;
        Ok(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let long = |offset: usize| -> Result<u32> {
        let bytes: [u8; 4] = data
            .get(offset..offset + 4)
            .ok_or_else(|| anyhow::anyhow!("Truncated DCP"))?
            .try_into()?;
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    ensure!([42, 0x4352].contains(&word(2)?), "Not a DCP/TIFF profile");
    let offset = long(4)? as usize;
    let count = word(offset)? as usize;
    ensure!(count <= 256, "Too many DCP tags");
    let mut tags = HashMap::<u16, Vec<f32>>::new();
    let mut name = String::from("Camera profile");
    let mut calibration_signature = String::new();
    for i in 0..count {
        let entry = offset + 2 + i * 12;
        let tag = word(entry)?;
        let kind = word(entry + 2)?;
        let count = long(entry + 4)? as usize;
        let size = match kind {
            1 | 2 => 1,
            3 => 2,
            4 | 9 | 11 => 4,
            5 | 10 => 8,
            _ => continue,
        };
        let length = count
            .checked_mul(size)
            .ok_or_else(|| anyhow::anyhow!("DCP size overflow"))?;
        ensure!(length <= data.len(), "DCP tag exceeds file");
        let start = if length <= 4 {
            entry + 8
        } else {
            long(entry + 8)? as usize
        };
        ensure!(
            start
                .checked_add(length)
                .is_some_and(|end| end <= data.len()),
            "DCP tag out of bounds"
        );
        if tag == 50932 {
            calibration_signature = String::from_utf8_lossy(&data[start..start + length])
                .trim_end_matches('\0')
                .to_string();
        }
        if tag == 50936 {
            name = String::from_utf8_lossy(&data[start..start + length])
                .trim_end_matches('\0')
                .to_string();
        }
        if ![
            50721, 50722, 50778, 50779, 50964, 50965, 50937, 50938, 50939, 50940, 50981, 50982,
            51107, 51108, 51109,
        ]
        .contains(&tag)
        {
            continue;
        }
        let mut values = Vec::with_capacity(count);
        for n in 0..count {
            let pos = start + n * size;
            let value = match kind {
                3 => word(pos)? as f32,
                4 => long(pos)? as f32,
                9 => (long(pos)? as i32) as f32,
                11 => f32::from_bits(long(pos)?),
                5 => long(pos)? as f32 / long(pos + 4)? as f32,
                10 => (long(pos)? as i32) as f32 / (long(pos + 4)? as i32) as f32,
                _ => anyhow::bail!("Invalid DCP numeric type"),
            };
            ensure!(value.is_finite(), "Non-finite DCP value");
            values.push(value);
        }
        tags.insert(tag, values);
    }
    let matrix = |tag| -> Result<Matrix> {
        let values = tags
            .get(&tag)
            .ok_or_else(|| anyhow::anyhow!("Missing DCP matrix {tag}"))?;
        ensure!(
            values.len() == 9 && values.iter().all(|v| v.abs() <= 100.),
            "Invalid DCP matrix"
        );
        let m: Matrix = std::array::from_fn(|i| std::array::from_fn(|j| values[i * 3 + j]));
        let [a, b, c] = m;
        let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
        ensure!(det.abs() > 1e-10, "Singular DCP matrix");
        Ok(m)
    };
    let scalar = |tag, default| {
        tags.get(&tag)
            .and_then(|v| v.first())
            .copied()
            .unwrap_or(default)
    };
    let temperature = |tag| -> Result<f32> {
        Ok(match scalar(tag, 21.) as u16 {
            17 => 2856.,
            21 => 6504.,
            23 => 5003.,
            20 => 5503.,
            22 => 7504.,
            _ => anyhow::bail!("Unsupported DCP illuminant"),
        })
    };
    let table = |dims, entries, encoding| -> Result<Option<HsvTable>> {
        let Some(values) = tags.get(&entries) else {
            return Ok(None);
        };
        let dims = tags
            .get(&dims)
            .ok_or_else(|| anyhow::anyhow!("Missing DCP table dimensions"))?;
        ensure!(
            dims.len() == 3
                && dims
                    .iter()
                    .all(|v| *v >= 1. && *v <= 256. && v.fract() == 0.)
                && dims[1] >= 2.,
            "Invalid DCP table dimensions"
        );
        let dimensions = [dims[0] as usize, dims[1] as usize, dims[2] as usize];
        ensure!(
            values.len() == dimensions.iter().product::<usize>() * 3,
            "DCP table length mismatch"
        );
        ensure!(
            [0., 1.].contains(&scalar(encoding, 0.)),
            "Unsupported DCP table encoding"
        );
        Ok(Some(HsvTable {
            dimensions,
            values: values.as_chunks::<3>().0.to_vec(),
            srgb_encoding: scalar(encoding, 0.) == 1.,
        }))
    };
    let points = tags.get(&50940).cloned().unwrap_or_default();
    ensure!(
        points.len() % 2 == 0 && points.iter().all(|v| (0. ..=1.).contains(v)),
        "Invalid DCP tone curve"
    );
    let curve = points.as_chunks::<2>().0.to_vec();
    ensure!(
        curve.windows(2).all(|p| p[0][0] < p[1][0]),
        "Unordered DCP tone curve"
    );
    let color1 = matrix(50721)?;
    Ok(CameraProfile {
        name,
        calibration_signature,
        color: [
            color1,
            if tags.contains_key(&50722) {
                matrix(50722)?
            } else {
                color1
            },
        ],
        forward: if tags.contains_key(&50964) {
            let first = matrix(50964)?;
            Some([
                first,
                if tags.contains_key(&50965) {
                    matrix(50965)?
                } else {
                    first
                },
            ])
        } else {
            None
        },
        temperatures: [temperature(50778)?, temperature(50779)?],
        hue_sat: [table(50937, 50938, 51107)?, table(50937, 50939, 51107)?],
        look: table(50981, 50982, 51108)?,
        curve,
        exposure_offset: scalar(51109, 0.).clamp(-10., 10.),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_both_byte_orders_and_rejects_singular_matrices() {
        for little in [true, false] {
            let word = |v: u16| {
                if little {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                }
            };
            let long = |v: u32| {
                if little {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                }
            };
            let mut bytes = if little {
                b"II".to_vec()
            } else {
                b"MM".to_vec()
            };
            bytes.extend(word(0x4352));
            bytes.extend(long(8));
            bytes.extend(word(1));
            bytes.extend(word(50721));
            bytes.extend(word(11));
            bytes.extend(long(9));
            bytes.extend(long(26));
            bytes.extend(long(0));
            for value in [1f32, 0., 0., 0., 1., 0., 0., 0., 1.] {
                bytes.extend(long(value.to_bits()));
            }
            let profile = parse(&bytes).unwrap();
            assert_eq!(profile.color[0], [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
            assert_eq!(profile.color[0], profile.color[1]);
            bytes[26..30].fill(0);
            assert!(parse(&bytes).is_err());
        }
    }
    #[test]
    fn rejects_truncated_and_oversized_profile_entries() {
        for bytes in [
            vec![],
            b"IIRC\x08\0\0\0\xff\xff".to_vec(),
            b"IIRC\xff\xff\xff\xff".to_vec(),
        ] {
            assert!(parse(&bytes).is_err());
        }
        let mut bytes = b"IIRC\x08\0\0\0\x01\0".to_vec();
        bytes.extend_from_slice(&50721u16.to_le_bytes());
        bytes.extend_from_slice(&10u16.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(parse(&bytes).is_err());
    }
}
