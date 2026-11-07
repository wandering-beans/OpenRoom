//! Up to seven display-size inputs, with the interactive decoder warmed only for the selection.
use super::*;
use std::{
    collections::HashMap,
    sync::{Weak, atomic::AtomicBool},
    time::Duration,
};

#[derive(Default)]
pub(super) struct Flight {
    pub(super) lock: Arc<tokio::sync::Mutex<()>>,
    removed: AtomicBool,
    pub(super) error: Mutex<Option<String>>,
}

#[derive(Default)]
pub struct Prefetch {
    generation: AtomicUsize,
    selection: Mutex<Selection>,
    job: tokio::sync::Mutex<()>,
    pub(super) base_job: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct Selection {
    ids: Vec<i64>,
    ready: HashMap<i64, Arc<Entry>>,
    flights: HashMap<(i64, PathBuf), Weak<Flight>>,
}

pub(super) struct Entry {
    key: PathBuf,
    store: Arc<crate::library_previews::Store>,
    directory: tempfile::TempDir,
    frame: DevelopedPreview,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.store.discard_work_directory(self.directory.path());
    }
}

impl Prefetch {
    pub(super) async fn shutdown(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.replace(Vec::new());
        let _job = self.job.lock().await;
        let _base = self.base_job.lock().await;
        let mut selection = self.selection.lock().unwrap();
        selection.ids.clear();
        selection.ready.clear();
    }

    fn replace(&self, ids: Vec<i64>) -> Option<usize> {
        let mut selection = self.selection.lock().unwrap();
        if selection.ids == ids {
            return None;
        }
        let version = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        selection.ready.retain(|id, _| ids.contains(id));
        selection.flights.retain(|(id, _), flight| {
            let Some(flight) = flight.upgrade() else {
                return false;
            };
            if ids.contains(id) {
                return true;
            }
            flight.removed.store(true, Ordering::SeqCst);
            false
        });
        selection.ids = ids;
        Some(version)
    }

    pub(super) fn flight(&self, id: i64, request: &Request) -> Arc<Flight> {
        let mut selection = self.selection.lock().unwrap();
        selection.flights.retain(|_, weak| weak.strong_count() > 0);
        let weak = selection
            .flights
            .entry((id, request.image.clone()))
            .or_default();
        if let Some(existing) = weak.upgrade() {
            return existing;
        }
        let flight = Arc::new(Flight::default());
        *weak = Arc::downgrade(&flight);
        flight
    }

    pub(super) fn get(&self, id: i64, request: &Request) -> Option<Arc<Entry>> {
        let mut selection = self.selection.lock().unwrap();
        if selection
            .ready
            .get(&id)
            .is_some_and(|entry| entry.key != request.image)
        {
            selection.ready.remove(&id);
        }
        selection.ready.get(&id).cloned()
    }

    fn publish(&self, id: i64, flight: &Flight, entry: Entry) {
        let mut selection = self.selection.lock().unwrap();
        if !flight.removed.load(Ordering::SeqCst) && selection.ids.contains(&id) {
            selection.ready.insert(id, Arc::new(entry));
        }
        // Dropping an obsolete entry also removes its complete temporary directory.
    }
}

impl Entry {
    pub(super) fn load_base(
        &self,
        request: &Request,
        resident: &Mutex<Option<Resident>>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        let base = engine::read_base(
            &self.directory.path().join("base.tif"),
            self.frame.base.clone(),
        )?;
        remember_base(base, request, 1600, resident, true, cancelled)?;
        Ok(())
    }

    pub(super) fn preview(&self) -> Result<DevelopedPreview> {
        let mut frame = self.frame.clone();
        let path = self.directory.path().join("preview.png");
        ensure!(
            self.frame.image_bytes > 0
                && std::fs::metadata(&path)?.len() == self.frame.image_bytes
                && image::image_dimensions(&path)?
                    == (self.frame.info.width, self.frame.info.height),
            "Invalid prefetched image"
        );
        let png = std::fs::read(path)?;
        // The UI may finish loading after navigation has already deleted this entry.
        frame.path.clear();
        frame.inline_image = Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ));
        Ok(frame)
    }
}

