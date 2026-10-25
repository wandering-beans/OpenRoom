use advanced_explorer::*;
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;
fn setup() -> (TempDir, TempDir, Catalog) {
    let root = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    let c = Catalog::open(db.path().join("test.db")).unwrap();
    c.add_root(root.path()).unwrap();
    (root, db, c)
}
fn scan(c: &mut Catalog) {
    let r = c.scan(&AtomicBool::new(false), |_| {}).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!r.cancelled);
}
#[test]
fn multiple_roots_are_atomic_and_preserve_originals() {
    let directory = TempDir::new().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("photo.jpg"), b"original").unwrap();
    let catalog = Catalog::open(directory.path().join("catalog.db")).unwrap();
    assert!(
        catalog
            .add_roots(&[first.clone(), directory.path().join("missing")])
            .is_err()
    );
    assert!(catalog.roots().unwrap().is_empty());
    assert!(
        catalog
            .add_roots(&[first.clone(), directory.path().to_owned()])
            .is_err()
    );
    assert!(catalog.roots().unwrap().is_empty());
    let roots = catalog.add_roots(&[first.clone(), second.clone()]).unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(
        catalog.add_roots(&[first.clone(), second]).unwrap()[0].id,
        roots[0].id
    );
    assert_eq!(catalog.roots().unwrap().len(), 2);
    assert_eq!(std::fs::read(first.join("photo.jpg")).unwrap(), b"original");
}
fn all(c: &Catalog, q: &Query) -> Vec<Item> {
    let mut out = vec![];
    let mut cursor = None;
    loop {
        let page = c.page(q, 7, cursor.as_ref()).unwrap();
        out.extend(page.items);
        cursor = page.next;
        if cursor.is_none() {
            return out;
        }
    }
}
#[test]
fn search_matches_reference_and_pagination() {
    let (root, _db, mut c) = setup();
    let names = [
        "猫",
        "日本",
        "東京写真",
        "ＡＢＣ",
        "cafe\u{301}",
        "a%b_c",
        if cfg!(windows) { "x_OR_y" } else { "x\"OR\"y" },
        "絵😀画",
    ];
    for n in 0..211 {
        std::fs::write(
            root.path()
                .join(format!("{}-{n:04}.png", names[n % names.len()])),
            vec![1; n % 19],
        )
        .unwrap();
    }
    scan(&mut c);
    let initial = all(&c, &Query::default());
    for (n, item) in initial.iter().enumerate() {
        c.annotate(
            &[item.id],
            &Annotation {
                tags: vec![format!("タグ{}", n % 5)],
                rating: (n % 6) as u8,
                memo: format!("メモ 日本語 {}", n % 11),
            },
        )
        .unwrap();
    }
    for text in [
        "",
        "猫",
        "日本",
        "東京写",
        "abc",
        "café",
        "%",
        "_",
        if cfg!(windows) { "x_OR_y" } else { "x\"OR\"y" },
        "😀",
        "タグ1",
        "メモ 3",
        "日本語 png",
        "存在しない",
        "ＰＮＧ",
        "OR",
        "\"",
        "a%b",
        "é-",
    ] {
        for sort in [Sort::Name, Sort::Added, Sort::Modified, Sort::Size] {
            for descending in [false, true] {
                let q = Query {
                    text: text.into(),
                    sort,
                    descending,
                    ..Default::default()
                };
                let terms: Vec<_> = normalize(text)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let base = all(
                    &c,
                    &Query {
                        sort,
                        descending,
                        ..Default::default()
                    },
                );
                let expected: Vec<_> = base
                    .into_iter()
                    .filter(|i| {
                        let d = c.detail(i.id).unwrap();
                        let fields = [
                            normalize(&i.name),
                            normalize(&d.tags.join("\n")),
                            normalize(&d.memo),
                        ];
                        terms.iter().all(|t| fields.iter().any(|f| f.contains(t)))
                    })
                    .collect();
                assert_eq!(all(&c, &q), expected, "{text} {sort:?} {descending}");
                assert_eq!(c.count(&q).unwrap(), expected.len() as i64);
            }
        }
    }
    let q = Query {
        tags: vec!["タグ1".into()],
        rating: Some(1),
        extensions: vec!["PNG".into()],
        ..Default::default()
    };
    for i in all(&c, &q) {
        assert_eq!(i.rating, 1);
        assert_eq!(c.detail(i.id).unwrap().tags, vec!["タグ1"]);
    }
    c.check().unwrap();
}
#[cfg(unix)]
#[test]
fn scan_rename_missing_restore_cancel_and_originals() {
    let (root, _db, mut c) = setup();
    let original = root.path().join("before.png");
    std::fs::write(&original, b"original bytes").unwrap();
    scan(&mut c);
    let id = all(&c, &Query::default())[0].id;
    c.annotate(
        &[id],
        &Annotation {
            tags: vec!["素材".into()],
            rating: 5,
            memo: "keep".into(),
        },
    )
    .unwrap();
    let r = c.scan(&AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(r.unchanged, 1);
    assert_eq!(r.indexed, 0);
    std::fs::rename(&original, root.path().join("after.png")).unwrap();
    scan(&mut c);
    assert_eq!(all(&c, &Query::default())[0].id, id);
    assert_eq!(c.detail(id).unwrap().memo, "keep");
    assert_eq!(
        std::fs::read(root.path().join("after.png")).unwrap(),
        b"original bytes"
    );
    std::fs::remove_file(root.path().join("after.png")).unwrap();
    scan(&mut c);
    assert_eq!(c.count(&Query::default()).unwrap(), 0);
    assert!(c.detail(id).unwrap().item.missing);
    std::fs::write(root.path().join("after.png"), b"restored").unwrap();
    scan(&mut c);
    assert_eq!(c.detail(id).unwrap().memo, "keep");
    let r = c.scan(&AtomicBool::new(true), |_| {}).unwrap();
    assert!(r.cancelled);
    assert_eq!(c.count(&Query::default()).unwrap(), 1);
    c.check().unwrap();
}
#[test]
fn stale_cursor_atomic_annotations_backup_and_roots() {
    let (root, db, mut c) = setup();
    for i in 0..12 {
        std::fs::write(root.path().join(format!("{i}.jpg")), b"").unwrap();
    }
    scan(&mut c);
    assert!(c.add_root(root.path()).is_ok());
    let sub = root.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    assert!(c.add_root(&sub).is_err());
    let page = c.page(&Query::default(), 2, None).unwrap();
    let id = page.items[0].id;
    let annotation = Annotation {
        tags: vec!["ＡＢＣ".into(), "abc".into()],
        rating: 3,
        memo: "memo".into(),
    };
    assert!(c.annotate(&[id, 99999], &annotation).is_err());
    assert_eq!(c.detail(id).unwrap().memo, "");
    c.annotate(&[id], &annotation).unwrap();
    assert_eq!(c.detail(id).unwrap().tags.len(), 1);
    assert!(c.page(&Query::default(), 2, page.next.as_ref()).is_err());
    let backup = db.path().join("backup.db");
    c.backup(&backup).unwrap();
    let restored = Catalog::open(&backup).unwrap();
    assert_eq!(restored.detail(id).unwrap().memo, "memo");
    restored.check().unwrap();
    c.remove_root(c.roots().unwrap()[0].id).unwrap();
    assert_eq!(c.count(&Query::default()).unwrap(), 0);
    assert!(root.path().join("0.jpg").exists());
    c.check().unwrap();
}
#[test]
fn concurrent_reader_and_invalid_input() {
    let (root, db, mut c) = setup();
    std::fs::write(root.path().join("a.png"), b"").unwrap();
    scan(&mut c);
    let reader = Catalog::open(db.path().join("test.db")).unwrap();
    let thread = std::thread::spawn(move || {
        for _ in 0..50 {
            assert_eq!(reader.count(&Query::default()).unwrap(), 1);
        }
    });
    for n in 0..50 {
        c.annotate(
            &[1],
            &Annotation {
                tags: vec![],
                rating: 0,
                memo: n.to_string(),
            },
        )
        .unwrap();
    }
    thread.join().unwrap();
    assert!(c.page(&Query::default(), 0, None).is_err());
    assert!(c.page(&Query::default(), 501, None).is_err());
    assert!(
        c.count(&Query {
            rating: Some(6),
            ..Default::default()
        })
        .is_err()
    );
}
#[cfg(unix)]
#[test]
fn exclusions_hardlinks_inaccessible_root_and_symlink() {
    let (root, _db, mut c) = setup();
    std::fs::write(root.path().join("a.png"), b"a").unwrap();
    std::fs::hard_link(root.path().join("a.png"), root.path().join("b.png")).unwrap();
    std::fs::write(root.path().join(".hidden.png"), b"").unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    std::fs::create_dir(root.path().join("Test.app")).unwrap();
    std::fs::write(root.path().join("Test.app/hidden.png"), b"").unwrap();
    scan(&mut c);
    assert_eq!(c.count(&Query::default()).unwrap(), 2);
    let moved = root.path().with_extension("offline");
    std::fs::rename(root.path(), &moved).unwrap();
    let r = c.scan(&AtomicBool::new(false), |_| {}).unwrap();
    assert!(!r.errors.is_empty());
    assert_eq!(c.count(&Query::default()).unwrap(), 2);
    std::fs::rename(moved, root.path()).unwrap();
}

#[test]
fn seeded_random_unicode_search() {
    let (root, _db, mut c) = setup();
    let chars: Vec<char> = "abＣ猫日本語\"%_é😀東京123"
        .chars()
        .filter(|c| !cfg!(windows) || *c != '"')
        .collect();
    let mut seed = 0x19ac1201u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        seed
    };
    let mut names = vec![];
    for i in 0..180 {
        let text: String = (0..16)
            .map(|_| chars[next() as usize % chars.len()])
            .collect();
        let name = format!("{i}-{text}.png");
        std::fs::write(root.path().join(&name), b"").unwrap();
        names.push(name);
    }
    scan(&mut c);
    let base = all(&c, &Query::default());
    for _ in 0..180 {
        let name = &names[next() as usize % names.len()];
        let start = next() as usize % 10;
        let len = next() as usize % 5 + 1;
        let text: String = name.chars().skip(start).take(len).collect();
        let q = Query {
            text: text.clone(),
            ..Default::default()
        };
        let norm = normalize(&text);
        let expected: Vec<_> = base
            .iter()
            .filter(|i| normalize(&i.name).contains(&norm))
            .cloned()
            .collect();
        assert_eq!(all(&c, &q), expected, "{text:?}");
    }
}
#[test]
fn cancellation_during_index_rolls_back() {
    let (root, _db, mut c) = setup();
    for i in 0..1100 {
        std::fs::write(root.path().join(format!("{i}.png")), b"").unwrap();
    }
    let cancel = AtomicBool::new(false);
    let r = c
        .scan(&cancel, |p| {
            if p.indexed >= 1000 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed)
            }
        })
        .unwrap();
    assert!(r.cancelled);
    assert_eq!(r.indexed, 0);
    assert_eq!(c.count(&Query::default()).unwrap(), 0);
    scan(&mut c);
    assert_eq!(c.count(&Query::default()).unwrap(), 1100);
}
#[test]
fn scanning_added_root_does_not_visit_unavailable_existing_roots() {
    let (old_root, _db, mut catalog) = setup();
    std::fs::write(old_root.path().join("existing.jpg"), b"old photo").unwrap();
    scan(&mut catalog);
    // Simulate an unavailable NAS after its photos have been cataloged.
    std::fs::remove_file(old_root.path().join("existing.jpg")).unwrap();
    std::fs::remove_dir(old_root.path()).unwrap();
    let added = TempDir::new().unwrap();
    let nested = added.path().join("20260607ドライブ");
    std::fs::create_dir(&nested).unwrap();
    for number in 0..42 {
        std::fs::write(nested.join(format!("IMGP{number}.JPG")), b"new photo").unwrap();
    }
    let root = catalog.add_root(added.path()).unwrap();
    let report = catalog
        .scan_root(root.id, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.visited, 42);
    assert_eq!(report.indexed, 42);
    assert_eq!(report.missing, 0);
    assert_eq!(catalog.count(&Query::default()).unwrap(), 43);
    let report = catalog
        .scan_root(root.id, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert_eq!(report.unchanged, 42);
    assert!(
        catalog
            .scan_root(-1, &AtomicBool::new(false), |_| {})
            .is_err()
    );
}
#[test]
fn database_failure_rolls_back_annotations() {
    let (root, db, mut c) = setup();
    std::fs::write(root.path().join("a.png"), b"").unwrap();
    scan(&mut c);
    let raw = rusqlite::Connection::open(db.path().join("test.db")).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_annotation BEFORE INSERT ON item_tags BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
    assert!(
        c.annotate(
            &[1],
            &Annotation {
                tags: vec!["tag".into()],
                rating: 4,
                memo: "must rollback".into()
            }
        )
        .is_err()
    );
    assert_eq!(c.detail(1).unwrap().memo, "");
    assert_eq!(c.detail(1).unwrap().item.rating, 0);
    c.check().unwrap();
}
#[test]
fn crash_writer_child() {
    let Ok(db) = std::env::var("AE_CRASH_TEST_DB") else {
        return;
    };
    let ready = std::env::var("AE_CRASH_TEST_READY").unwrap();
    let raw = rusqlite::Connection::open(db).unwrap();
    raw.execute_batch("PRAGMA cache_size=1;BEGIN IMMEDIATE; UPDATE items SET memo='uncommitted',memo_norm='uncommitted';").unwrap();
    std::fs::write(ready, b"ready").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(30));
    panic!("parent should kill this child");
}
#[test]
fn forced_termination_recovers_committed_data() {
    let (root, db, mut c) = setup();
    std::fs::write(root.path().join("a.png"), b"original").unwrap();
    scan(&mut c);
    c.annotate(
        &[1],
        &Annotation {
            tags: vec!["keep".into()],
            rating: 5,
            memo: "committed".into(),
        },
    )
    .unwrap();
    drop(c);
    let ready = db.path().join("ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer_child", "--nocapture"])
        .env("AE_CRASH_TEST_DB", db.path().join("test.db"))
        .env("AE_CRASH_TEST_READY", &ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() {
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("child did not start transaction");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let c = Catalog::open(db.path().join("test.db")).unwrap();
    assert_eq!(c.detail(1).unwrap().memo, "committed");
    assert_eq!(c.detail(1).unwrap().tags, vec!["keep"]);
    c.check().unwrap();
    assert_eq!(
        std::fs::read(root.path().join("a.png")).unwrap(),
        b"original"
    );
}

#[test]
fn filters_index_updates_long_memo_and_validation() {
    let (root, _db, mut c) = setup();
    std::fs::write(root.path().join("original.png"), b"").unwrap();
    scan(&mut c);
    let i = all(&c, &Query::default()).remove(0);
    let q = Query {
        root_id: Some(i.root_id),
        added_from: Some(i.added),
        added_to: Some(i.added),
        ..Default::default()
    };
    assert_eq!(c.count(&q).unwrap(), 1);
    assert_eq!(
        c.count(&Query {
            root_id: Some(999),
            ..Default::default()
        })
        .unwrap(),
        0
    );
    c.annotate(
        &[i.id],
        &Annotation {
            tags: vec!["古いタグ".into()],
            rating: 2,
            memo: format!("{}最終メモ", "a".repeat(65536)),
        },
    )
    .unwrap();
    assert_eq!(
        c.count(&Query {
            text: "最終メモ".into(),
            ..Default::default()
        })
        .unwrap(),
        1
    );
    c.annotate(
        &[i.id],
        &Annotation {
            tags: vec!["新しいタグ".into()],
            rating: 4,
            memo: "updated".into(),
        },
    )
    .unwrap();
    assert_eq!(
        c.count(&Query {
            text: "最終メモ".into(),
            ..Default::default()
        })
        .unwrap(),
        0
    );
    assert_eq!(
        c.count(&Query {
            tags: vec!["古いタグ".into()],
            ..Default::default()
        })
        .unwrap(),
        0
    );
    assert!(
        c.annotate(
            &[i.id],
            &Annotation {
                tags: vec![],
                rating: 0,
                memo: "nul\0memo".into()
            }
        )
        .is_err()
    );
    assert!(
        c.count(&Query {
            text: "a\0bc".into(),
            ..Default::default()
        })
        .is_err()
    );
    std::fs::rename(
        root.path().join("original.png"),
        root.path().join("renamed.png"),
    )
    .unwrap();
    scan(&mut c);
    assert_eq!(
        c.count(&Query {
            text: "original".into(),
            ..Default::default()
        })
        .unwrap(),
        0
    );
    assert_eq!(
        c.count(&Query {
            text: "renamed".into(),
            ..Default::default()
        })
        .unwrap(),
        1
    );
    std::fs::remove_file(root.path().join("renamed.png")).unwrap();
    scan(&mut c);
    assert_eq!(
        c.count(&Query {
            include_missing: true,
            ..Default::default()
        })
        .unwrap(),
        if cfg!(unix) { 1 } else { 2 }
    );
    c.check().unwrap();
}
#[cfg(unix)]
#[test]
fn permission_errors_do_not_mark_items_missing() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _db, mut c) = setup();
    let dir = root.path().join("restricted");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a.png"), b"").unwrap();
    scan(&mut c);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    let report = c.scan(&AtomicBool::new(false), |_| {});
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let report = report.unwrap();
    assert!(!report.errors.is_empty(), "test requires unprivileged user");
    assert_eq!(c.count(&Query::default()).unwrap(), 1);
    assert_eq!(report.missing, 0);
}

