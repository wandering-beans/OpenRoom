use super::{tests::fixture, *};
use std::{fs, sync::Barrier};

fn both(store: &Store, catalog: &Catalog, id: i64) -> Vec<(String, Vec<u8>)> {
    [384, 1600]
        .map(|bound| {
            let preview = store.lookup_stored(catalog, id, bound).unwrap().unwrap();
            let bytes = fs::read(&preview.path).unwrap();
            (preview.path, bytes)
        })
        .into()
}

fn concurrent_then_relink(sample: Option<PathBuf>) {
    let (dir, state, root) = fixture();
    let mut cat = Catalog::open(&state.db).unwrap();
    let root_id = cat.roots().unwrap()[0].id;
    let id = if let Some(sample) = sample {
        fs::copy(&sample, root.join(sample.file_name().unwrap())).unwrap();
        cat.scan(&AtomicBool::new(false), |_| {}).unwrap();
        cat.preview_items(None, 0)
            .unwrap()
            .into_iter()
            .find(|&id| Store::is_raw(&cat, id).unwrap())
            .unwrap()
    } else {
        cat.preview_items(None, 0).unwrap()[0]
    };
    state.library.set_avif_supported(false);
    let gate = Barrier::new(3);
    std::thread::scope(|scope| {
        for priority in 0..3 {
            let gate = &gate;
            let state = &state;
            scope.spawn(move || {
                let catalog = Catalog::open(&state.db).unwrap();
                gate.wait();
                state
                    .library
                    .generate_inner(state, &catalog, id, &|| false, false, true, priority)
                    .unwrap();
            });
        }
    });
    let initial = state.library.counts.lock().unwrap()[&id];
    assert_eq!(
        initial,
        WorkCounts {
            requests: 3,
            renders: 1,
            encodes: 2
        }
    );
    let original = both(&state.library, &cat, id);
    if Store::is_raw(&cat, id).unwrap() {
        let data: Images =
            serde_json::from_str(&cat.library_preview(id).unwrap().unwrap().data.unwrap()).unwrap();
        assert!(data.raw_developed && !data.camera_fallback);
    }
    let moved = dir.path().join("renamed");
    fs::rename(&root, &moved).unwrap();
    let result = state
        .library
        .relink_folder(&mut cat, root_id, Path::new(""), &moved, || false)
        .unwrap();
    assert_eq!(result.not_found + result.unreadable + result.conflicts, 0);
    assert_eq!(both(&state.library, &cat, id), original);
    assert!(!state.library.generate_explicit(&state, id).unwrap());
    let work = state.library.counts.lock().unwrap()[&id];
    assert_eq!((work.renders, work.encodes), (1, 2));
    let moved_again = dir.path().join("renamed-again");
    fs::rename(&moved, &moved_again).unwrap();
    let id2 = cat.roots().unwrap()[0].id;
    state
        .library
        .relink_folder(&mut cat, id2, Path::new(""), &moved_again, || false)
        .unwrap();
    let backup = dir.path().join("backup.openroom");
    state.library.backup(&state.db, &backup).unwrap();
    let restored = Store::new(&backup).unwrap();
    restored.set_avif_supported(false);
    let copy = both(&restored, &Catalog::open(&backup).unwrap(), id);
    for (a, b) in original.iter().zip(copy) {
        assert_eq!(a.1, b.1);
    }
    println!("Before rename: {initial:?}. After repeated rename: 0 renders, 0 encodes.");
}

#[test]
fn concurrent_requests_and_repeated_relinks_reuse_the_pair() {
    concurrent_then_relink(None);
}

#[test]
#[ignore = "requires AE_PREVIEW_PHOTO and AE_TEST_EXECUTABLE"]
fn native_raw_relocation_counts() {
    concurrent_then_relink(Some(std::env::var_os("AE_PREVIEW_PHOTO").unwrap().into()));
}

#[test]
fn relink_repairs_only_bad_thumbnail_and_rejects_a_different_original() {
    let (dir, state, root) = fixture();
    let mut cat = Catalog::open(&state.db).unwrap();
    state.library.set_avif_supported(false);
    prepare_pending(&state, |_| {}).unwrap();
    let root_id = cat.roots().unwrap()[0].id;
    let ids = cat.preview_items(None, 0).unwrap();
    let original = both(&state.library, &cat, ids[0]);
    let mut damaged = original[0].1.clone();
    damaged[0] ^= 1;
    fs::write(&original[0].0, damaged).unwrap();
    let moved = dir.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    // Keep the original inode alive; create a different file with identical catalog metadata.
    let file = moved.join(&cat.detail(ids[1]).unwrap().item.path);
    let kept = dir.path().join("original");
    fs::rename(&file, &kept).unwrap();
    fs::copy(&kept, &file).unwrap();
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(fs::metadata(&kept).unwrap().modified().unwrap())
        .unwrap();
    let report = state
        .library
        .relink_folder(&mut cat, root_id, Path::new(""), &moved, || false)
        .unwrap();
    assert_eq!(report.linked, 2);
    assert!(
        state
            .library
            .lookup_stored(&cat, ids[0], 384)
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .library
            .lookup_stored(&cat, ids[0], 1600)
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .library
            .lookup_stored(&cat, ids[1], 1600)
            .unwrap()
            .is_none()
    );
    let counts = state.library.counts.lock().unwrap().clone();
    assert_eq!(prepare_pending(&state, |_| {}).unwrap().generated, 2);
    let after = state.library.counts.lock().unwrap();
    assert_eq!(after[&ids[0]].renders, counts[&ids[0]].renders);
    assert_eq!(after[&ids[0]].encodes, counts[&ids[0]].encodes + 1);
    assert_eq!(after[&ids[1]].renders, counts[&ids[1]].renders + 1);
    assert_eq!(both(&state.library, &cat, ids[0])[1], original[1]);
}

