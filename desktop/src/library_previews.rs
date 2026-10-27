//! Catalog-owned library images, separate from disposable development working files.
use crate::{AppState, Catalog, Preview};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

const VERSION: &str = "library-avif-srgb-v1";
const IMAGE_LIMIT: u64 = 7_500_000_000;
pub(crate) const WORK_LIMIT: u64 = 1_000_000_000;
const TOTAL_LIMIT: u64 = 9_500_000_000; // Leave 500 MB for database logs and atomic replacements.
pub(crate) const IMPORT_RESERVE: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
struct StorageFull(&'static str);
impl std::fmt::Display for StorageFull {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for StorageFull {}

#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub total: usize,
    pub ready: usize,
    pub generated: usize,
    pub retained: usize,
    pub failed: usize,
    pub cancelled: bool,
    pub paused: bool,
    pub error: Option<String>,
    pub last_ready_id: Option<i64>,
}

#[derive(Serialize, Deserialize)]
struct Images {
    version: String,
    #[serde(default)]
    render_version: String,
    #[serde(default)]
    raw_developed: bool,
    #[serde(default)]
    camera_fallback: bool,
    #[serde(default)]
    jpeg_fallback: bool,
    extension: String,
    width: usize,
    height: usize,
    large: [usize; 2],
    thumbnail: [usize; 2],
    large_bytes: u64,
    thumbnail_bytes: u64,
}

struct Usage {
    files: HashMap<PathBuf, u64>,
    bytes: u64,
    reserved: u64,
}

pub struct Store {
    pub updates: crate::deferred_previews::Updates,
    pub directory: PathBuf,
    pub ready: tokio::sync::Notify,
    usage: Mutex<Usage>,
    queue: crate::preview_queue::Queue,
    workspace: std::sync::RwLock<()>,
    early: Mutex<HashMap<i64, (String, serde_json::Value)>>,
    avif: AtomicBool,
    encoder_available: bool,
    protected: Mutex<Option<PathBuf>>,
}

struct Reservation<'a> {
    store: &'a Store,
    id: i64,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.store.early.lock().unwrap().remove(&self.id);
        self.store.usage.lock().unwrap().reserved -= IMPORT_RESERVE;
    }
}

pub fn directory(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(".previews");
    db.with_file_name(name)
}

fn allocated(path: &Path) -> Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file(),
        "プレビューの保存領域に通常ファイル以外があります。"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.len().max(metadata.blocks() * 512))
    }
    #[cfg(not(unix))]
    {
        Ok(metadata.len().div_ceil(4096) * 4096)
    }
}

fn files(directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "プレビューの保存領域にシンボリックリンクは使えません。"
    );
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "プレビューの保存領域にシンボリックリンクがあります。"
        );
        if kind.is_dir() {
            files(&entry.path(), output)?;
        } else if kind.is_file() {
            output.push(entry.path());
        }
    }
    Ok(())
}

fn working_bytes(cache: &Path) -> Result<u64> {
    let mut paths = Vec::new();
    files(cache, &mut paths)?;
    paths
        .iter()
        .try_fold(0, |total, path| Ok(total + allocated(path)?))
}

fn database_bytes(db: &Path) -> Result<u64> {
    let mut total = allocated(db)?;
    for suffix in ["-wal", "-shm"] {
        let mut path = db.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        if path.exists() {
            total += allocated(&path)?;
        }
    }
    Ok(total)
}

// Both callers hold the cache writer lock before reclaiming disposable files.
fn reclaim_work_space(cache: &Path, reserve: u64) -> Result<u64> {
    ensure!(
        reserve <= WORK_LIMIT,
        StorageFull("現像用の作業領域が不足しています。")
    );
    let mut bytes = working_bytes(cache)?;
    if bytes + reserve <= WORK_LIMIT {
        return Ok(bytes);
    }
    let mut paths = Vec::new();
    files(cache, &mut paths)?;
    paths.sort_by_cached_key(|path| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    // Keep both ends of an image handoff; older render outputs are disposable even during a burst.
    let displayed: Vec<_> = paths
        .iter()
        .rev()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("develop-") && name.ends_with(".png"))
        })
        .take(2)
        .cloned()
        .collect();
    for path in paths {
        if bytes + reserve <= WORK_LIMIT {
            break;
        }
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("renderer-"))
        {
            continue;
        }
        if displayed.contains(&path) {
            continue;
        }
        // This lock serializes base writers; decoded TIFFs can be recreated even when recent.
        let decoded_base = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".develop-") && name.ends_with(".tif"));
        let rendered_image = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("develop-") && name.ends_with(".png"));
        if !decoded_base
            && !rendered_image
            && std::fs::metadata(&path)?
                .modified()?
                .elapsed()
                .unwrap_or_default()
                .as_secs()
                < 60
        {
            continue;
        }
        let size = allocated(&path)?;
        std::fs::remove_file(path)?;
        bytes = bytes.saturating_sub(size);
    }
    ensure!(
        bytes + reserve <= WORK_LIMIT,
        StorageFull("現像用の作業領域が不足しています。処理を終えてから再試行してください。")
    );
    Ok(bytes)
}

impl Store {
    pub fn new(db: &Path) -> Result<Self> {
        let directory = directory(db);
        std::fs::create_dir_all(&directory)?;
        ensure!(
            !std::fs::symlink_metadata(&directory)?
                .file_type()
                .is_symlink(),
            "プレビュー保存先にシンボリックリンクは使えません。"
        );
        let mut paths = Vec::new();
        for folder in ["large", "thumbnails"] {
            files(&directory.join(folder), &mut paths)?;
        }
        let mut usage = Usage {
            files: HashMap::new(),
            bytes: 0,
            reserved: 0,
        };
        for path in paths {
            let bytes = allocated(&path)?;
            usage.bytes += bytes;
            usage.files.insert(path, bytes);
        }
        let encoder_available = advanced_explorer_media::preview::avif_available();
        let working = directory.join("working");
        if working.exists() {
            ensure!(
                !std::fs::symlink_metadata(working)?.file_type().is_symlink(),
                "作業領域にシンボリックリンクは使えません。"
            );
        }
        Ok(Self {
            directory,
            ready: Default::default(),
            usage: Mutex::new(usage),
            queue: Default::default(),
            workspace: Default::default(),
            early: Default::default(),
            updates: Default::default(),
            avif: AtomicBool::new(encoder_available),
            encoder_available,
            protected: Mutex::new(None),
        })
    }

    pub fn set_avif_supported(&self, supported: bool) {
        self.avif
            .store(supported && self.encoder_available, Ordering::SeqCst);
    }

    fn key(stamp: &str, render_version: &str) -> String {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (VERSION, stamp).hash(&mut hash);
        if !render_version.is_empty() {
            render_version.hash(&mut hash);
        }
        format!("{:016x}", hash.finish())
    }

    fn path(&self, stamp: &str, extension: &str, bound: u32, render_version: &str) -> PathBuf {
        let key = Self::key(stamp, render_version);
        self.directory
            .join(if bound == 384 { "thumbnails" } else { "large" })
            .join(&key[..2])
            .join(format!("{key}.{extension}"))
    }

    pub fn lookup(&self, catalog: &Catalog, id: i64, bound: u32) -> Result<Option<Preview>> {
        let preview = self.lookup_stored(catalog, id, bound)?;
        if let Some(preview) = &preview {
            let _ = std::fs::File::open(&preview.path)
                .and_then(|file| file.set_modified(SystemTime::now()));
        }
        Ok(preview)
    }