#[test]
fn malformed_annotations_are_reported_not_silently_erased() {
    let (root, db, mut c) = setup();
    std::fs::write(root.path().join("a.png"), b"").unwrap();
    scan(&mut c);
    let raw = rusqlite::Connection::open(db.path().join("test.db")).unwrap();
    raw.execute("UPDATE items SET tags='invalid-json' WHERE id=1", [])
        .unwrap();
    assert!(c.detail(1).is_err());
}

#[test]
fn photo_formats_only_and_case_insensitive_extensions() {
    let (root, _db, mut c) = setup();
    for name in [
        "raw.DNG",
        "image.PNG",
        "photo.jpeg",
        "photo.JPG",
        "paper.pdf",
        "canon.CR2",
        "canon.CR3",
        "canon.CRW",
        "sony.ARW",
        "sony.SR2",
        "sony.SRF",
        "fuji.RAF",
        "camera.NEF",
        "camera.NRW",
        "camera.RW2",
        "camera.ORF",
        "camera.PEF",
        "camera.RWL",
        "camera.SRW",
        "camera.3FR",
        "camera.FFF",
        "camera.IIQ",
        "apple.HEIC",
        "camera.HIF",
        "image.HEIF",
        "new.GIF",
        "new.TIF",
        "new.TIFF",
        "new.BMP",
        "new.SVG",
        "new.AVIF",
        "new.PSD",
        "new.PSB",
        "new.HDR",
        "new.EXR",
        "old.webp",
        "old.mp4",
        "old.mov",
    ] {
        std::fs::write(root.path().join(name), b"fixture").unwrap();
    }
    scan(&mut c);
    assert_eq!(
        c.count(&Query::default()).unwrap(),
        advanced_explorer::SUPPORTED_EXTENSIONS.len() as i64
    );
    assert_eq!(
        c.count(&Query {
            extensions: vec!["dng".into()],
            ..Default::default()
        })
        .unwrap(),
        1
    );
}