#[tauri::command]
pub async fn set_develop_prefetch(
    state: State<'_, AppState>,
    develop: State<'_, DevelopState>,
    ids: Vec<i64>,
) -> Reply<()> {
    start((*state).clone(), develop.prefetch.clone(), ids)
}

fn start(state: AppState, prefetch: Arc<Prefetch>, ids: Vec<i64>) -> Reply<()> {
    if ids.len() > 7
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id))
    {
        return Err("Invalid development prefetch selection".into());
    }
    let Some(version) = prefetch.replace(ids.clone()) else {
        return Ok(());
    };
    tauri::async_runtime::spawn(async move {
        let _job = prefetch.job.lock().await;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let selected = ids.first().copied();
        for id in ids {
            loop {
                if prefetch.generation.load(Ordering::SeqCst) != version {
                    return;
                }
                // Debounce navigation and give foreground renders/sliders priority.
                if state.library.updates.busy() {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                let foreground = state.library.updates.generation.load(Ordering::SeqCst);
                let copied = state.clone();
                let cache = prefetch.clone();
                let _ = work(move || {
                    let catalog = Catalog::open(&copied.db)?;
                    let (recipe, _) = catalog.develop_recipe(id)?;
                    let settings = recipe
                        .map(|json| serde_json::from_str::<Settings>(&json))
                        .transpose()?
                        .unwrap_or_default();
                    let request = request(
                        copied.db.clone(),
                        copied.cache.clone(),
                        copied.renderer.clone(),
                        id,
                        &settings,
                        1600,
                    )?;
                    let flight = cache.flight(id, &request);
                    let owner = flight.lock.blocking_lock();
                    if cache.generation.load(Ordering::SeqCst) != version {
                        return Ok(());
                    }
                    let cancelled = || flight.removed.load(Ordering::SeqCst);
                    let display_needed =
                        cache.get(id, &request).is_none() && cached(&request).is_none();
                    std::thread::scope(|_scope| {
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        if Some(id) == selected
                            && engine::is_native_raw(&request.source)
                            && (!cfg!(test) || std::env::var_os("AE_TEST_EXECUTABLE").is_some())
                        {
                            _scope.spawn(|| {
                                let obsolete = || {
                                    cancelled()
                                        || copied.library.updates.active.load(Ordering::SeqCst)
                                            != id
                                };
                                let _ = crate::rtengine_worker::warm_interactive(
                                    &request.source,
                                    &settings,
                                    &obsolete,
                                );
                            });
                        }
                        if display_needed {
                            match prepare(&copied, &request, &settings, &cancelled) {
                                Ok(entry) => cache.publish(id, &flight, entry),
                                Err(error) if !cancelled() => {
                                    *flight.error.lock().unwrap() = Some(format!("{error:#}"))
                                }
                                Err(_) => (),
                            }
                        }
                        // A matching foreground display can consume the PNG before the drag worker finishes.
                        drop(owner);
                    });
                    Ok(())
                })
                .await;
                if prefetch.generation.load(Ordering::SeqCst) != version {
                    return;
                }
                if state.library.updates.generation.load(Ordering::SeqCst) != foreground {
                    continue;
                }
                // Missing sources and decoder failures must not become a background retry loop.
                break;
            }
        }
    });
    Ok(())
}

