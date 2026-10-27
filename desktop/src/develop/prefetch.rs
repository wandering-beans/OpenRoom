//! Disposable, display-size development inputs. Only one isolated decoder runs at a time.
use super::*;
use std::{collections::HashMap, time::Duration};

#[derive(Default)]
pub struct Prefetch {
    generation: AtomicUsize,
    selection: Mutex<Selection>,
    job: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Selection {
    ids: Vec<i64>,
    ready: HashMap<i64, Arc<Entry>>,
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
        let _job = self.job.lock().await;
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
        selection.ids = ids;
        Some(version)
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

    fn publish(&self, version: usize, id: i64, entry: Entry) {
        let mut selection = self.selection.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) == version && selection.ids.contains(&id) {
            selection.ready.insert(id, Arc::new(entry));
        }
        // Dropping an obsolete entry also removes its complete temporary directory.
    }
}

impl Entry {
    pub(super) fn preview(
        &self,
        request: &Request,
        resident: &Mutex<Option<Resident>>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<DevelopedPreview> {
        let mut frame = self.frame.clone();
        let png = std::fs::read(self.directory.path().join("preview.png"))?;
        let base = engine::read_base(&self.directory.path().join("base.tif"), frame.base.clone())?;
        remember_base(base, request, 1600, resident, true, cancelled)?;
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
                    let cancelled = || {
                        cache.generation.load(Ordering::SeqCst) != version
                            || copied.library.updates.generation.load(Ordering::SeqCst)
                                != foreground
                            || copied.library.updates.busy()
                    };
                    ensure!(!cancelled(), "Prefetch cancelled");
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
                    if cache.get(id, &request).is_some() {
                        return Ok(());
                    }
                    let entry = prepare(&copied, &request, &settings, &cancelled)?;
                    ensure!(!cancelled(), "Prefetch cancelled");
                    cache.publish(version, id, entry);
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
    let (directory, frame) = state.library.with_work_space(state, 64_000_000, || {
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
        let version = develop.prefetch.replace(vec![id]).unwrap();
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        develop.prefetch.publish(version, id, entry);
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
        let version = develop.prefetch.replace(vec![id]).unwrap();
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        let expected = std::fs::read(entry.directory.path().join("preview.png")).unwrap();
        let temporary = entry.directory.path().to_owned();
        develop.prefetch.publish(version, id, entry);
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
        assert!(develop.resident.lock().unwrap().is_some());
        assert!(
            develop
                .prefetch
                .get(id, &make_request(&Settings::default()))
                .is_none()
        );
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        develop.prefetch.publish(version, id, entry);
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
        let version = cache.replace(vec![id]).unwrap();
        let entry = prepare(&state, &request, &settings, &|| false).unwrap();
        let directory = entry.directory.path().to_owned();
        cache.publish(version, id, entry);
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
        cache.publish(version, id, late);
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
