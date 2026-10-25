// Run with a disposable AE_BENCH_DB copy and AE_TEST_EXECUTABLE pointing at the matching Release worker.
use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn state(directory: &Path) -> AppState {
    std::fs::create_dir_all(directory).unwrap();
    let db = directory.join("catalog.db");
    std::fs::copy(std::env::var_os("AE_BENCH_DB").unwrap(), &db).unwrap();
    let library = Arc::new(Store::new(&db).unwrap());
    let cache = library.directory.join("working");
    std::fs::create_dir_all(&cache).unwrap();
    AppState {
        renderer: crate::Renderer::new(&cache).unwrap(),
        db,
        cache,
        library,
        scanning: Arc::new(AtomicBool::new(false)),
        cancel: Arc::new(AtomicBool::new(false)),
        thumbs: Arc::new(tokio::sync::Semaphore::new(3)),
        previews: Arc::new(tokio::sync::Semaphore::new(1)),
        preview_version: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
        metadata_version: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    }
}

#[test]
#[ignore = "reads opt-in NAS sample catalog; writes only the designated temporary output"]
fn nas_pipeline() {
    let output = PathBuf::from(std::env::var_os("AE_BENCH_OUTPUT").unwrap());
    assert!(!output.exists(), "use a new benchmark output directory");
    let state = state(&output.join("bulk"));
    let catalog = Catalog::open(&state.db).unwrap();
    let ids = catalog.preview_items(None, 0).unwrap();
    assert_eq!(ids.len(), 12);
    let started = Instant::now();
    let mut availability = std::collections::BTreeMap::new();
    let done = AtomicBool::new(false);
    let report = std::thread::scope(|scope| {
        let job = scope.spawn(|| {
            let result = prepare(&state, None, |_| {}).unwrap();
            done.store(true, Ordering::SeqCst);
            result
        });
        while !done.load(Ordering::SeqCst) {
            for id in &ids {
                if !availability.contains_key(id)
                    && state
                        .library
                        .lookup_stored(&catalog, *id, 384)
                        .unwrap()
                        .is_some()
                {
                    availability.insert(*id, started.elapsed().as_secs_f64() * 1000.);
                }
            }
            assert!(started.elapsed() < Duration::from_secs(180));
            std::thread::sleep(Duration::from_millis(2));
        }
        job.join().unwrap()
    });
    let total_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!((report.ready, report.generated, report.failed), (12, 12, 0));
    let started = Instant::now();
    assert_eq!(prepare(&state, None, |_| {}).unwrap().generated, 0);
    let reuse_ms = started.elapsed().as_secs_f64() * 1000.;
    let mut selected = Vec::new();
    for id in &ids {
        let directory = output.join(format!("selected-{id}"));
        let state = self::state(&directory);
        let catalog = Catalog::open(&state.db).unwrap();
        let started = Instant::now();
        let done = AtomicBool::new(false);
        let ready = std::thread::scope(|scope| {
            let job = scope.spawn(|| {
                state.library.generate_explicit(&state, *id).unwrap();
                done.store(true, Ordering::SeqCst);
            });
            let ready = loop {
                if let Some(preview) = state.library.early_preview(&catalog, *id, true).unwrap() {
                    break (started.elapsed().as_secs_f64() * 1000., preview);
                }
                if done.load(Ordering::SeqCst) {
                    break (
                        started.elapsed().as_secs_f64() * 1000.,
                        serde_json::to_value(
                            state
                                .library
                                .lookup_stored(&catalog, *id, 1600)
                                .unwrap()
                                .unwrap(),
                        )
                        .unwrap(),
                    );
                }
                assert!(started.elapsed() < Duration::from_secs(60));
                std::thread::sleep(Duration::from_millis(2));
            };
            job.join().unwrap();
            ready
        });
        let finished_ms = started.elapsed().as_secs_f64() * 1000.;
        let final_preview = state
            .library
            .lookup_stored(&catalog, *id, 1600)
            .unwrap()
            .unwrap();
        std::fs::copy(&final_preview.path, directory.join("final.avif")).unwrap();
        let decoded = directory.join("decoded.tif");
        advanced_explorer_media::render(Path::new(&final_preview.path), &decoded, 1600).unwrap();
        let pixels = advanced_explorer_media::preview::read_srgb(&decoded).unwrap();
        std::fs::write(directory.join("pixels.rgb"), pixels.as_raw()).unwrap();
        if let Some(inline) = ready.1["inline_image"].as_str() {
            use base64::Engine;
            let png = base64::engine::general_purpose::STANDARD
                .decode(inline.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            std::fs::write(directory.join("early.png"), png).unwrap();
        }
        selected.push(serde_json::json!({"id":id,"ready_ms":ready.0,"finished_ms":finished_ms,"inline":ready.1["inline_image"].is_string(),"dimensions":[pixels.width(),pixels.height()]}));
    }
    let result = serde_json::json!({"bulk_ms":total_ms,"thumbnail_ready_ms":availability,"cached_bulk_ms":reuse_ms,"selected":selected});
    std::fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
}