#[test]
fn legacy_assets_keep_annotations_but_are_excluded_from_photo_queries() {
    let (root, db, mut catalog) = setup();
    let path = root.path().join("legacy.pdf");
    std::fs::write(&path, b"original document").unwrap();
    std::fs::write(root.path().join("photo.jpg"), b"original photo").unwrap();
    scan(&mut catalog);
    let id = all(&catalog, &Query::default())[0].id;
    catalog
        .annotate(
            &[id],
            &Annotation {
                tags: vec!["keep".into()],
                rating: 4,
                memo: "legacy note".into(),
            },
        )
        .unwrap();
    // Reproduce a pre-photo catalog record without needing an older executable.
    let raw = rusqlite::Connection::open(db.path().join("test.db")).unwrap();
    raw.execute(
        "UPDATE items SET extension='pdf',path='legacy.pdf' WHERE id=?",
        [id],
    )
    .unwrap();
    for query in [
        Query::default(),
        Query {
            include_missing: true,
            ..Default::default()
        },
        Query {
            extensions: vec!["pdf".into()],
            ..Default::default()
        },
    ] {
        assert_eq!(catalog.count(&query).unwrap(), 0);
        assert!(catalog.page(&query, 10, None).unwrap().items.is_empty());
    }
    assert!(catalog.source_path(id).is_err());
    scan(&mut catalog);
    let legacy = catalog.detail(id).unwrap();
    assert_eq!(legacy.memo, "legacy note");
    assert_eq!(legacy.tags, ["keep"]);
    assert_eq!(legacy.item.rating, 4);
    assert_eq!(catalog.count(&Query::default()).unwrap(), 1);
    assert_eq!(std::fs::read(path).unwrap(), b"original document");
    assert_eq!(
        std::fs::read(root.path().join("photo.jpg")).unwrap(),
        b"original photo"
    );
    catalog.check().unwrap();
}

