//! Compare release builds using the same durable catalog and batch sizes.
use advanced_explorer::{Annotation, Catalog, Query};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{sync::atomic::AtomicBool, time::Instant};

fn measure(mut update: impl FnMut(usize) -> Result<()>) -> Result<serde_json::Value> {
    let mut times = Vec::new();
    for run in 0..22 {
        let start = Instant::now();
        update(run)?;
        if run > 0 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    Ok(json!({"median_ms": times[10], "p95_ms": times[19], "samples": times.len()}))
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let photos = directory.path().join("photos");
    std::fs::create_dir(&photos)?;
    for index in 0..10000 {
        std::fs::write(photos.join(format!("photo-{index:05}.jpg")), b"fixture")?;
    }
    let mut catalog = Catalog::open(directory.path().join("catalog.db"))?;
    catalog.add_root(&photos)?;
    catalog.scan(&AtomicBool::new(false), |_| {})?;
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = catalog.page(&Query::default(), 500, cursor.as_ref())?;
        ids.extend(page.items.into_iter().map(|item| item.id));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    ensure!(ids.len() == 10000, "fixture count changed");
    let attributes =
        measure(|run| catalog.set_attributes(&ids, Some((run % 6) as u8), Some(1), Some("red")))?;
    ensure!(
        catalog.count(&Query {
            rating: Some(3),
            ..Default::default()
        })? == 10000,
        "attribute update missed photos"
    );
    let annotations = measure(|run| {
        catalog.annotate(
            &ids[..500],
            &Annotation {
                tags: (0..10).map(|tag| format!("tag-{run}-{tag}")).collect(),
                rating: 4,
                memo: format!("Batch annotation {run}"),
            },
        )
    })?;
    ensure!(
        catalog.count(&Query {
            text: "annotation 21".into(),
            ..Default::default()
        })? == 500,
        "annotation search index changed"
    );
    catalog.check()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "synthetic": true, "attributes_10000": attributes, "annotations_500_with_10_tags": annotations
        }))?
    );
    Ok(())
}