#[test]
fn cancel_rolls_back_preview_and_stale_edit_cannot_inherit_pixels() {
    let (dir, state, root) = fixture();
    let mut cat = Catalog::open(&state.db).unwrap();
    state.library.set_avif_supported(false);
    prepare_pending(&state, |_| {}).unwrap();
    let root_id = cat.roots().unwrap()[0].id;
    let ids = cat.preview_items(None, 0).unwrap();
    let old = cat.library_preview(ids[0]).unwrap().unwrap();
    let moved = dir.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    let checked = std::cell::Cell::new(false);
    assert!(
        cat.relink_folder_with_previews(
            root_id,
            Path::new(""),
            &moved,
            || checked.get(),
            |record, path| {
                let data = state.library.reusable_images(record, path);
                assert!(data.is_some());
                checked.set(true);
                data
            }
        )
        .is_err()
    );
    assert!(checked.get());
    assert_eq!(cat.roots().unwrap()[0].id, root_id);
    assert_eq!(
        cat.library_preview(ids[0]).unwrap().unwrap().stamp,
        old.stamp
    );
    let recipe = advanced_explorer_develop::Settings {
        exposure: 1.,
        ..Default::default()
    };
    cat.save_develop_recipe(ids[0], &recipe.catalog_recipe().unwrap(), 0)
        .unwrap();
    let old2 = cat.library_preview(ids[1]).unwrap().unwrap();
    let mut legacy: Images = serde_json::from_str(old2.data.as_ref().unwrap()).unwrap();
    legacy.binding = None;
    cat.save_library_preview(
        ids[1],
        &old2.stamp,
        Some(&serde_json::to_string(&legacy).unwrap()),
        None,
    )
    .unwrap();
    state
        .library
        .relink_folder(&mut cat, root_id, Path::new(""), &moved, || false)
        .unwrap();
    for id in ids {
        assert_ne!(
            cat.preview_stamp(id).unwrap(),
            cat.library_preview(id).unwrap().unwrap().stamp
        );
    }
}

#[test]
fn different_file_restored_to_the_same_path_must_render_again() {
    let (dir, state, root) = fixture();
    let mut cat = Catalog::open(&state.db).unwrap();
    let root_id = cat.roots().unwrap()[0].id;
    let id = cat.preview_items(None, 0).unwrap()[0];
    state.library.set_avif_supported(false);
    assert!(state.library.generate_explicit(&state, id).unwrap());
    let target = root.join(&cat.detail(id).unwrap().item.path);
    let kept = dir.path().join("original-offline");
    fs::rename(&target, &kept).unwrap();
    cat.scan(&AtomicBool::new(false), |_| {}).unwrap();
    assert!(cat.detail(id).unwrap().item.missing);
    fs::copy(&kept, &target).unwrap();
    fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_modified(fs::metadata(&kept).unwrap().modified().unwrap())
        .unwrap();
    let result = state
        .library
        .relink_folder(&mut cat, root_id, Path::new(""), &root, || false)
        .unwrap();
    assert_eq!(result.linked, 1);
    assert!(
        state
            .library
            .lookup_stored(&cat, id, 1600)
            .unwrap()
            .is_none()
    );
    assert!(state.library.generate_explicit(&state, id).unwrap());
    assert_eq!(state.library.counts.lock().unwrap()[&id].renders, 2);
}

#[test]
#[ignore = "bulk relocation acceptance"]
fn thousand_completed_previews_survive_reopen() {
    let (dir, mut state, root) = fixture();
    for id in 2..1000u16 {
        image::RgbImage::from_pixel(12, 8, image::Rgb([id as u8, (id >> 8) as u8, 70]))
            .save(root.join(format!("{id}.jpg")))
            .unwrap();
    }
    let mut cat = Catalog::open(&state.db).unwrap();
    cat.scan(&AtomicBool::new(false), |_| {}).unwrap();
    state.library.set_avif_supported(false);
    let initial = prepare_pending(&state, |_| {}).unwrap();
    assert_eq!((initial.generated, initial.failed), (1000, 0));
    let before = state.library.usage.lock().unwrap().files.clone();
    let moved = dir.path().join("renamed");
    fs::rename(&root, &moved).unwrap();
    let root_id = cat.roots().unwrap()[0].id;
    let t = std::time::Instant::now();
    let result = state
        .library
        .relink_folder(&mut cat, root_id, Path::new(""), &moved, || false)
        .unwrap();
    assert_eq!(result.linked, 1000);
    state.library = std::sync::Arc::new(Store::new(&state.db).unwrap());
    state.library.set_avif_supported(false);
    assert_eq!(prepare_pending(&state, |_| {}).unwrap().total, 0);
    assert_eq!(state.library.usage.lock().unwrap().files, before);
    let mut after_id = 0;
    loop {
        let page = cat.preview_items(None, after_id).unwrap();
        if page.is_empty() {
            break;
        }
        for id in page {
            for bound in [384, 1600] {
                assert!(
                    state
                        .library
                        .lookup_stored(&cat, id, bound)
                        .unwrap()
                        .is_some()
                );
            }
            after_id = id;
        }
    }
    assert!(state.library.counts.lock().unwrap().is_empty());
    println!(
        "1000 photos, 2000 usable images, zero render/encode, unchanged disk allocation; relocation and verification {:?}",
        t.elapsed()
    );
}