#[test]
fn folder_tree_counts_and_filtering_respect_directory_boundaries() {
    let (root, _db, mut catalog) = setup();
    for name in [
        "root.jpg",
        "撮影/a.jpg",
        "撮影/選定/b.jpg",
        "撮影別/c.jpg",
        "撮影/notes.pdf",
    ] {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fixture").unwrap();
    }
    scan(&mut catalog);
    let root_id = catalog.roots().unwrap()[0].id;
    let folders = catalog.folders().unwrap();
    let counts: std::collections::BTreeMap<_, _> = folders
        .iter()
        .map(|folder| {
            assert_eq!(folder.root_id, root_id);
            (folder.path.clone(), folder.count)
        })
        .collect();
    assert_eq!(counts.get(""), Some(&4));
    assert_eq!(counts.get("撮影"), Some(&2));
    assert_eq!(
        counts.get(std::path::Path::new("撮影").join("選定").to_str().unwrap()),
        Some(&1)
    );
    let query = Query {
        root_id: Some(root_id),
        folder: Some("撮影".into()),
        ..Default::default()
    };
    assert_eq!(catalog.count(&query).unwrap(), 2);
    let first = catalog.page(&query, 1, None).unwrap();
    let second = catalog.page(&query, 1, first.next.as_ref()).unwrap();
    assert_ne!(first.items[0].id, second.items[0].id);
    assert!(second.next.is_none());
    assert_eq!(
        catalog
            .count(&Query {
                folder: Some(String::new()),
                ..query.clone()
            })
            .unwrap(),
        4
    );
    assert!(
        catalog
            .count(&Query {
                root_id: None,
                ..query.clone()
            })
            .is_err()
    );
    assert!(
        catalog
            .count(&Query {
                folder: Some("../outside".into()),
                ..query.clone()
            })
            .is_err()
    );
    assert!(
        catalog
            .count(&Query {
                folder: Some(root.path().to_str().unwrap().into()),
                ..query.clone()
            })
            .is_err()
    );
    assert!(
        catalog
            .page(
                &Query {
                    folder: Some("撮影別".into()),
                    ..query
                },
                1,
                first.next.as_ref()
            )
            .is_err()
    );
}

