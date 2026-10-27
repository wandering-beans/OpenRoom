use crate::{AppState, Catalog, Query, Reply, media, work};
use advanced_explorer::library::Facet;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::State;

#[derive(Serialize)]
pub struct LibraryPage {
    #[serde(flatten)]
    page: advanced_explorer::Page,
    adjusted_ids: Vec<i64>,
}

pub fn photo_page(
    catalog: &Catalog,
    query: &Query,
    cursor: Option<&advanced_explorer::Cursor>,
) -> anyhow::Result<LibraryPage> {
    let page = catalog.page(query, 200, cursor)?;
    let mut adjusted_ids = Vec::new();
    let defaults = advanced_explorer_develop::Settings::default();
    for item in &page.items {
        if let Some(recipe) = catalog.develop_recipe(item.id)?.0 {
            // Keep photos with unreadable recipes visible and marked as having saved edits.
            let adjusted = serde_json::from_str::<advanced_explorer_develop::Settings>(&recipe)
                .map_or(true, |settings| settings != defaults);
            if adjusted {
                adjusted_ids.push(item.id);
            }
        }
    }
    Ok(LibraryPage { page, adjusted_ids })
}

#[derive(Default)]
pub struct IndexState {
    busy: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}
struct IndexGuard(Arc<AtomicBool>);
impl Drop for IndexGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn remove_from_catalog(
    state: State<'_, AppState>,
    develop: State<'_, crate::develop::DevelopState>,
    selection: advanced_explorer::removal::Removal,
) -> Reply<usize> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("読み込みを中止するか、完了してからカタログから削除してください。".into());
    }
    let _scan = IndexGuard(state.scanning.clone());
    let _foreground = state.library.updates.foreground();
    state.preview_version.fetch_add(1, Ordering::SeqCst);
    state.metadata_version.fetch_add(1, Ordering::SeqCst);
    develop.discard_previews().await;
    let _metadata = state.metadata_jobs.acquire().await.map_err(|error| error.to_string())?;
    let copied = (*state).clone();
    work(move || copied.library.remove_catalog_items(&copied, &selection)).await
}
#[derive(Serialize)]
pub struct IndexProgress {
    processed: usize,
    failed: usize,
    remaining: bool,
}
#[tauri::command]
pub async fn set_photo_attributes(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    rating: Option<u8>,
    flag: Option<i8>,
    label: Option<String>,
) -> Reply<()> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.set_attributes(&ids, rating, flag, label.as_deref())).await
}
#[tauri::command]
pub async fn metadata_facets(
    state: State<'_, AppState>,
    query: Query,
    field: String,
) -> Reply<Vec<Facet>> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.metadata_facets(&query, &field)).await
}
#[tauri::command]
pub fn cancel_metadata_index(index: State<'_, IndexState>) {
    index.cancelled.store(true, Ordering::SeqCst);
}
#[tauri::command]
pub async fn index_metadata_batch(
    state: State<'_, AppState>,
    index: State<'_, IndexState>,
    query: Query,
) -> Reply<IndexProgress> {
    if index.busy.swap(true, Ordering::SeqCst) {
        return Err("メタデータを読み込み中です".into());
    }
    let guard = IndexGuard(index.busy.clone());
    index.cancelled.store(false, Ordering::SeqCst);
    let cancelled = index.cancelled.clone();
    let db = state.db.clone();
    work(move || {
        let _guard = guard;
        let catalog = Catalog::open(db)?;
        let candidates = catalog.metadata_candidates(&query, 16)?;
        let mut processed = 0;
        let mut failed = 0;
        for item in candidates {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            let result =
                (|| -> anyhow::Result<advanced_explorer_media::metadata::PhotoMetadata> {
                    let source = catalog.source_path(item.id)?;
                    media::read_metadata(&source, &|| cancelled.load(Ordering::SeqCst))
                })();
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            match result {
                Ok(fields) => catalog.index_metadata(&item, &fields, None)?,
                Err(error) => {
                    failed += 1;
                    catalog.index_metadata(&item, &Default::default(), Some(&error.to_string()))?;
                }
            }
            processed += 1;
        }
        Ok(IndexProgress {
            processed,
            failed,
            remaining: !catalog.metadata_candidates(&query, 1)?.is_empty(),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use advanced_explorer_develop::Settings;

    #[test]
    fn adjustment_badges_follow_saved_settings_and_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let photos = directory.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        for name in ["a.jpg", "b.jpg"] {
            std::fs::write(photos.join(name), b"catalog-only fixture").unwrap();
        }
        let db = directory.path().join("catalog.db");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let initial = photo_page(&catalog, &Query::default(), None).unwrap();
        assert!(initial.adjusted_ids.is_empty());
        let ids: Vec<_> = initial.page.items.iter().map(|item| item.id).collect();
        let defaults = Settings::default();
        // Older catalogs stored complete settings, including neutral values.
        catalog
            .save_develop_recipe(ids[0], &serde_json::to_string(&defaults).unwrap(), 0)
            .unwrap();
        let rotated = Settings {
            quarter_turns: 1,
            ..defaults.clone()
        };
        catalog
            .save_develop_recipe(ids[1], &rotated.catalog_recipe().unwrap(), 0)
            .unwrap();
        drop(catalog);
        let mut catalog = Catalog::open(&db).unwrap();
        let page = photo_page(&catalog, &Query::default(), None).unwrap();
        assert_eq!(page.adjusted_ids, vec![ids[1]]);
        let json = serde_json::to_value(&page).unwrap();
        assert!(json["items"].is_array());
        assert_eq!(json["adjusted_ids"], serde_json::json!([ids[1]]));
        catalog
            .save_develop_recipe(ids[1], &defaults.catalog_recipe().unwrap(), 1)
            .unwrap();
        let reset = photo_page(&catalog, &Query::default(), None).unwrap();
        assert!(reset.adjusted_ids.is_empty());
        assert_eq!(reset.page.items, initial.page.items);
        catalog.check().unwrap();
    }
}
