use advanced_explorer::*;
use anyhow::{Result, ensure};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
fn measure(mut f: impl FnMut() -> Result<()>, runs: usize) -> Result<serde_json::Value> {
    for _ in 0..10 {
        f()?;
    }
    let mut times = vec![];
    for _ in 0..runs {
        let start = Instant::now();
        f()?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    Ok(
        json!({"median_ms":times[times.len()/2],"p95_ms":times[(times.len()*95).div_ceil(100)-1],"max_ms":times[times.len()-1],"runs":runs}),
    )
}
fn main() -> Result<()> {
    let n: usize = std::env::args().nth(1).unwrap_or("30000".into()).parse()?;
    let runs: usize = std::env::args().nth(2).unwrap_or("100".into()).parse()?;
    ensure!(
        n >= 100 && runs >= 100,
        "at least 100 items and repetitions required"
    );
    let data = tempfile::tempdir()?;
    let storage = tempfile::tempdir()?;
    let db = storage.path().join("bench.db");
    for bucket in 0..100 {
        std::fs::create_dir(data.path().join(format!("{bucket:03}")))?;
    }
    let start = Instant::now();
    for i in 0..n {
        let name = match i % 5 {
            0 => "東京写真",
            1 => "日本風景",
            2 => "design",
            3 => "ＡＢＣ資料",
            _ => "猫画像",
        };
        std::fs::write(
            data.path()
                .join(format!("{:03}/{name}-{i:07}.png", i % 100)),
            (i as u64).to_le_bytes(),
        )?;
    }
    let fixture_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut c = Catalog::open(&db)?;
    c.add_root(data.path())?;
    let start = Instant::now();
    let initial = c.scan(&AtomicBool::new(false), |_| {})?;
    let initial_ms = start.elapsed().as_secs_f64() * 1000.0;
    ensure!(initial.errors.is_empty(), "scan errors");
    ensure!(
        c.count(&Query::default())? == n as i64,
        "index count mismatch"
    );
    let ids: Vec<_> = {
        let mut all = vec![];
        let mut cursor = None;
        loop {
            let p = c.page(&Query::default(), 500, cursor.as_ref())?;
            all.extend(p.items.into_iter().map(|i| i.id));
            cursor = p.next;
            if cursor.is_none() {
                break;
            }
        }
        all
    };
    c.annotate(
        &ids.iter().step_by(10).copied().collect::<Vec<_>>(),
        &Annotation {
            tags: vec!["参考".into(), "日本素材".into()],
            rating: 4,
            memo: "デザイン参考資料 sample memo".into(),
        },
    )?;
    let start = Instant::now();
    let unchanged = c.scan(&AtomicBool::new(false), |_| {})?;
    let unchanged_ms = start.elapsed().as_secs_f64() * 1000.0;
    ensure!(
        unchanged.unchanged == n && unchanged.indexed == 0,
        "idempotency failed"
    );
    let queries = vec![
        ("list", Query::default(), 50.0),
        (
            "list_size",
            Query {
                sort: Sort::Size,
                ..Default::default()
            },
            50.0,
        ),
        (
            "list_added",
            Query {
                sort: Sort::Added,
                ..Default::default()
            },
            50.0,
        ),
        (
            "list_modified_desc",
            Query {
                sort: Sort::Modified,
                descending: true,
                ..Default::default()
            },
            50.0,
        ),
        (
            "short_no_match",
            Query {
                text: "龍".into(),
                ..Default::default()
            },
            200.0,
        ),
        (
            "no_match",
            Query {
                text: "見つからない".into(),
                ..Default::default()
            },
            100.0,
        ),
        (
            "rare",
            Query {
                text: "0000012".into(),
                ..Default::default()
            },
            100.0,
        ),
        (
            "common",
            Query {
                text: "png".into(),
                ..Default::default()
            },
            100.0,
        ),
        (
            "japanese_long",
            Query {
                text: "東京写真".into(),
                ..Default::default()
            },
            100.0,
        ),
        (
            "one_char",
            Query {
                text: "猫".into(),
                ..Default::default()
            },
            200.0,
        ),
        (
            "two_chars",
            Query {
                text: "日本".into(),
                ..Default::default()
            },
            200.0,
        ),
        (
            "combined",
            Query {
                text: "日本".into(),
                rating: Some(4),
                tags: vec!["参考".into()],
                extensions: vec!["png".into()],
                ..Default::default()
            },
            200.0,
        ),
        (
            "memo",
            Query {
                text: "sample".into(),
                ..Default::default()
            },
            100.0,
        ),
    ];
    let mut results = serde_json::Map::new();
    let mut passed = true;
    drop(c);
    let mut c = Catalog::open(&db)?;
    let first = Instant::now();
    c.page(&Query::default(), 100, None)?;
    let first_page_ms = first.elapsed().as_secs_f64() * 1000.0;
    for (label, q, budget) in &queries {
        let mut value = measure(
            || {
                std::hint::black_box(c.page(q, 100, None)?);
                Ok(())
            },
            runs,
        )?;
        let ok = value["p95_ms"].as_f64().unwrap() <= *budget;
        passed &= ok;
        value["target_ms"] = json!(budget);
        value["passed"] = json!(ok);
        value["count"] = json!(c.count(q)?);
        results.insert(label.to_string(), value);
    }
    let detail = measure(
        || {
            std::hint::black_box(c.detail(ids[0])?);
            Ok(())
        },
        runs,
    )?;
    passed &= detail["p95_ms"].as_f64().unwrap() <= 10.0;
    results.insert("detail".into(), detail);
    let count = measure(
        || {
            std::hint::black_box(c.count(&Query {
                text: "日本".into(),
                ..Default::default()
            })?);
            Ok(())
        },
        runs,
    )?;
    passed &= count["p95_ms"].as_f64().unwrap() <= 200.0;
    results.insert("count".into(), count);
    let mut cursor = None;
    for _ in 0..(n / 100 - 2) {
        cursor = c.page(&Query::default(), 100, cursor.as_ref())?.next;
    }
    results.insert(
        "deep_page".into(),
        measure(
            || {
                std::hint::black_box(c.page(&Query::default(), 100, cursor.as_ref())?);
                Ok(())
            },
            runs,
        )?,
    );
    for i in 0..n / 100 {
        let name = match i % 5 {
            0 => "東京写真",
            1 => "日本風景",
            2 => "design",
            3 => "ＡＢＣ資料",
            _ => "猫画像",
        };
        std::fs::write(
            data.path()
                .join(format!("{:03}/{name}-{i:07}.png", i % 100)),
            b"updated fixture contents",
        )?;
    }
    let reader = Catalog::open(&db)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();
    let thread = std::thread::spawn(move || -> Result<Vec<f64>> {
        let mut samples = vec![];
        while !stop_reader.load(Ordering::Relaxed) {
            let start = Instant::now();
            reader.page(
                &Query {
                    text: "日本".into(),
                    ..Default::default()
                },
                100,
                None,
            )?;
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(samples)
    });
    let start = Instant::now();
    let changed = c.scan(&AtomicBool::new(false), |_| {})?;
    let changed_ms = start.elapsed().as_secs_f64() * 1000.0;
    stop.store(true, Ordering::Relaxed);
    let mut concurrent = thread.join().unwrap()?;
    concurrent.sort_by(f64::total_cmp);
    ensure!(changed.indexed == n / 100, "incremental count mismatch");
    c.check()?;
    let raw = rusqlite::Connection::open(&db)?;
    let plan:Vec<String>=raw.prepare("EXPLAIN QUERY PLAN SELECT id FROM items WHERE missing=0 ORDER BY name_norm,id LIMIT 100")?.query_map([],|r|r.get(3))?.collect::<rusqlite::Result<_>>()?;
    raw.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let report = json!({"items":n,"runs":runs,"fixture_seed":"deterministic-v1","fixture_creation_ms":fixture_ms,"initial_scan_ms":initial_ms,"unchanged_scan_ms":unchanged_ms,"one_percent_update_ms":changed_ms,"first_page_after_reopen_ms":first_page_ms,"database_bytes":std::fs::metadata(&db)?.len(),"warm":results,"concurrent_search":{"samples":concurrent.len(),"p95_ms":concurrent.get((concurrent.len()*95/100).min(concurrent.len().saturating_sub(1))),"max_ms":concurrent.last()},"list_query_plan":plan,"performance_targets_passed":passed,"note":"metadata-only synthetic files; first read is not OS cold-cache; RSS measured with /usr/bin/time -l"});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if n == 30000 {
        ensure!(passed, "30k performance target failed");
    }
    Ok(())
}