fn prepare(
    state: &AppState,
    request: &Request,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Entry> {
    let (directory, frame) = state.library.with_private_workspace(|| {
        ensure!(!cancelled(), "Prefetch cancelled");
        let directory = tempfile::Builder::new()
            .prefix("develop-prefetch-")
            .tempdir_in(&state.cache)?;
        let recipe = directory.path().join("recipe.json");
        std::fs::write(&recipe, serde_json::to_vec(settings)?)?;
        let frame = if cfg!(test) && std::env::var_os("AE_TEST_EXECUTABLE").is_none() {
            render_worker(&request.source, directory.path(), settings)?
        } else {
            let mut command = std::process::Command::new(media::worker_executable()?);
            command
                .arg("--develop-prefetch")
                .arg(&request.source)
                .arg(directory.path())
                .arg(recipe)
                .env("OMP_NUM_THREADS", "2")
                .env("OMP_MAX_ACTIVE_LEVELS", "1");
            media::run_worker(command, None, cancelled)?
        };
        ensure!(!cancelled(), "Prefetch cancelled");
        Ok((directory, frame))
    })?;
    Ok(Entry {
        key: request.image.clone(),
        store: state.library.clone(),
        directory,
        frame,
    })
}

pub(super) fn render_worker(
    source: &Path,
    directory: &Path,
    settings: &Settings,
) -> Result<DevelopedPreview> {
    settings.validate()?;
    let base = engine::prepare(source, 1600)?;
    let rendered = engine::render(&base, settings, &|| false)?;
    engine::write_base(&directory.join("base.tif"), &base)?;
    let output = directory.join("preview.png");
    advanced_explorer_media::write_preview(&output, &rendered.image)?;
    let mut info = base.info;
    engine::refresh_raw_profile(&mut info);
    Ok(DevelopedPreview {
        path: output.to_string_lossy().into_owned(),
        inline_image: None,
        image_bytes: std::fs::metadata(output)?.len(),
        info: rendered.info,
        base: info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires AE_PREVIEW_PHOTO and the built AE_TEST_EXECUTABLE"]
    fn raw_prefetch_matches_foreground_and_cancellation_removes_worker_outputs() {
        let (_directory, state, _) = super::super::tests::fixture();
        let original = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap());
        assert!(std::env::var_os("AE_TEST_EXECUTABLE").is_some());
        let source = state
            .db
            .parent()
            .unwrap()
            .join("photos")
            .join(original.file_name().unwrap());
        std::fs::copy(&original, &source).unwrap();
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog
            .scan(&std::sync::atomic::AtomicBool::new(false), |_| {})
            .unwrap();
        let id = catalog
            .page(
                &advanced_explorer::Query {
                    text: original.file_name().unwrap().to_string_lossy().into_owned(),
                    ..Default::default()
                },
                1,
                None,
            )
            .unwrap()
            .items[0]
            .id;
        let settings = Settings {
            contrast: 12.,
            saturation: 8.,
            ..Default::default()
        };
        let develop = DevelopState::new().unwrap();
        let started = std::time::Instant::now();
        let cold = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            settings.clone(),
            1600,
        ))
        .unwrap();
        let cold_ms = started.elapsed().as_secs_f64() * 1000.;
        let expected = image::open(&cold.path).unwrap().into_rgba8();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &settings,
            1600,
        )
        .unwrap();
        let cancelled_at = std::time::Instant::now();
        assert!(
            prepare(&state, &request, &settings, &|| cancelled_at.elapsed()
                > Duration::from_millis(50))
            .is_err()
        );
        assert!(cancelled_at.elapsed() < Duration::from_secs(5));
        develop.prefetch.replace(vec![id]).unwrap();
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        develop
            .prefetch
            .publish(id, &develop.prefetch.flight(id, &request), entry);
        *develop.resident.lock().unwrap() = None;
        let started = std::time::Instant::now();
        let warm =
            tauri::async_runtime::block_on(preview_inner(&state, &develop, id, settings, 1600))
                .unwrap();
        let warm_ms = started.elapsed().as_secs_f64() * 1000.;
        let png = base64::engine::general_purpose::STANDARD
            .decode(
                warm.inline_image
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgba8(),
            expected
        );
        develop.shutdown();
        assert!(!std::fs::read_dir(&state.cache).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("develop-prefetch-")
        }));
        assert_eq!(
            std::fs::read(original).unwrap(),
            std::fs::read(source).unwrap()
        );
        assert_eq!(catalog.develop_recipe(id).unwrap().1, 0);
        println!(
            "prefetch: cold={cold_ms:.1}ms warm={warm_ms:.1}ms; identical pixels, original unchanged, no temporary files"
        );
    }

    #[test]
    fn queue_waits_for_foreground_and_keeps_only_the_latest_window() {
        let (_directory, state, id) = super::super::tests::fixture();
        let source = Catalog::open(&state.db).unwrap().source_path(id).unwrap();
        for index in 1..7 {
            std::fs::copy(&source, source.with_file_name(format!("photo-{index}.png"))).unwrap();
        }
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog
            .scan(&std::sync::atomic::AtomicBool::new(false), |_| {})
            .unwrap();
        let ids: Vec<_> = catalog
            .page(&Default::default(), 10, None)
            .unwrap()
            .items
            .into_iter()
            .map(|photo| photo.id)
            .collect();
        let prefetch = Arc::new(Prefetch::default());
        assert!(start((*state).clone(), prefetch.clone(), vec![id; 8]).is_err());
        assert!(start((*state).clone(), prefetch.clone(), vec![id, id]).is_err());
        assert!(start((*state).clone(), prefetch.clone(), vec![0]).is_err());
        let foreground = state.library.updates.foreground();
        start((*state).clone(), prefetch.clone(), ids.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        assert!(prefetch.selection.lock().unwrap().ready.is_empty());
        drop(foreground);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while prefetch.selection.lock().unwrap().ready.len() != 7 {
            assert!(
                std::time::Instant::now() < deadline,
                "prefetch queue did not resume"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        start((*state).clone(), prefetch.clone(), vec![id]).unwrap();
        assert_eq!(prefetch.selection.lock().unwrap().ready.len(), 1);
        start((*state).clone(), prefetch.clone(), vec![]).unwrap();
        tauri::async_runtime::block_on(prefetch.shutdown());
        assert!(prefetch.selection.lock().unwrap().ready.is_empty());
        assert!(!std::fs::read_dir(&state.cache).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("develop-prefetch-")
        }));
    }

    #[test]
    fn prefetch_reuses_exact_pixels_and_rejects_changed_settings_or_source() {
        let (_directory, state, id) = super::super::tests::fixture();
        let develop = DevelopState::new().unwrap();
        let settings = Settings {
            exposure: 0.5,
            ..Default::default()
        };
        let make_request = |settings: &Settings| {
            request(
                state.db.clone(),
                state.cache.clone(),
                state.renderer.clone(),
                id,
                settings,
                1600,
            )
            .unwrap()
        };
        let request = make_request(&settings);
        let original = std::fs::read(&request.source).unwrap();
        develop.prefetch.replace(vec![id]).unwrap();
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        let expected = std::fs::read(entry.directory.path().join("preview.png")).unwrap();
        let temporary = entry.directory.path().to_owned();
        // A damaged editable TIFF must not hide an otherwise valid prefetched PNG.
        std::fs::write(temporary.join("base.tif"), b"damaged").unwrap();
        develop
            .prefetch
            .publish(id, &develop.prefetch.flight(id, &request), entry);
        // A hit must not enter the normal decoder queue.
        let _busy = tauri::async_runtime::block_on(develop.jobs.clone().acquire_owned()).unwrap();
        let frame = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            settings.clone(),
            1600,
        ))
        .unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(
                frame
                    .inline_image
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(png, expected);
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgba8(),
            engine::render(
                &engine::prepare(&request.source, 1600).unwrap(),
                &settings,
                &|| false
            )
            .unwrap()
            .image
        );
        // Display succeeds independently of asynchronously loading the editable base.
        assert!(
            develop
                .prefetch
                .get(id, &make_request(&Settings::default()))
                .is_none()
        );
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        develop
            .prefetch
            .publish(id, &develop.prefetch.flight(id, &request), entry);
        image::RgbaImage::from_pixel(68, 37, image::Rgba([10, 20, 30, 255]))
            .save(&request.source)
            .unwrap();
        assert!(develop.prefetch.get(id, &make_request(&settings)).is_none());
        std::fs::write(&request.source, original).unwrap();
        assert_eq!(
            Catalog::open(&state.db)
                .unwrap()
                .develop_recipe(id)
                .unwrap()
                .1,
            0
        );
        develop.shutdown();
        assert!(!temporary.exists());
    }

    #[test]
    fn foreground_joins_running_prefetch_and_cancelled_waiters_release() {
        let (_directory, state, id) = super::super::tests::fixture();
        let develop = DevelopState::new().unwrap();
        let settings = Settings::default();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &settings,
            1600,
        )
        .unwrap();
        // Force a cold request: without the published entry this consumer would need a new decoder.
        std::fs::remove_file(&request.base).unwrap();
        std::fs::remove_file(&request.base_info).unwrap();
        develop.prefetch.replace(vec![id]).unwrap();
        let flight = develop.prefetch.flight(id, &request);
        tauri::async_runtime::block_on(async {
            let owner = flight.lock.lock().await;
            let foreground = preview_inner(&state, &develop, id, settings.clone(), 1600);
            tokio::pin!(foreground);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut foreground)
                    .await
                    .is_err()
            );
            develop.generation.fetch_add(1, Ordering::SeqCst);
            assert!(
                tokio::time::timeout(Duration::from_secs(2), foreground)
                    .await
                    .unwrap()
                    .is_err()
            );
            let failed = preview_inner(&state, &develop, id, settings.clone(), 1600);
            tokio::pin!(failed);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut failed)
                    .await
                    .is_err()
            );
            *flight.error.lock().unwrap() = Some("decoder failed".into());
            drop(owner);
            assert_eq!(failed.await.err().as_deref(), Some("decoder failed"));
            let owner = flight.lock.lock().await;
            *flight.error.lock().unwrap() = None;
            // Only the original owner prepares pixels. The foreground cannot use its normal decoder.
            let _blocked = develop.jobs.acquire().await.unwrap();
            let consumer = preview_inner(&state, &develop, id, settings.clone(), 1600);
            tokio::pin!(consumer);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut consumer)
                    .await
                    .is_err()
            );
            let entry = prepare(&state, &request, &settings, &|| false).unwrap();
            develop
                .prefetch
                .publish(id, &develop.prefetch.flight(id, &request), entry);
            drop(owner);
            assert!(
                tokio::time::timeout(Duration::from_secs(2), consumer)
                    .await
                    .unwrap()
                    .unwrap()
                    .inline_image
                    .is_some()
            );
        });
        develop.shutdown();
    }

    #[test]
    fn leaving_window_discards_results_and_late_completion_cannot_restore_them() {
        let (_directory, state, id) = super::super::tests::fixture();
        let cache = Prefetch::default();
        let settings = Settings::default();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &settings,
            1600,
        )
        .unwrap();
        cache.replace(vec![id]).unwrap();
        let flight = cache.flight(id, &request);
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        let directory = entry.directory.path().to_owned();
        cache.publish(id, &flight, entry);
        assert!(cache.replace(vec![id]).is_none());
        cache.replace(vec![id, id + 1]);
        assert!(
            directory.exists(),
            "overlapping neighbors should survive navigation"
        );
        cache.replace(vec![id + 1]);
        assert!(!directory.exists(), "out-of-window data must be deleted");
        let late = prepare(&state, &request, &settings, &|| false).unwrap();
        let late_directory = late.directory.path().to_owned();
        cache.replace(vec![id]);
        cache.publish(id, &flight, late);
        assert!(!late_directory.exists());
        assert!(cache.get(id, &request).is_none());
        assert!(prepare(&state, &request, &settings, &|| true).is_err());
        assert!(!std::fs::read_dir(&state.cache).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("develop-prefetch-")
        }));
    }
}