// The current non-Unix identity is a path; renames become new catalog items.
#[cfg(not(unix))]
#[test]
fn rename_with_path_identity_preserves_missing_record() {
    let (root, _db, mut c) = setup();
    std::fs::write(root.path().join("before.png"), b"source").unwrap();
    scan(&mut c);
    let old = all(&c, &Query::default())[0].id;
    std::fs::rename(
        root.path().join("before.png"),
        root.path().join("after.png"),
    )
    .unwrap();
    scan(&mut c);
    assert!(c.detail(old).unwrap().item.missing);
    assert_ne!(all(&c, &Query::default())[0].id, old);
    assert_eq!(c.count(&Query::default()).unwrap(), 1);
    c.check().unwrap();
}

#[test]
fn exact_extension_filter_migration_and_annotation_limits() {
    let (root, db, mut catalog) = setup();
    for ext in ["jpg", "jpeg", "psd", "psb", "tif", "tiff", "cr2", "cr3"] {
        std::fs::write(root.path().join(format!("photo.{ext}")), b"original").unwrap();
    }
    scan(&mut catalog);
    for ext in ["jpg", "jpeg", "psd", "psb", "tif", "tiff", "cr2", "cr3"] {
        let q = Query {
            extensions: vec![format!(".{}", ext.to_uppercase())],
            ..Default::default()
        };
        let page = catalog.page(&q, 10, None).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].extension, ext);
        let id = page.items[0].id;
        let annotation = Annotation {
            tags: vec!["作品".into(), "作品".into()],
            rating: 5,
            memo: "保存確認".into(),
        };
        catalog.annotate(&[id], &annotation).unwrap();
        assert_eq!(catalog.detail(id).unwrap().tags, vec!["作品"]);
        assert!(
            catalog
                .annotate(
                    &[id],
                    &Annotation {
                        memo: "a".repeat(1024 * 1024 + 1),
                        ..annotation
                    }
                )
                .is_err()
        );
        assert_eq!(catalog.detail(id).unwrap().memo, "保存確認");
    }
    let path = db.path().join("library.db");
    // Simulate a pre-migration catalog and verify existing annotations survive opening.
    catalog.backup(&path).unwrap();
    drop(catalog);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP INDEX item_extension_name; DROP TABLE saved_recipes; DROP TABLE photo_metadata; DROP INDEX items_flag_rating; DROP INDEX items_label; ALTER TABLE items DROP COLUMN flag; ALTER TABLE items DROP COLUMN label; PRAGMA user_version=1;",
    )
    .unwrap();
    drop(conn);
    let upgraded = Catalog::open(&path).unwrap();
    let items = all(&upgraded, &Query::default());
    assert_eq!(items.len(), 8);
    assert!(items.iter().all(|i| i.rating == 5));
    upgraded.check().unwrap();
}
