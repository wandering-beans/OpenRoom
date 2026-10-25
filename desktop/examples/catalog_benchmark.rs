//! Synthetic catalog footprint and save latency. Run in release mode.
use advanced_explorer::{Catalog, Query};
use advanced_explorer_develop::Settings;
use anyhow::Result;
use serde_json::json;
use std::{sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let photos = directory.path().join("photos");
    std::fs::create_dir(&photos)?;
    for index in 0..5000 {
        std::fs::write(
            photos.join(format!("photo-{index:05}.jpg")),
            b"synthetic reference",
        )?;
    }
    let base = directory.path().join("base.db");
    let mut catalog = Catalog::open(&base)?;
    catalog.add_root(&photos)?;
    catalog.scan(&AtomicBool::new(false), |_| {})?;
    drop(catalog);
    let base_bytes = std::fs::metadata(&base)?.len();
    let mut results = Vec::new();
    for compact in [false, true] {
        let path = directory
            .path()
            .join(if compact { "compact.db" } else { "full.db" });
        std::fs::copy(&base, &path)?;
        let mut catalog = Catalog::open(&path)?;
        let settings = Settings {
            exposure: 1.25,
            ..Default::default()
        };
        let recipe = if compact {
            settings.catalog_recipe()?
        } else {
            serde_json::to_string(&settings)?
        };
        let mut ids = Vec::new();
        let mut cursor = None;
        loop {
            let page = catalog.page(&Query::default(), 200, cursor.as_ref())?;
            ids.extend(page.items.into_iter().map(|item| item.id));
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let start = Instant::now();
        for id in &ids {
            catalog.save_develop_recipe(*id, &recipe, 0)?;
        }
        let initial_save_ms = start.elapsed().as_secs_f64() * 1000.;
        drop(catalog);
        let bytes = std::fs::metadata(&path)?.len();
        // Match desktop behavior: connection, durable commit, close for each saved edit.
        let mut latencies = Vec::new();
        for id in ids.iter().take(200) {
            let start = Instant::now();
            Catalog::open(&path)?.save_develop_recipe(*id, &recipe, 1)?;
            latencies.push(start.elapsed().as_secs_f64() * 1000.);
        }
        latencies.sort_by(f64::total_cmp);
        let catalog = Catalog::open(&path)?;
        let start = Instant::now();
        let snapshot = directory.path().join(if compact {
            "compact.openroom"
        } else {
            "full.openroom"
        });
        catalog.backup(&snapshot)?;
        let snapshot_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        for id in &ids {
            let (stored, _) = catalog.develop_recipe(*id)?;
            let restored: Settings = serde_json::from_str(&stored.unwrap())?;
            anyhow::ensure!(restored == settings, "recipe changed");
        }
        results.push(json!({
            "encoding": if compact {"default_omitting_json"} else {"full_json"},
            "photo_count": ids.len(), "recipe_bytes": recipe.len(),
            "base_catalog_bytes": base_bytes, "catalog_bytes": bytes,
            "edit_storage_growth_bytes": bytes - base_bytes,
            "initial_save_5000_ms": initial_save_ms,
            "desktop_save_median_ms": latencies[100], "desktop_save_p95_ms": latencies[189],
            "snapshot_bytes": std::fs::metadata(snapshot)?.len(), "snapshot_ms": snapshot_ms,
            "read_and_decode_5000_ms": start.elapsed().as_secs_f64()*1000.,
            "exact_roundtrips": ids.len()
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"synthetic":true,"results":results}))?
    );
    Ok(())
}