    // Only viewing a preview should refresh its eviction age, not a background import check.
    fn lookup_stored(&self, catalog: &Catalog, id: i64, bound: u32) -> Result<Option<Preview>> {
        ensure!(
            [384, 1600].contains(&bound),
            "ライブラリの表示サイズが不正です。"
        );
        let Some(record) = catalog.library_preview(id)? else {
            return Ok(None);
        };
        if record.stamp != catalog.preview_stamp(id)? {
            if catalog.preview_update(id)?.is_none() {
                return Ok(None);
            }
            let mut old: Vec<serde_json::Value> = serde_json::from_str(&record.stamp)?;
            let mut current: Vec<serde_json::Value> =
                serde_json::from_str(&catalog.preview_stamp(id)?)?;
            old.pop();
            current.pop();
            if old != current {
                return Ok(None);
            }
        }
        let Some(data) = record.data else {
            return Ok(None);
        };
        let Ok(images) = serde_json::from_str::<Images>(&data) else {
            return Ok(None);
        };
        if images.version != VERSION || !["avif", "jpg"].contains(&images.extension.as_str()) {
            return Ok(None);
        }
        // Keep old RAW previews usable offline, but update their tone before online display.
        let raw_version = format!("{}-raw-library-v1", advanced_explorer_develop::RENDER_VERSION);
        if (images.render_version != raw_version
            || (!images.raw_developed && !images.camera_fallback))
            && Self::is_raw(catalog, id)?
            && catalog.source_path(id).is_ok()
        {
            return Ok(None);
        }
        if images.extension == "avif" && !self.avif.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let dimensions = if bound == 384 {
            images.thumbnail
        } else {
            images.large
        };
        // Old imports resized both axes to 384, stretching every non-square photo.
        let aspect_error = (images.thumbnail[0] as u128 * images.large[1] as u128)
            .abs_diff(images.thumbnail[1] as u128 * images.large[0] as u128);
        if bound == 384 && aspect_error > images.large[0].max(images.large[1]) as u128 {
            return Ok(None);
        }
        ensure!(
            dimensions.iter().all(|v| *v > 0 && *v <= bound as usize),
            "プレビューの寸法情報が不正です。"
        );
        let path = self.path(
            &record.stamp,
            &images.extension,
            bound,
            &images.render_version,
        );
        let expected_bytes = if bound == 384 {
            images.thumbnail_bytes
        } else {
            images.large_bytes
        };
        if !path.is_file()
            || std::fs::metadata(&path)?.len() != expected_bytes
            || expected_bytes == 0
        {
            return Ok(None);
        }
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink()
                && std::fs::canonicalize(&path)?
                    .starts_with(std::fs::canonicalize(&self.directory)?),
            "プレビュー保存先の外にはアクセスできません。"
        );
        Ok(Some(Preview {
            path: path.to_string_lossy().into_owned(),
            width: images.width,
            height: images.height,
            render_width: dimensions[0],
            render_height: dimensions[1],
        }))
    }

    pub fn protect(&self, preview: &Preview) {
        *self.protected.lock().unwrap() = Some(PathBuf::from(&preview.path));
    }

    fn remove(usage: &mut Usage, path: &Path) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(bytes) = usage.files.remove(path) {
            usage.bytes = usage.bytes.saturating_sub(bytes);
        }
        Ok(())
    }

    fn reserve(&self, state: &AppState, usage: &mut Usage, bytes: u64, evict: bool) -> Result<()> {
        let other = database_bytes(&state.db)? + working_bytes(&state.cache)?;
        let limit = IMAGE_LIMIT.min(TOTAL_LIMIT.saturating_sub(other));
        if usage.bytes + bytes <= limit {
            return Ok(());
        }
        if evict {
            let mut candidates: Vec<_> = usage
                .files
                .keys()
                .filter(|path| path.starts_with(self.directory.join("large")))
                .filter_map(|path| {
                    std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .map(|time| (path.clone(), time))
                })
                .filter(|(_, time)| time.elapsed().unwrap_or_default().as_secs() >= 60)
                .collect();
            candidates.sort_by_key(|(_, time)| *time);
            for (path, _) in candidates {
                if self.protected.lock().unwrap().as_ref() == Some(&path) {
                    continue;
                }
                Self::remove(usage, &path)?;
                if usage.bytes + bytes <= limit {
                    break;
                }
            }
        }
        ensure!(
            usage.bytes + bytes <= limit,
            StorageFull(
                "プレビューの保存容量が上限に達しました。カタログを分けて取り込んでください。"
            )
        );
        Ok(())
    }

    pub(crate) fn discard_work_directory(&self, path: &Path) {
        let _workspace = self.workspace.write().unwrap();
        let _ = std::fs::remove_dir_all(path);
    }

    pub fn with_work_space<T>(
        &self,
        state: &AppState,
        reserve: u64,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let _workspace = self.workspace.write().unwrap();
        let usage = self.usage.lock().unwrap();
        let bytes = reclaim_work_space(&state.cache, reserve)?;
        ensure!(
            database_bytes(&state.db)? + usage.bytes + bytes + reserve <= TOTAL_LIMIT,
            StorageFull("カタログ全体の保存容量が不足しています。")
        );
        let mut before_paths = Vec::new();
        files(&state.cache, &mut before_paths)?;
        let before = before_paths
            .into_iter()
            .map(|path| allocated(&path).map(|size| (path, size)))
            .collect::<Result<HashMap<_, _>>>()?;
        let result = operation();
        let after_bytes = working_bytes(&state.cache)?;
        let oversized = after_bytes > WORK_LIMIT
            || database_bytes(&state.db)? + usage.bytes + after_bytes > TOTAL_LIMIT;
        if result.is_err() || oversized {
            let mut after = Vec::new();
            files(&state.cache, &mut after)?;
            for path in after {
                if before.get(&path).copied() != Some(allocated(&path)?) {
                    let _ = std::fs::remove_file(path);
                }
            }
            if result.is_ok() && oversized {
                anyhow::bail!("現像結果が作業領域の上限を超えました。");
            }
        }
        result
    }

    pub fn invalidate(&self, catalog: &Catalog, id: i64, bound: u32) -> Result<()> {
        let _workspace = self.workspace.write().unwrap();
        ensure!([384, 1600].contains(&bound), "Invalid library preview size");
        let mut usage = self.usage.lock().unwrap();
        let Some(record) = catalog.library_preview(id)? else {
            return Ok(());
        };
        let Some(data) = record.data else {
            return Ok(());
        };
        let mut images: Images = serde_json::from_str(&data)?;
        ensure!(
            ["avif", "jpg"].contains(&images.extension.as_str()),
            "Invalid preview format"
        );
        if images.extension == "avif" {
            // A decodable AVIF can still be rejected by the browser; remember its JPEG fallback.
            images.jpeg_fallback = true;
            catalog.save_library_preview(
                id,
                &record.stamp,
                Some(&serde_json::to_string(&images)?),
                None,
            )?;
        }
        Self::remove(
            &mut usage,
            &self.path(
                &record.stamp,
                &images.extension,
                bound,
                &images.render_version,
            ),
        )
    }

    fn retains_thumbnail_only(&self, catalog: &Catalog, id: i64) -> Result<bool> {
        if self.lookup_stored(catalog, id, 384)?.is_none() {
            return Ok(false);
        }
        let Some(record) = catalog.library_preview(id)? else {
            return Ok(false);
        };
        let Some(data) = record.data else {
            return Ok(false);
        };
        let Ok(images) = serde_json::from_str::<Images>(&data) else {
            return Ok(false);
        };
        // A published thumbnail with no completed large image is an interrupted import.
        if images.large_bytes == 0 || !["avif", "jpg"].contains(&images.extension.as_str())
            || record.stamp != catalog.preview_stamp(id)?
        {
            return Ok(false);
        }
        Ok(!self
            .path(
                &record.stamp,
                &images.extension,
                1600,
                &images.render_version,
            )
            .exists())
    }

    pub fn backup(&self, db: &Path, destination: &Path) -> Result<()> {
        let _workspace = self.workspace.write().unwrap();
        let _writer = self.usage.lock().unwrap();
        ensure!(
            !directory(destination).exists(),
            "コピー先のプレビュー保存先はすでに存在します。"
        );
        Catalog::open(db)?.backup(destination)?;
        if let Err(error) = copy(db, destination) {
            let _ = std::fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    pub fn generate(
        &self,
        state: &AppState,
        id: i64,
        cancelled: &(dyn Fn() -> bool + Sync),
        evict: bool,
    ) -> Result<bool> {
        self.generate_inner(state, &Catalog::open(&state.db)?, id, cancelled, evict, true, 1)
    }

    pub fn generate_explicit(&self, state: &AppState, id: i64) -> Result<bool> {
        self.generate_inner(state, &Catalog::open(&state.db)?, id, &|| false, true, false, 2)
    }

    pub fn generate_visible(&self, state: &AppState, id: i64, large: bool) -> Result<bool> {
        self.generate_inner(
            state,
            &Catalog::open(&state.db)?,
            id,
            &|| false,
            true,
            true,
            if large { 2 } else { 1 },
        )
    }

    pub fn early_preview(
        &self,
        catalog: &Catalog,
        id: i64,
        large: bool,
    ) -> Result<Option<serde_json::Value>> {
        if large {
            let stamp = catalog.preview_stamp(id)?;
            return Ok(self
                .early
                .lock()
                .unwrap()
                .get(&id)
                .filter(|(version, _)| *version == stamp)
                .map(|(_, value)| value.clone()));
        }
        self.lookup_stored(catalog, id, 384)?
            .map(serde_json::to_value)
            .transpose()
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_inner(
        &self,
        state: &AppState,
        catalog: &Catalog,
        id: i64,
        cancelled: &(dyn Fn() -> bool + Sync),
        evict: bool,
        defer_edits: bool,
        priority: u8,
    ) -> Result<bool> {
        // Explicit library edits need their result now; Develop autosaves remain deferred.
        let deferred = if defer_edits {
            catalog.preview_update(id)?
        } else {
            None
        };
        if deferred
            .is_some_and(|(_, ready)| !ready || self.updates.active.load(Ordering::SeqCst) == id)
        {
            return Ok(false);
        }
        let generation = self.updates.generation.load(Ordering::SeqCst);
        let cancelled = &|| {
            cancelled()
                || (deferred.is_some()
                    && self.updates.generation.load(Ordering::SeqCst) != generation)
        };
        let mut outdated = self.outdated_development(catalog, id)?
            || catalog.library_preview(id)?.is_some_and(|record| {
                catalog
                    .preview_stamp(id)
                    .is_ok_and(|stamp| stamp != record.stamp)
            });
        if outdated && catalog.source_path(id).is_err() {
            return Ok(false);
        }
        if !outdated
            && self.lookup_stored(catalog, id, 1600)?.is_some()
            && self.lookup_stored(catalog, id, 384)?.is_some()
        {
            return Ok(false);
        }
        if self.lookup_stored(catalog, id, 1600)?.is_none() {
            catalog.refresh_preview_source(id)?;
        }
        let _job = self.queue.enter(id, priority, cancelled)?;
        let _workspace = self.workspace.read().unwrap();
        let mut usage = self.usage.lock().unwrap();
        let stamp = catalog.preview_stamp(id)?;
        let old = catalog.library_preview(id)?;
        // Recheck after another import, repair, or edit-save job released the writer.
        outdated = self.outdated_development(catalog, id)?
            || old.as_ref().is_some_and(|record| record.stamp != stamp);
        if !outdated
            && self.lookup_stored(catalog, id, 1600)?.is_some()
            && self.lookup_stored(catalog, id, 384)?.is_some()
        {
            return Ok(false);
        }
        let stored = if outdated {
            None
        } else {
            self.lookup_stored(catalog, id, 1600)?
        };
        let raw = Self::is_raw(catalog, id)?;
        let previous = old
            .as_ref()
            .and_then(|record| record.data.as_deref())
            .and_then(|data| serde_json::from_str::<Images>(data).ok());
        let mut camera_fallback = stored.is_some()
            && previous.as_ref().is_some_and(|images| images.camera_fallback);
        let mut raw_developed = raw && !camera_fallback;
        let render_version = if stored.is_some() {
            serde_json::from_str::<Images>(old.as_ref().unwrap().data.as_ref().unwrap())?
                .render_version
        } else if raw {
            format!("{}-raw-library-v1", advanced_explorer_develop::RENDER_VERSION)
        } else {
            advanced_explorer_develop::RENDER_VERSION.into()
        };
        let jpeg_fallback = previous.as_ref().is_some_and(|images| images.jpeg_fallback);
        let avif = !jpeg_fallback
            && stored
                .as_ref()
                .map_or(self.avif.load(Ordering::SeqCst), |preview| {
                    Path::new(&preview.path)
                        .extension()
                        .is_some_and(|extension| extension == "avif")
                });
        let extension = if avif { "avif" } else { "jpg" };
        let reuse_large = stored.as_ref().is_some_and(|preview| {
            Path::new(&preview.path)
                .extension()
                .is_some_and(|value| value == extension)
        });
        ensure!(!cancelled(), "プレビュー生成を中止しました。");
        let required = usage.reserved + IMPORT_RESERVE;
        // Active decoders own their temporary files until their job finishes.
        if usage.reserved == 0 {
            reclaim_work_space(&state.cache, required)?;
        } else {
            ensure!(
                working_bytes(&state.cache)? + required <= WORK_LIMIT,
                StorageFull("現像用の作業領域が不足しています。")
            );
        }
        self.reserve(state, &mut usage, required, evict)?;
        usage.reserved += IMPORT_RESERVE;
        drop(usage);
        let _reservation = Reservation { store: self, id };
        let result = (|| -> Result<()> {
            let temporary = tempfile::Builder::new()
                .prefix("import-")
                .tempdir_in(&state.cache)?;
            let request = if stored.is_none() {
                Some(crate::prepare_render(
                    state.db.clone(),
                    state.cache.clone(),
                    state.renderer.clone(),
                    id,
                    1600,
                )?)
            } else {
                None
            };
            let (json, _) = catalog.develop_recipe(id)?;
            let render_source = || -> Result<_> {
                let intermediate = temporary.path().join("source.tif");
                let info = state.renderer.render(
                    &request.as_ref().unwrap().source,
                    &intermediate,
                    1600,
                    cancelled,
                )?;
                Ok((
                    advanced_explorer_media::preview::read_srgb(&intermediate)?,
                    info,
                ))
            };
            let (pixels, info) = if let Some(stored) = &stored {
                let intermediate = temporary.path().join("stored.tif");
                state
                    .renderer
                    .render(Path::new(&stored.path), &intermediate, 1600, cancelled)?;
                (
                    advanced_explorer_media::preview::read_srgb(&intermediate)?,
                    stored.clone(),
                )
            } else if json.is_some() || raw {
                let settings = json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default();
                match crate::develop::library_pixels(state, id, &settings, cancelled) {
                    Ok(result) => result,
                    Err(error)
                        if raw
                            && json.is_none()
                            && error.to_string().contains("rtengine cannot decode the photo (-2)") =>
                    {
                        // LibRaw -2 is unsupported input; never hide edits or transient failures.
                        camera_fallback = true;
                        raw_developed = false;
                        render_source()?
                    }
                    Err(error) => return Err(error),
                }
            } else {
                render_source()?
            };
            let pixels = image::DynamicImage::ImageRgb8(pixels);
            let small = pixels.thumbnail(384, 384).into_rgb8();
            let pixels = pixels.into_rgb8();
            if let Some(request) = &request {
                let after = crate::prepare_render(
                    state.db.clone(),
                    state.cache.clone(),
                    state.renderer.clone(),
                    id,
                    1600,
                )?;
                ensure!(
                    request.image == after.image,
                    "生成中に原本が変更されました。再走査してください。"
                );
            }
            if self.queue.wants_large(id) && !cancelled() {
                use base64::Engine;
                let mut png = Vec::new();
                advanced_explorer_media::encode_preview(
                    &mut png,
                    &image::DynamicImage::ImageRgb8(pixels.clone()).to_rgba8(),
                )?;
                let mut value = serde_json::to_value(&info)?;
                value["inline_image"] = format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
                .into();
                self.early
                    .lock()
                    .unwrap()
                    .insert(id, (stamp.clone(), value));
                self.ready.notify_waiters();
            }
            let mut images = Images {
                version: VERSION.into(),
                render_version: render_version.clone(),
                raw_developed,
                camera_fallback,
                jpeg_fallback,
                extension: extension.into(),
                width: info.width,
                height: info.height,
                large: [pixels.width() as usize, pixels.height() as usize],
                thumbnail: [small.width() as usize, small.height() as usize],
                large_bytes: if reuse_large {
                    std::fs::metadata(&stored.as_ref().unwrap().path)?.len()
                } else {
                    0
                },
                thumbnail_bytes: 0,
            };
            for (bound, pixels) in [(384, &small), (1600, &pixels)] {
                if bound == 1600 && reuse_large {
                    continue;
                }
                ensure!(!cancelled(), "プレビュー生成を中止しました。");
                let bytes = advanced_explorer_media::preview::encode(pixels, bound == 384, avif)?;
                ensure!(
                    bytes.len() < 16 * 1024 * 1024,
                    "プレビューが保存予約量を超えています。"
                );
                let mut usage = self.usage.lock().unwrap();
                let target = self.path(&stamp, extension, bound, &render_version);
                std::fs::create_dir_all(target.parent().unwrap())?;
                ensure!(
                    !std::fs::symlink_metadata(target.parent().unwrap())?
                        .file_type()
                        .is_symlink(),
                    "プレビュー保存先にシンボリックリンクは使えません。"
                );
                let file = temporary.path().join(format!("{bound}.{extension}"));
                std::fs::write(&file, bytes)?;
                std::fs::rename(&file, &target)?;
                if let Some(size) = usage.files.remove(&target) {
                    usage.bytes -= size;
                }
                let size = allocated(&target)?;
                usage.bytes += size;
                usage.files.insert(target.clone(), size);
                if bound == 384 {
                    images.thumbnail_bytes = std::fs::metadata(&target)?.len();
                } else {
                    images.large_bytes = std::fs::metadata(&target)?.len();
                }
                ensure!(
                    !cancelled()
                        && catalog.save_library_preview(
                            id,
                            &stamp,
                            Some(&serde_json::to_string(&images)?),
                            None
                        )?,
                    "生成中に写真の編集設定が変更されました。"
                );
                self.ready.notify_waiters();
            }
            ensure!(!cancelled(), "プレビュー生成を中止しました。");
            if let Some(request) = request {
                let after = crate::prepare_render(
                    state.db.clone(),
                    state.cache.clone(),
                    state.renderer.clone(),
                    id,
                    1600,
                )?;
                ensure!(
                    request.image == after.image,
                    "生成中に原本が変更されました。再走査してください。"
                );
            }
            ensure!(
                !cancelled()
                    && catalog.save_library_preview(
                        id,
                        &stamp,
                        Some(&serde_json::to_string(&images)?),
                        None
                    )?,
                "生成中に写真の編集設定が変更されました。"
            );
            let mut usage = self.usage.lock().unwrap();
            if let Some(old) = old.as_ref()
                && let Some(data) = &old.data
                && let Ok(images) = serde_json::from_str::<Images>(data)
                && ["avif", "jpg"].contains(&images.extension.as_str())
                && (old.stamp != stamp
                    || images.extension != extension
                    || images.render_version != render_version)
            {
                for bound in [384, 1600] {
                    Self::remove(
                        &mut usage,
                        &self.path(&old.stamp, &images.extension, bound, &images.render_version),
                    )?;
                }
            }
            Ok(())
        })();
        if let Err(error) = &result {
            let mut usage = self.usage.lock().unwrap();
            let retained = if stored.is_some()
                || (outdated && old.as_ref().is_some_and(|record| record.stamp == stamp))
            {
                old.as_ref().and_then(|record| record.data.as_deref())
            } else {
                None
            };
            if old.as_ref().is_none_or(|record| record.stamp == stamp) {
                let _ =
                    catalog.save_library_preview(id, &stamp, retained, Some(&error.to_string()));
            }
            for bound in [384, 1600] {
                if bound == 1600 && reuse_large {
                    continue;
                }
                let _ = Self::remove(
                    &mut usage,
                    &self.path(&stamp, extension, bound, &render_version),
                );
            }
        }
        result.map(|_| true)
    }

    fn outdated_development(&self, catalog: &Catalog, id: i64) -> Result<bool> {
        let raw = Self::is_raw(catalog, id)?;
        if !raw && catalog.develop_recipe(id)?.0.is_none() {
            return Ok(false);
        }
        let expected = if raw {
            format!("{}-raw-library-v1", advanced_explorer_develop::RENDER_VERSION)
        } else {
            advanced_explorer_develop::RENDER_VERSION.into()
        };
        Ok(catalog
            .library_preview(id)?
            .and_then(|record| record.data)
            .and_then(|data| serde_json::from_str::<Images>(&data).ok())
            .is_some_and(|images| {
                images.render_version != expected
                    || (raw && !images.raw_developed && !images.camera_fallback)
            }))
    }

    fn is_raw(catalog: &Catalog, id: i64) -> Result<bool> {
        Ok(advanced_explorer::RAW_EXTENSIONS.contains(&catalog.detail(id)?.item.extension.as_str()))
    }
}

pub fn prepare(
    state: &AppState,
    root: Option<i64>,
    progress: impl Fn(&Progress),
) -> Result<Progress> {
    prepare_batch(state, root, false, progress)
}

pub fn prepare_import(
    state: &AppState,
    root: Option<i64>,
    progress: impl Fn(&Progress),
) -> Result<Progress> {
    prepare_batch(state, root, true, progress)
}

fn prepare_batch(
    state: &AppState,
    root: Option<i64>,
    skip_imported: bool,
    progress: impl Fn(&Progress),
) -> Result<Progress> {
    let catalog = Catalog::open(&state.db)?;
    let total = catalog.count(&crate::Query {
        root_id: root,
        ..Default::default()
    })? as usize;
    let mut report = Progress {
        total,
        ..Default::default()
    };
    let mut last_progress = std::time::Instant::now();
    let mut publish = |report: &Progress, force: bool| {
        if force || last_progress.elapsed() >= std::time::Duration::from_millis(100) {
            progress(report);
            last_progress = std::time::Instant::now();
        }
    };
    let mut after = 0;
    while !report.cancelled && !report.paused {
        let ids = catalog.preview_items(root, after)?;
        if ids.is_empty() {
            break;
        }
        after = *ids.last().unwrap();
        let pending = Mutex::new(ids.into_iter());
        let stopped = AtomicBool::new(false);
        std::thread::scope(|scope| -> Result<()> {
            let (send, receive) = std::sync::mpsc::channel();
            for _ in 0..2 {
                let send = send.clone();
                let pending = &pending;
                let stopped = &stopped;
                scope.spawn(move || {
                    let cancelled =
                        || state.cancel.load(Ordering::SeqCst) || stopped.load(Ordering::SeqCst);
                    let catalog = match Catalog::open(&state.db) {
                        Ok(catalog) => catalog,
                        Err(error) => {
                            let _ = send.send((0, Err(error)));
                            return;
                        }
                    };
                    while !cancelled() {
                        let Some(id) = pending.lock().unwrap().next() else {
                            break;
                        };
                        let result = (|| -> Result<Option<bool>> {
                            if skip_imported
                                && let Some(record) = catalog.library_preview(id)?
                            {
                                let images = record
                                    .data
                                    .as_deref()
                                    .and_then(|data| serde_json::from_str::<Images>(data).ok());
                                // Re-imports keep completed previews; interrupted imports still resume.
                                if record.error.is_none()
                                    && record.stamp == catalog.preview_stamp(id)?
                                    && images.is_some_and(|images| {
                                        images.version == VERSION
                                            && ["avif", "jpg"].contains(&images.extension.as_str())
                                            && images.large_bytes > 0
                                            && images.thumbnail_bytes > 0
                                            && [
                                                (384, images.thumbnail_bytes),
                                                (1600, images.large_bytes),
                                            ]
                                            .into_iter()
                                            .all(|(bound, bytes)| {
                                                let path = state.library.path(
                                                    &record.stamp,
                                                    &images.extension,
                                                    bound,
                                                    &images.render_version,
                                                );
                                                std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                                                    metadata.is_file() && metadata.len() == bytes
                                                })
                                            })
                                    })
                                {
                                    return Ok(Some(false));
                                }
                            }
                            if state.library.retains_thumbnail_only(&catalog, id)? {
                                return Ok(None);
                            }
                            state
                                .library
                                .generate_inner(state, &catalog, id, &cancelled, false, true, 0)
                                .map(Some)
                        })();
                        if send.send((id, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(send);
            for (id, result) in receive {
                if id == 0 {
                    stopped.store(true, Ordering::SeqCst);
                    return result.map(|_| ());
                }
                match result {
                    Ok(None) => report.retained += 1,
                    Ok(Some(generated)) => {
                        report.ready += 1;
                        if generated {
                            report.last_ready_id = Some(id);
                        }
                        report.generated += usize::from(generated);
                    }
                    Err(error) if !stopped.load(Ordering::SeqCst) => {
                        report.cancelled = state.cancel.load(Ordering::SeqCst);
                        report.paused = error.is::<StorageFull>()
                            || error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                                error.kind() == std::io::ErrorKind::StorageFull
                            });
                        if !report.cancelled && !report.paused {
                            report.failed += 1;
                        }
                        report.error = Some(error.to_string());
                        if report.cancelled || report.paused {
                            stopped.store(true, Ordering::SeqCst);
                        }
                    }
                    Err(_) => {}
                }
                publish(&report, report.cancelled || report.paused);
            }
            Ok(())
        })?;
        report.cancelled |= state.cancel.load(Ordering::SeqCst);
    }
    publish(&report, true);
    Ok(report)
}

pub fn copy(db: &Path, destination: &Path) -> Result<()> {
    let source = directory(db);
    if !source.exists() {
        return Ok(());
    }
    let target = directory(destination);
    ensure!(
        !target.exists(),
        "コピー先のプレビュー保存先はすでに存在します。"
    );
    let staging = tempfile::Builder::new()
        .prefix(".openroom-preview-copy-")
        .tempdir_in(destination.parent().unwrap())?;
    let mut paths = Vec::new();
    for folder in ["large", "thumbnails"] {
        files(&source.join(folder), &mut paths)?;
    }
    for path in paths {
        let out = staging.path().join(path.strip_prefix(&source)?);
        std::fs::create_dir_all(out.parent().unwrap())?;
        std::fs::copy(path, out)?;
    }
    std::fs::rename(staging.path(), target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Renderer;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    };

    #[test]
    #[ignore = "release microbenchmark; creates 2000 disposable cache files"]
    fn workspace_check_benchmark() {
        let (_dir, state, _) = fixture();
        for i in 0..2000 {
            std::fs::write(state.cache.join(format!("develop-{i:04}.png")), [0u8; 1024]).unwrap();
        }
        let before = working_bytes(&state.cache).unwrap();
        let mut times = Vec::new();
        for _ in 0..31 {
            let start = std::time::Instant::now();
            state.library.with_work_space(&state, IMPORT_RESERVE, || Ok(())).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        assert_eq!(working_bytes(&state.cache).unwrap(), before);
        times.sort_by(f64::total_cmp);
        println!("workspace_check {}", serde_json::json!({"files":2000,"samples":31,"median_ms":times[15],"p95_ms":times[29]}));
    }

    #[test]
    fn under_budget_reclamation_preserves_files_and_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("develop-old.png");
        std::fs::write(&output, b"cached image").unwrap();
        let expected = allocated(&output).unwrap();
        assert_eq!(reclaim_work_space(dir.path(), IMPORT_RESERVE).unwrap(), expected);
        assert_eq!(std::fs::read(&output).unwrap(), b"cached image");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&output, dir.path().join("link.png")).unwrap();
            assert!(reclaim_work_space(dir.path(), 0).is_err());
            assert_eq!(std::fs::read(&output).unwrap(), b"cached image");
        }
    }

    fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("originals");
        std::fs::create_dir(&root).unwrap();
        for (name, width, height) in [("landscape.png", 1800, 1200), ("portrait.jpg", 1200, 1800)] {
            image::RgbImage::from_fn(width, height, |x, y| {
                image::Rgb([(x % 256) as u8, (y % 256) as u8, 100])
            })
            .save(root.join(name))
            .unwrap();
        }
        let db = dir.path().join("catalog.openroom");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let library = Arc::new(Store::new(&db).unwrap());
        let cache = library.directory.join("working");
        std::fs::create_dir(&cache).unwrap();
        let state = AppState {
            renderer: Renderer::new(&cache).unwrap(),
            db,
            cache,
            library,
            scanning: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            thumbs: Arc::new(tokio::sync::Semaphore::new(3)),
            previews: Arc::new(tokio::sync::Semaphore::new(1)),
            preview_version: Arc::new(AtomicUsize::new(0)),
            metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
            metadata_version: Arc::new(AtomicUsize::new(0)),
        };
        (dir, state, root)
    }

    #[test]
    #[ignore = "requires an unsupported AE_PREVIEW_PHOTO and packaged AE_TEST_EXECUTABLE"]
    fn unsupported_raw_keeps_camera_preview_without_hiding_edits() {
        let (_dir, state, root) = fixture();
        let original = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap());
        let source = root.join(original.file_name().unwrap());
        std::fs::copy(&original, &source).unwrap();
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog
            .preview_items(None, 0)
            .unwrap()
            .into_iter()
            .find(|id| Store::is_raw(&catalog, *id).unwrap())
            .unwrap();
        assert!(state.library.generate_explicit(&state, id).unwrap());
        let record = catalog.library_preview(id).unwrap().unwrap();
        let images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
        assert!(!images.raw_developed);
        assert!(images.camera_fallback);
        assert!(record.error.is_none());
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
        assert!(state.library.lookup(&catalog, id, 384).unwrap().is_some());
        assert!(!state.library.generate_explicit(&state, id).unwrap());
        assert_eq!(catalog.develop_recipe(id).unwrap(), (None, 0));
        assert!(catalog.develop_history(id, None).unwrap().is_empty());

        let settings = advanced_explorer_develop::Settings {
            exposure: 1.,
            ..Default::default()
        };
        catalog
            .save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 0)
            .unwrap();
        let error = state.library.generate_explicit(&state, id).unwrap_err();
        assert!(error.to_string().contains("rtengine cannot decode the photo (-2)"));
        assert_eq!(catalog.library_preview(id).unwrap().unwrap().stamp, record.stamp);
        assert_ne!(catalog.preview_stamp(id).unwrap(), record.stamp);
    }

    #[test]
    #[ignore = "requires AE_PREVIEW_PHOTO and the packaged AE_TEST_EXECUTABLE; uses a disposable catalog"]
    fn raw_library_matches_develop_and_migrates_camera_previews() {
        let (dir, state, root) = fixture();
        let original = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap());
        let source = root.join(original.file_name().unwrap());
        std::fs::copy(&original, &source).unwrap();
        let original_bytes = std::fs::read(&source).unwrap();
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog
            .preview_items(None, 0)
            .unwrap()
            .into_iter()
            .find(|id| Store::is_raw(&catalog, *id).unwrap())
            .unwrap();
        let settings = advanced_explorer_develop::Settings::default();
        let (reference, _) =
            crate::develop::library_pixels(&state, id, &settings, &|| false).unwrap();
        let started = std::time::Instant::now();
        assert!(state.library.generate_explicit(&state, id).unwrap());
        let generated_ms = started.elapsed().as_secs_f64() * 1000.;
        let preview = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        let decoded = dir.path().join("decoded.tif");
        state
            .renderer
            .render(Path::new(&preview.path), &decoded, 1600, &|| false)
            .unwrap();
        let actual = advanced_explorer_media::preview::read_srgb(&decoded).unwrap();
        assert_eq!(actual.dimensions(), reference.dimensions());
        let mean_error = actual
            .as_raw()
            .iter()
            .zip(reference.as_raw())
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / actual.as_raw().len() as f64;
        // Compare decoded codec output: ICC profiles include their creation timestamp.
        let expected_bytes = advanced_explorer_media::preview::encode(
            &reference,
            false,
            preview.path.ends_with(".avif"),
        )
        .unwrap();
        let expected = dir.path().join(if preview.path.ends_with(".avif") {
            "expected.avif"
        } else {
            "expected.jpg"
        });
        std::fs::write(&expected, expected_bytes).unwrap();
        let expected_decoded = dir.path().join("expected.tif");
        state
            .renderer
            .render(&expected, &expected_decoded, 1600, &|| false)
            .unwrap();
        let expected_pixels =
            advanced_explorer_media::preview::read_srgb(&expected_decoded).unwrap();
        assert!(
            actual == expected_pixels,
            "Library pixels differ from encoded development output"
        );
        assert_eq!(catalog.develop_recipe(id).unwrap(), (None, 0));
        assert!(catalog.develop_history(id, None).unwrap().is_empty());
        let started = std::time::Instant::now();
        assert!(!state.library.generate_explicit(&state, id).unwrap());
        let reused_ms = started.elapsed().as_secs_f64() * 1000.;

        for raw_developed in [false, true] {
            let record = catalog.library_preview(id).unwrap().unwrap();
            let mut images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
            let current_version = images.render_version.clone();
            images.raw_developed = raw_developed;
            images.render_version = advanced_explorer_develop::RENDER_VERSION.into();
            for bound in [384, 1600] {
                let legacy = state.library.path(
                    &record.stamp,
                    &images.extension,
                    bound,
                    &images.render_version,
                );
                std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
                std::fs::rename(
                    state
                        .library
                        .path(&record.stamp, &images.extension, bound, &current_version),
                    legacy,
                )
                .unwrap();
            }
            catalog
                .save_library_preview(
                    id,
                    &record.stamp,
                    Some(&serde_json::to_string(&images).unwrap()),
                    None,
                )
                .unwrap();
            assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_none());
            let offline = dir.path().join("offline");
            std::fs::rename(&root, &offline).unwrap();
            assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
            assert!(!state.library.generate_explicit(&state, id).unwrap());
            std::fs::rename(&offline, &root).unwrap();
            assert!(state.library.generate_explicit(&state, id).unwrap());
            assert!(!state.library.outdated_development(&catalog, id).unwrap());
        }
        assert_eq!(catalog.develop_recipe(id).unwrap(), (None, 0));
        assert_eq!(std::fs::read(source).unwrap(), original_bytes);
        println!(
            "raw_color_parity {}",
            serde_json::json!({"mean_error_255":mean_error,"generated_ms":generated_ms,"reused_ms":reused_ms})
        );
    }

    #[test]
    fn early_image_is_lossless_and_partial_cache_is_removed_on_cancel() {
        use base64::Engine;
        let (_dir, state, _) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let source = catalog.source_path(id).unwrap();
        let intermediate = state.cache.join("reference.tif");
        state
            .renderer
            .render(&source, &intermediate, 1600, &|| false)
            .unwrap();
        let reference = advanced_explorer_media::preview::read_srgb(&intermediate).unwrap();
        let early = Mutex::new(None);
        // Cancel at the first saved thumbnail, independent of encoder speed and thread scheduling.
        let result = state.library.generate_inner(
            &state,
            &catalog,
            id,
            &|| {
                let catalog = Catalog::open(&state.db).unwrap();
                if !state.cancel.load(Ordering::SeqCst)
                    && state
                        .library
                        .lookup_stored(&catalog, id, 384)
                        .unwrap()
                        .is_some()
                {
                    *early.lock().unwrap() =
                        state.library.early_preview(&catalog, id, true).unwrap();
                    assert!(
                        state
                            .library
                            .lookup_stored(&catalog, id, 1600)
                            .unwrap()
                            .is_none()
                    );
                    state.cancel.store(true, Ordering::SeqCst);
                }
                state.cancel.load(Ordering::SeqCst)
            },
            false,
            true,
            2,
        );
        assert!(result.is_err());
        let early = early
            .into_inner()
            .unwrap()
            .expect("lossless early preview before cancellation");
        let png = base64::engine::general_purpose::STANDARD
            .decode(
                early["inline_image"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgb8(),
            reference
        );
        assert!(
            state
                .library
                .early_preview(&catalog, id, true)
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .library
                .lookup_stored(&catalog, id, 384)
                .unwrap()
                .is_none()
        );
        assert_eq!(state.library.usage.lock().unwrap().reserved, 0);
        state.cancel.store(false, Ordering::SeqCst);
        assert!(state.library.generate_explicit(&state, id).unwrap());
        let preview = state
            .library
            .lookup_stored(&catalog, id, 1600)
            .unwrap()
            .unwrap();
        std::fs::remove_file(preview.path).unwrap();
        let record = catalog.library_preview(id).unwrap().unwrap();
        let mut images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
        images.large_bytes = 0;
        catalog
            .save_library_preview(
                id,
                &record.stamp,
                Some(&serde_json::to_string(&images).unwrap()),
                None,
            )
            .unwrap();
        assert!(!state.library.retains_thumbnail_only(&catalog, id).unwrap());
        assert_eq!(
            prepare(&state, None, |_| {}).unwrap().generated,
            2,
            "resume imports interrupted after the thumbnail commit"
        );
    }

    #[test]
    fn leaving_develop_refreshes_a_disabled_mask_on_the_same_photo() {
        let (dir, state, root) = fixture();
        let mut catalog = Catalog::open(&state.db).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        state.library.set_avif_supported(false);
        state.library.updates.active.store(id, Ordering::SeqCst);
        let mut settings = advanced_explorer_develop::Settings {
            exposure: 0.1,
            ..Default::default()
        };
        let preview_pixels = |catalog: &Catalog| {
            let preview = state.library.lookup(catalog, id, 1600).unwrap().unwrap();
            image::open(preview.path).unwrap().into_rgb8()
        };
        catalog.save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 0).unwrap();
        assert!(state.library.generate_explicit(&state, id).unwrap());
        let unmasked = preview_pixels(&catalog);
        settings.masks.push(serde_json::from_value(serde_json::json!({
            "name":"Mask", "enabled":true, "invert":false, "amount":1.,
            "adjustments":{"exposure":-2.},
            "shapes":[{"kind":"linear", "operation":"add", "start":[0.,0.],
                "end":[0.,0.5], "radius":0.1, "feather":0.5, "opacity":1.,
                "points":[], "color":[0.,0.,0.], "range":[0.,1.]}]
        })).unwrap());
        catalog.save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 1).unwrap();
        assert!(state.library.generate_explicit(&state, id).unwrap());
        let masked = preview_pixels(&catalog);
        assert!(unmasked != masked);
        settings.masks[0].enabled = false;
        catalog.save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 2).unwrap();
        assert!(!state.library.generate(&state, id, &|| false, false).unwrap());
        assert!(preview_pixels(&catalog) == masked, "autosaves retain the old library preview while editing");
        assert!(state.library.generate_explicit(&state, id).unwrap());
        assert!(preview_pixels(&catalog) == unmasked, "leaving Develop must show the disabled mask result");
        assert_eq!(catalog.library_preview(id).unwrap().unwrap().stamp, catalog.preview_stamp(id).unwrap());
        std::fs::rename(root, dir.path().join("offline")).unwrap();
        assert!(!state.library.generate_explicit(&state, id).unwrap(), "unchanged previews must work offline without another render");
    }

    #[test]
    fn old_development_preview_is_retained_offline_and_refreshed_when_source_returns() {
        let (dir, state, root) = fixture();
        let mut catalog = Catalog::open(&state.db).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let source = catalog.source_path(id).unwrap();
        let settings = advanced_explorer_develop::Settings {
            exposure: 1.,
            ..Default::default()
        };
        catalog
            .save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 0)
            .unwrap();
        catalog.release_preview_updates(id + 1).unwrap();
        let request = crate::prepare_render(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            4096,
        )
        .unwrap();
        let base = advanced_explorer_develop::prepare(&source, 1600).unwrap();
        let base_path = request.image.with_extension(format!(
            "{}-1600.tif",
            advanced_explorer_develop::RENDER_VERSION
        ));
        advanced_explorer_develop::write_base(&base_path, &base).unwrap();
        std::fs::write(
            base_path.with_extension("json"),
            serde_json::to_vec(&base.info).unwrap(),
        )
        .unwrap();
        state
            .library
            .generate(&state, id, &|| false, false)
            .unwrap();
        let record = catalog.library_preview(id).unwrap().unwrap();
        let mut images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
        let old_version = images.render_version.clone();
        images.render_version = String::new();
        for bound in [384, 1600] {
            let legacy = state.library.path(
                &record.stamp,
                &images.extension,
                bound,
                &images.render_version,
            );
            std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
            std::fs::rename(
                state
                    .library
                    .path(&record.stamp, &images.extension, bound, &old_version),
                legacy,
            )
            .unwrap();
        }
        catalog
            .save_library_preview(
                id,
                &record.stamp,
                Some(&serde_json::to_string(&images).unwrap()),
                None,
            )
            .unwrap();
        let preview = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        let bytes = std::fs::read(&preview.path).unwrap();
        let offline = dir.path().join("offline");
        std::fs::rename(&root, &offline).unwrap();
        assert!(
            !state
                .library
                .generate(&state, id, &|| false, false)
                .unwrap()
        );
        assert_eq!(bytes, std::fs::read(&preview.path).unwrap());
        std::fs::rename(&offline, &root).unwrap();
        let calls = AtomicUsize::new(0);
        assert!(
            state
                .library
                .generate(
                    &state,
                    id,
                    &|| calls.fetch_add(1, Ordering::SeqCst) > 0,
                    false
                )
                .is_err()
        );
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
        assert_eq!(bytes, std::fs::read(&preview.path).unwrap());
        assert!(
            state
                .library
                .generate(&state, id, &|| false, false)
                .unwrap()
        );
        assert!(!state.library.outdated_development(&catalog, id).unwrap());
        assert_eq!(catalog.develop_recipe(id).unwrap().1, 1);
    }

    #[test]
    fn rapid_development_reclaims_old_outputs_and_protects_the_two_latest_images() {
        let (_dir, state, _) = fixture();
        for index in 0..80 {
            state
                .library
                .with_work_space(&state, 20_000_000, || {
                    let path = state.cache.join(format!("develop-{index:03}.png"));
                    std::fs::File::create(path)?.set_len(20_000_000)?;
                    Ok(())
                })
                .unwrap();
            assert!(working_bytes(&state.cache).unwrap() <= WORK_LIMIT);
            assert!(state.cache.join(format!("develop-{index:03}.png")).exists());
            if index > 0 {
                assert!(
                    state
                        .cache
                        .join(format!("develop-{:03}.png", index - 1))
                        .exists()
                );
            }
        }
        assert!(!state.cache.join("develop-000.png").exists());
        assert!(!state.cache.join("develop-001.png").exists());
    }

    #[test]
    fn oversized_replacements_and_cancelled_outputs_are_removed() {
        let (_dir, state, _) = fixture();
        let existing = state.cache.join("damaged.develop-v6-0.tif");
        std::fs::write(&existing, b"damaged cache").unwrap();
        let created = state.cache.join("develop-new.png");
        let result = state.library.with_work_space(&state, 1, || {
            std::fs::File::create(&existing)?.set_len(900_000_000)?;
            std::fs::File::create(&created)?.set_len(200_000_000)?;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!existing.exists());
        assert!(!created.exists());
        assert!(
            state
                .library
                .with_work_space::<()>(&state, 1, || {
                    std::fs::write(&created, b"cancelled")?;
                    anyhow::bail!("cancelled")
                })
                .is_err()
        );
        assert!(!created.exists());
        assert!(
            state
                .library
                .with_work_space(&state, u64::MAX, || Ok(()))
                .is_err()
        );
    }

    #[test]
    fn development_reclaims_recent_decoded_bases_but_keeps_displayed_images() {
        let (_dir, state, _) = fixture();
        for version in ["develop-v5", advanced_explorer_develop::RENDER_VERSION] {
            let base = state.cache.join(format!("old-photo.{version}-0.tif"));
            std::fs::File::create(&base)
                .unwrap()
                .set_len(800_000_000)
                .unwrap();
            let displayed = state.cache.join("develop-current.png");
            std::fs::write(&displayed, b"current image").unwrap();
            let protected = state.cache.join("renderer-test");
            std::fs::write(&protected, b"renderer").unwrap();
            state
                .library
                .with_work_space(&state, 400_000_000, || {
                    assert!(!base.exists());
                    assert_eq!(std::fs::read(&displayed)?, b"current image");
                    assert_eq!(std::fs::read(&protected)?, b"renderer");
                    Ok(())
                })
                .unwrap();
        }
    }

    #[test]
    fn library_generation_reclaims_development_cache_before_reserving_space() {
        let (_dir, state, _) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let source = catalog.source_path(id).unwrap();
        let original = std::fs::read(&source).unwrap();
        let base = state.cache.join(format!(
            "old-photo.{}-0.tif",
            advanced_explorer_develop::RENDER_VERSION
        ));
        std::fs::File::create(&base)
            .unwrap()
            .set_len(964_000_000)
            .unwrap();
        let displayed = state.cache.join("develop-current.png");
        std::fs::write(&displayed, b"displayed image").unwrap();

        assert!(
            state
                .library
                .generate(&state, id, &|| false, false)
                .unwrap()
        );

        assert!(!base.exists());
        assert_eq!(std::fs::read(displayed).unwrap(), b"displayed image");
        assert!(working_bytes(&state.cache).unwrap() + IMPORT_RESERVE <= WORK_LIMIT);
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
        assert!(state.library.lookup(&catalog, id, 384).unwrap().is_some());
        assert_eq!(std::fs::read(source).unwrap(), original);
        assert_eq!(catalog.develop_recipe(id).unwrap().1, 0);
    }

    #[test]
    fn legacy_square_thumbnails_are_repaired_offline_without_reencoding_large_previews() {
        let (dir, state, root) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        prepare(&state, None, |_| {}).unwrap();
        let ids = catalog.preview_items(None, 0).unwrap();
        let mut large_images = Vec::new();
        for id in &ids {
            let large = state.library.lookup(&catalog, *id, 1600).unwrap().unwrap();
            large_images.push((large.path.clone(), std::fs::read(&large.path).unwrap()));
            let record = catalog.library_preview(*id).unwrap().unwrap();
            let mut images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
            let square = image::RgbImage::from_pixel(384, 384, image::Rgb([20, 40, 60]));
            let bytes =
                advanced_explorer_media::preview::encode(&square, true, images.extension == "avif")
                    .unwrap();
            std::fs::write(
                state.library.path(
                    &record.stamp,
                    &images.extension,
                    384,
                    &images.render_version,
                ),
                &bytes,
            )
            .unwrap();
            images.thumbnail = [384, 384];
            images.thumbnail_bytes = bytes.len() as u64;
            catalog
                .save_library_preview(
                    *id,
                    &record.stamp,
                    Some(&serde_json::to_string(&images).unwrap()),
                    None,
                )
                .unwrap();
            assert!(state.library.lookup(&catalog, *id, 384).unwrap().is_none());
        }
        std::fs::rename(&root, dir.path().join("offline-originals")).unwrap();
        let cancelled = AtomicUsize::new(0);
        assert!(
            state
                .library
                .generate(
                    &state,
                    ids[0],
                    &|| cancelled.fetch_add(1, Ordering::SeqCst) > 0,
                    false
                )
                .is_err()
        );
        assert!(
            state
                .library
                .lookup(&catalog, ids[0], 1600)
                .unwrap()
                .is_some(),
            "interrupted repair must retain the existing large preview"
        );
        let repaired = prepare(&state, None, |_| {}).unwrap();
        assert_eq!(
            (repaired.ready, repaired.generated, repaired.failed),
            (2, 2, 0)
        );
        for (id, (path, bytes)) in ids.iter().zip(large_images) {
            assert_eq!(
                std::fs::read(path).unwrap(),
                bytes,
                "large preview must not be reencoded"
            );
            let small = state.library.lookup(&catalog, *id, 384).unwrap().unwrap();
            assert!(matches!(
                (small.render_width, small.render_height),
                (384, 256) | (256, 384)
            ));
        }
    }

    #[test]
    fn square_and_four_three_photos_keep_their_aspect_ratios() {
        let (_dir, state, root) = fixture();
        let cases = [
            ("square.png", 600, 600, (384, 384)),
            ("four-three.png", 800, 600, (384, 288)),
            ("three-four.png", 600, 800, (288, 384)),
        ];
        for (name, width, height, _) in cases {
            image::RgbImage::from_pixel(width, height, image::Rgb([30, 50, 70]))
                .save(root.join(name))
                .unwrap();
        }
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        prepare(&state, None, |_| {}).unwrap();
        let photos = catalog
            .page(&crate::Query::default(), 10, None)
            .unwrap()
            .items;
        for (name, _, _, expected) in cases {
            let id = photos.iter().find(|item| item.name == name).unwrap().id;
            let small = state.library.lookup(&catalog, id, 384).unwrap().unwrap();
            assert_eq!(
                (small.render_width, small.render_height),
                expected,
                "{name}"
            );
            assert!(
                !state
                    .library
                    .generate(&state, id, &|| false, false)
                    .unwrap(),
                "valid thumbnails must not regenerate"
            );
        }
    }

    #[test]
    fn import_generates_both_sizes_without_selection_and_reopens_offline() {
        let (dir, state, root) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        let first = prepare(&state, None, |_| {}).unwrap();
        assert_eq!(
            (first.total, first.ready, first.generated, first.failed),
            (2, 2, 2, 0)
        );
        let ids = catalog.preview_items(None, 0).unwrap();
        let mut before = Vec::new();
        for id in &ids {
            for bound in [384, 1600] {
                let preview = state.library.lookup(&catalog, *id, bound).unwrap().unwrap();
                assert!(preview.path.ends_with(".avif"));
                assert_eq!(
                    preview.render_width.max(preview.render_height),
                    bound as usize
                );
                if bound == 384 {
                    assert!(matches!(
                        (preview.render_width, preview.render_height),
                        (384, 256) | (256, 384)
                    ));
                    let decoded = dir.path().join(format!("decoded-{id}.png"));
                    let info =
                        advanced_explorer_media::render(Path::new(&preview.path), &decoded, 384)
                            .unwrap();
                    assert_eq!(
                        (info.width, info.height),
                        (preview.render_width, preview.render_height)
                    );
                }
                before.push(std::fs::read(preview.path).unwrap());
            }
        }
        let cached_files: Vec<_> = ids.iter().flat_map(|id| [384, 1600].map(|bound| {
            let path = state.library.lookup(&catalog, *id, bound).unwrap().unwrap().path;
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            (path, modified)
        })).collect();
        let last_progress = std::cell::Cell::new((0, 0));
        let second = prepare(&state, None, |report| {
            last_progress.set((report.ready, report.generated));
        }).unwrap();
        assert_eq!((second.ready, second.generated), (2, 0));
        assert_eq!(last_progress.get(), (2, 0), "completion must always reach the UI");
        for (path, modified) in cached_files {
            assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), modified,
                "background cache checks must not mark every photo as recently viewed");
        }
        std::fs::rename(&root, dir.path().join("offline-originals")).unwrap();
        let reopened = Store::new(&state.db).unwrap();
        let mut index = 0;
        for id in ids {
            for bound in [384, 1600] {
                let preview = reopened.lookup(&catalog, id, bound).unwrap().unwrap();
                assert_eq!(before[index], std::fs::read(preview.path).unwrap());
                index += 1;
            }
        }
        let backup = dir.path().join("backup.openroom");
        catalog.backup(&backup).unwrap();
        copy(&state.db, &backup).unwrap();
        let copied = Store::new(&backup).unwrap();
        assert!(
            copied
                .lookup(&Catalog::open(backup).unwrap(), 1, 1600)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn repeated_import_preserves_catalog_and_cache_then_only_adds_new_photos() {
        let (dir, state, root) = fixture();
        let mut catalog = Catalog::open(&state.db).unwrap();
        let root_id = catalog.roots().unwrap()[0].id;
        assert_eq!(
            prepare_import(&state, Some(root_id), |_| {})
                .unwrap()
                .generated,
            2
        );
        let ids = catalog.preview_items(None, 0).unwrap();
        catalog
            .save_develop_recipe(ids[0], r#"{"version":1,"exposure":0.5}"#, 0)
            .unwrap();
        state.library.generate_explicit(&state, ids[0]).unwrap();
        catalog
            .annotate(
                &[ids[0]],
                &advanced_explorer::Annotation {
                    tags: vec!["keep".into()],
                    rating: 5,
                    memo: "keep this edit".into(),
                },
            )
            .unwrap();
        // A completed older render must not be rebuilt just because its folder is re-imported.
        let record = catalog.library_preview(ids[0]).unwrap().unwrap();
        let mut images: Images = serde_json::from_str(record.data.as_ref().unwrap()).unwrap();
        for bound in [384, 1600] {
            let old = state.library.path(
                &record.stamp,
                &images.extension,
                bound,
                &images.render_version,
            );
            let previous =
                state
                    .library
                    .path(&record.stamp, &images.extension, bound, "previous-render");
            std::fs::create_dir_all(previous.parent().unwrap()).unwrap();
            std::fs::rename(old, previous).unwrap();
        }
        images.render_version = "previous-render".into();
        catalog
            .save_library_preview(
                ids[0],
                &record.stamp,
                Some(&serde_json::to_string(&images).unwrap()),
                None,
            )
            .unwrap();
        let snapshot = || {
            let mut paths = Vec::new();
            files(&state.library.directory, &mut paths).unwrap();
            paths.sort();
            paths
                .into_iter()
                .map(|path| {
                    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
                    let bytes = std::fs::read(&path).unwrap();
                    (path, modified, bytes)
                })
                .collect::<Vec<_>>()
        };
        let cached = snapshot();
        let before = dir.path().join("before.openroom");
        catalog.backup(&before).unwrap();
        let database_size = database_bytes(&state.db).unwrap();
        for _ in 0..3 {
            assert_eq!(catalog.add_root(&root).unwrap().id, root_id);
            let scan = catalog.scan_root(root_id, &state.cancel, |_| {}).unwrap();
            assert_eq!((scan.indexed, scan.unchanged), (0, 2));
            let previews = prepare_import(&state, Some(root_id), |_| {}).unwrap();
            assert_eq!(
                (previews.generated, previews.failed, previews.last_ready_id),
                (0, 0, None)
            );
            assert_eq!(
                snapshot(),
                cached,
                "re-import must not write or duplicate cache files"
            );
            assert_eq!(database_bytes(&state.db).unwrap(), database_size);
        }
        let after = dir.path().join("after.openroom");
        catalog.backup(&after).unwrap();
        assert_eq!(
            std::fs::read(before).unwrap(),
            std::fs::read(after).unwrap(),
            "catalog rows, metadata and edit history must stay unchanged"
        );
        image::RgbImage::new(48, 32)
            .save(root.join("new.png"))
            .unwrap();
        let scan = catalog.scan_root(root_id, &state.cancel, |_| {}).unwrap();
        assert_eq!((scan.indexed, scan.unchanged), (1, 2));
        let previews = prepare_import(&state, Some(root_id), |_| {}).unwrap();
        assert_eq!((previews.generated, previews.failed), (1, 0));
        assert_eq!(catalog.preview_items(None, 0).unwrap().len(), 3);
        for (path, modified, bytes) in cached {
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(
                std::fs::metadata(path).unwrap().modified().unwrap(),
                modified
            );
        }
        image::RgbImage::new(72, 48)
            .save(root.join("new.png"))
            .unwrap();
        let scan = catalog.scan_root(root_id, &state.cancel, |_| {}).unwrap();
        assert_eq!((scan.indexed, scan.unchanged), (1, 2));
        assert_eq!(
            prepare_import(&state, Some(root_id), |_| {})
                .unwrap()
                .generated,
            1
        );
        assert_eq!(catalog.preview_items(None, 0).unwrap().len(), 3);
    }

    #[test]
    fn import_records_failures_resumes_and_repairs_truncated_previews() {
        let (_dir, state, root) = fixture();
        std::fs::write(root.join("broken.jpg"), b"invalid photo").unwrap();
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.scan(&state.cancel, |_| {}).unwrap();
        state.cancel.store(true, Ordering::SeqCst);
        let cancelled = prepare_import(&state, None, |_| {}).unwrap();
        assert!(cancelled.cancelled && cancelled.ready == 0);
        state.cancel.store(false, Ordering::SeqCst);
        let report = prepare_import(&state, None, |_| {}).unwrap();
        assert_eq!((report.ready, report.failed), (2, 1));
        let ids = catalog.preview_items(None, 0).unwrap();
        assert!(ids.iter().any(|id| {
            catalog
                .library_preview(*id)
                .unwrap()
                .is_some_and(|r| r.error.is_some())
        }));
        let id = ids
            .into_iter()
            .find(|id| state.library.lookup(&catalog, *id, 1600).unwrap().is_some())
            .unwrap();
        let preview = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        std::fs::write(&preview.path, b"truncated").unwrap();
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_none());
        assert!(state.library.generate(&state, id, &|| false, true).unwrap());
        assert!(std::fs::metadata(preview.path).unwrap().len() > 9);
    }

    #[test]
    fn storage_budget_pauses_bulk_import_and_only_evicts_large_previews() {
        let (_dir, mut state, _) = fixture();
        let large = state.library.directory.join("large/old.avif");
        let small = state.library.directory.join("thumbnails/keep.avif");
        std::fs::create_dir_all(large.parent().unwrap()).unwrap();
        std::fs::create_dir_all(small.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&large).unwrap();
        file.set_len(IMAGE_LIMIT).unwrap(); // Sparse file exercises the quota without allocating gigabytes.
        file.set_modified(SystemTime::now() - std::time::Duration::from_secs(120))
            .unwrap();
        std::fs::write(&small, b"keep thumbnail").unwrap();
        state.library = Arc::new(Store::new(&state.db).unwrap());
        let report = prepare(&state, None, |_| {}).unwrap();
        assert!(report.paused && report.generated == 0);
        assert!(large.exists() && small.exists());
        let id = Catalog::open(&state.db)
            .unwrap()
            .preview_items(None, 0)
            .unwrap()[0];
        *state.library.protected.lock().unwrap() = Some(large.clone());
        assert!(
            state.library.generate(&state, id, &|| false, true).is_err(),
            "the currently displayed preview must not be evicted"
        );
        assert!(large.exists());
        *state.library.protected.lock().unwrap() = None;
        assert!(state.library.generate(&state, id, &|| false, true).unwrap());
        assert!(!large.exists() && small.exists());
        assert!(state.db.exists());
    }

    #[test]
    fn browser_decode_failure_repairs_avif_as_jpeg_from_stored_large_image_offline() {
        let (dir, state, root) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        let ids = catalog.preview_items(None, 0).unwrap();
        prepare(&state, None, |_| {}).unwrap();
        let large = state
            .library
            .lookup(&catalog, ids[0], 1600)
            .unwrap()
            .unwrap();
        let original_bytes = std::fs::read(&large.path).unwrap();
        std::fs::rename(root, dir.path().join("offline")).unwrap();

        state.library.invalidate(&catalog, ids[0], 384).unwrap();
        let checks = AtomicUsize::new(0);
        assert!(
            state
                .library
                .generate(
                    &state,
                    ids[0],
                    &|| checks.fetch_add(1, Ordering::SeqCst) > 1,
                    true
                )
                .is_err()
        );
        assert_eq!(std::fs::read(&large.path).unwrap(), original_bytes);
        assert!(
            state
                .library
                .lookup(&catalog, ids[0], 1600)
                .unwrap()
                .is_some()
        );

        state
            .library
            .generate(&state, ids[0], &|| false, true)
            .unwrap();
        for bound in [384, 1600] {
            let repaired = state
                .library
                .lookup(&catalog, ids[0], bound)
                .unwrap()
                .unwrap();
            assert!(
                repaired.path.ends_with(".jpg"),
                "browser-rejected AVIF must fall back to JPEG"
            );
            assert_eq!(
                image::open(&repaired.path).unwrap().width(),
                repaired.render_width as u32
            );
        }
        assert!(!Path::new(&large.path).exists());
        assert!(
            state
                .library
                .lookup(&catalog, ids[1], 384)
                .unwrap()
                .unwrap()
                .path
                .ends_with(".avif")
        );
        let reopened = Store::new(&state.db).unwrap();
        assert!(
            reopened
                .lookup(&catalog, ids[0], 384)
                .unwrap()
                .unwrap()
                .path
                .ends_with(".jpg")
        );
        assert!(!reopened.generate(&state, ids[0], &|| false, true).unwrap());
    }

    #[test]
    fn jpeg_fallback_replaces_avif_and_corrupt_images_can_be_invalidated() {
        let (_dir, state, _) = fixture();
        let catalog = Catalog::open(&state.db).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        state
            .library
            .generate(&state, id, &|| false, false)
            .unwrap();
        let avif = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        state.library.set_avif_supported(false);
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_none());
        state
            .library
            .generate(&state, id, &|| false, false)
            .unwrap();
        let jpeg = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        assert!(jpeg.path.ends_with(".jpg") && !Path::new(&avif.path).exists());
        assert_eq!(
            image::open(&jpeg.path).unwrap().width(),
            jpeg.render_width as u32
        );
        state.library.invalidate(&catalog, id, 1600).unwrap();
        assert!(!Path::new(&jpeg.path).exists());
        assert!(state.library.lookup(&catalog, id, 384).unwrap().is_some());
        let resumed = prepare(&state, None, |_| {}).unwrap();
        assert_eq!(
            resumed.retained, 1,
            "bulk preparation must not recreate evicted large images"
        );
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_none());
        state.library.generate(&state, id, &|| false, true).unwrap();
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
    }
    #[test]
    fn library_intermediates_match_png_pixels() {
        let (dir, state, root) = fixture();
        let mut sources: Vec<_> = std::fs::read_dir(root).unwrap()
            .map(|file| file.unwrap().path()).collect();
        if let Some(samples) = std::env::var_os("AE_PREVIEW_SAMPLES") {
            sources.extend(std::fs::read_dir(samples).unwrap().map(|file| file.unwrap().path()));
        }
        for (index, source) in sources.iter().enumerate() {
            let png = dir.path().join(format!("{index}.png"));
            let tiff = dir.path().join(format!("{index}.tif"));
            let before = state.renderer.render(source, &png, 1600, &|| false).unwrap();
            let after = state.renderer.render(source, &tiff, 1600, &|| false).unwrap();
            assert_eq!((before.width, before.height, before.render_width, before.render_height),
                (after.width, after.height, after.render_width, after.render_height));
            let before = advanced_explorer_media::preview::read_srgb(&png).unwrap();
            let after = advanced_explorer_media::preview::read_srgb(&tiff).unwrap();
            assert!(before == after, "intermediate pixels differ: {}", source.display());
        }
        println!("PNG/TIFF intermediates: {} images have identical sRGB pixels", sources.len());
    }

    #[test]
    #[ignore = "release benchmark with optional AE_PREVIEW_SAMPLES folder"]
    fn import_preview_benchmark() {
        for round in 0..3 {
            let (_dir, state, root) = fixture();
            if let Some(samples) = std::env::var_os("AE_PREVIEW_SAMPLES") {
                for file in std::fs::read_dir(samples).unwrap() {
                    let file = file.unwrap();
                    std::fs::copy(file.path(), root.join(file.file_name())).unwrap();
                }
            }
            let mut catalog = Catalog::open(&state.db).unwrap();
            catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
            let start = std::time::Instant::now();
            let first = prepare(&state, None, |_| {}).unwrap();
            let generation_ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(first.failed, 0);
            assert_eq!(first.generated, first.total);
            let mut files = Vec::new();
            for id in catalog.preview_items(None, 0).unwrap() {
                for bound in [1600, 384] {
                    let preview = state.library.lookup(&catalog, id, bound).unwrap().unwrap();
                    let bytes = std::fs::read(&preview.path).unwrap();
                    files.push((preview.path, bytes));
                }
            }
            // A resumed import must work without access to any originals.
            std::fs::rename(&root, root.with_extension("offline")).unwrap();
            let mut cached_ms = Vec::new();
            for _ in 0..31 {
                let start = std::time::Instant::now();
                let cached = prepare(&state, None, |_| {}).unwrap();
                cached_ms.push(start.elapsed().as_secs_f64() * 1000.);
                assert_eq!((cached.ready, cached.generated, cached.failed), (first.total, 0, 0));
            }
            for (path, bytes) in files {
                assert_eq!(std::fs::read(path).unwrap(), bytes);
            }
            cached_ms.sort_by(f64::total_cmp);
            println!("preview_benchmark {}", serde_json::json!({
                "round":round,"photos":first.total,"generation_ms":generation_ms,
                "cached_median_ms":cached_ms[15],"cached_p95_ms":cached_ms[29]
            }));
        }
    }

}

#[cfg(test)]
#[path = "library_previews_benchmark.rs"]
mod pipeline_benchmark;
