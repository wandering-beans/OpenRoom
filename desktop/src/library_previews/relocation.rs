use super::{Images, Store, VERSION};
use advanced_explorer::{Catalog, previews::PreviewRecord, relink::RelinkReport};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Binding {
    file_id: String,
    // Include the photo ID so importing a replacement at the old path owns separate files.
    pub(super) storage_stamp: String,
    pub(super) thumbnail_crc: u32,
    pub(super) large_crc: u32,
}

impl Binding {
    pub(super) fn new(path: &Path, id: i64, stamp: &str) -> Option<Self> {
        Some(Self {
            file_id: file_id(path)?,
            storage_stamp: format!("{id}:{stamp}"),
            thumbnail_crc: 0,
            large_crc: 0,
        })
    }

    pub(super) fn matches(&self, path: &Path) -> bool {
        file_id(path).as_ref() == Some(&self.file_id)
    }
}

fn fingerprint(id: String, info: &std::fs::Metadata) -> Option<String> {
    let nanos = |t: std::time::SystemTime| {
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
    };
    let modified = nanos(info.modified().ok()?)?;
    let birth = info.created().ok().and_then(nanos);
    Some(format!("{id}:{}:{modified}:{birth:?}", info.len()))
}

#[cfg(windows)]
fn file_id(path: &Path) -> Option<String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx},
    };
    let file = File::open(path).ok()?;
    let mut info = FILE_ID_INFO::default();
    // The handle stays open through the call; unsupported volumes simply cannot reuse previews.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            (&raw mut info).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
        .ok()?;
    }
    let id = u128::from_le_bytes(info.FileId.Identifier);
    if id == 0 || info.VolumeSerialNumber == 0 {
        return None;
    }
    fingerprint(
        format!("win:{}:{id}", info.VolumeSerialNumber),
        &file.metadata().ok()?,
    )
}

#[cfg(unix)]
fn file_id(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let info = std::fs::metadata(path).ok()?;
    if info.ino() == 0 {
        return None;
    }
    fingerprint(format!("unix:{}:{}", info.dev(), info.ino()), &info)
}

impl Store {
    pub(crate) fn relink_folder(
        &self,
        catalog: &mut Catalog,
        root_id: i64,
        folder: &Path,
        directory: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<RelinkReport> {
        let _workspace = self.workspace.write().unwrap();
        catalog.relink_folder_with_previews(
            root_id,
            folder,
            directory,
            cancelled,
            |record, path| self.reusable_images(record, path),
        )
    }

    pub(super) fn reusable_images(&self, record: &PreviewRecord, path: &Path) -> Option<String> {
        let mut images: Images = serde_json::from_str(record.data.as_ref()?).ok()?;
        let binding = images.binding.as_ref()?;
        if !binding.matches(path)
            || images.version != VERSION
            || !["avif", "jpg"].contains(&images.extension.as_str())
            || (images.extension == "avif" && !self.avif.load(std::sync::atomic::Ordering::SeqCst))
        {
            return None;
        }
        let render = advanced_explorer_develop::RENDER_VERSION;
        let raw = advanced_explorer_media::is_raw(path);
        if raw {
            if images.render_version != format!("{render}-raw-library-v1")
                || !(images.raw_developed || images.camera_fallback)
            {
                return None;
            }
        } else if images.render_version != render {
            return None;
        }
        let complete = |bound, bytes, crc| {
            let path = self.path(
                &binding.storage_stamp,
                &images.extension,
                bound,
                &images.render_version,
            );
            preview_intact(&path, &self.directory, bytes, crc)
        };
        if !complete(1600, images.large_bytes, binding.large_crc) {
            return None;
        }
        if !complete(384, images.thumbnail_bytes, binding.thumbnail_crc) {
            images.thumbnail_bytes = 0;
        }
        serde_json::to_string(&images).ok()
    }
}

fn preview_intact(path: &Path, directory: &Path, bytes: u64, crc: u32) -> bool {
    fn verify(path: &Path, directory: &Path, bytes: u64, crc: u32) -> std::io::Result<bool> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file()
            || bytes == 0
            || meta.len() != bytes
            || !path.canonicalize()?.starts_with(directory.canonicalize()?)
        {
            return Ok(false);
        }
        let mut file = File::open(path)?;
        let mut sum = crc32fast::Hasher::new();
        let mut buf = [0; 16384];
        loop {
            let read = file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            sum.update(&buf[..read]);
        }
        Ok(sum.finalize() == crc)
    }
    verify(path, directory, bytes, crc).unwrap_or(false)
}
