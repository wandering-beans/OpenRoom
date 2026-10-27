//! Operations shared with the desktop's catalog, renderer, and export implementation.
use super::jobs::Jobs;
use crate::{AppState, Catalog};
use advanced_explorer::{AnnotationPatch, Cursor, HistoryEdit, Query};
use advanced_explorer_develop::{ExportOptions, Settings};
use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Operations {
    pub state: Arc<AppState>,
    pub jobs: Arc<Jobs>,
    pub app: Option<tauri::AppHandle>,
    pub stop: CancellationToken,
    pub reads: Arc<tokio::sync::Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Photo {
    id: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    query: Query,
    #[serde(default = "page_size")]
    limit: usize,
    cursor: Option<Cursor>,
}
fn page_size() -> usize {
    200
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryInput {
    #[serde(default)]
    query: Query,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attributes {
    ids: Vec<i64>,
    rating: Option<u8>,
    flag: Option<i8>,
    label: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotations {
    ids: Vec<i64>,
    patch: AnnotationPatch,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    id: i64,
    expected_revision: i64,
    patch: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewInput {
    id: i64,
    patch: Option<Value>,
    #[serde(default = "preview_size")]
    bound: u32,
}
fn preview_size() -> u32 {
    1600
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryInput {
    id: i64,
    before: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    id: i64,
    history_id: i64,
    expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetInput {
    name: String,
    patch: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    id: i64,
    expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPreset {
    preset_id: i64,
    targets: Vec<Target>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportInput {
    ids: Vec<i64>,
    destination: PathBuf,
    format: String,
    #[serde(default)]
    options: ExportOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootInput {
    path: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanInput {
    root_id: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Facets {
    #[serde(default)]
    query: Query,
    field: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobInput {
    job_id: String,
}

pub fn validate_ids(ids: &[i64], max: usize) -> Result<()> {
    let unique: HashSet<_> = ids.iter().collect();
    ensure!(
        !ids.is_empty()
            && ids.len() <= max
            && ids.iter().all(|id| *id > 0)
            && unique.len() == ids.len(),
        "Invalid or duplicate photo IDs (maximum {max})"
    );
    Ok(())
}
pub fn settings(catalog: &Catalog, id: i64) -> Result<(Settings, i64)> {
    let (recipe, revision) = catalog.develop_recipe(id)?;
    let settings: Settings = serde_json::from_str(recipe.as_deref().unwrap_or("{}"))?;
    settings.validate()?;
    Ok((settings, revision))
}
pub fn patched(settings: &Settings, patch: &Value) -> Result<Settings> {
    let fields = patch.as_object().context("patch must be an object")?;
    ensure!(
        !fields.is_empty() && !fields.contains_key("version"),
        "Provide settings to change; version cannot be changed"
    );
    let mut value = serde_json::to_value(settings)?;
    for (key, entry) in fields {
        ensure!(value.get(key).is_some(), "Unknown setting: {key}");
        value[key] = entry.clone();
    }
    let updated: Settings = serde_json::from_value(value)?;
    updated.validate()?;
    Ok(updated)
}
pub fn is_ui_operation(name: &str) -> bool {
    matches!(name, "get_app_state" | "select_photos" | "set_view") || is_mutation(name)
}
pub fn is_mutation(name: &str) -> bool {
    matches!(
        name,
        "add_root"
            | "start_scan"
            | "start_metadata_index"
            | "update_photo_attributes"
            | "update_annotations"
            | "update_develop_settings"
            | "save_preset"
            | "apply_preset"
            | "restore_develop_history"
            | "start_export"
    )
}
impl Operations {
    pub async fn execute(&self, name: String, arguments: Value) -> Result<Value> {
        ensure!(!self.stop.is_cancelled(), "MCP server is stopped");
        let _permit = if matches!(name.as_str(), "preview_photo" | "get_metadata") {
            Some(self.reads.acquire().await?)
        } else {
            None
        };
        let operations = self.clone();
        if matches!(
            name.as_str(),
            "start_scan" | "start_metadata_index" | "start_export"
        ) {
            return self.start_job(&name, arguments).await;
        }
        tokio::task::spawn_blocking(move || operations.execute_blocking(&name, arguments)).await?
    }
    fn execute_blocking(&self, name: &str, args: Value) -> Result<Value> {
        ensure!(!self.stop.is_cancelled(), "MCP server is stopped");
        let mut catalog = Catalog::open(&self.state.db)?;
        match name {
            "list_roots" => Ok(json!({"roots":catalog.roots()?})),
            "list_folders" => Ok(json!({"folders":catalog.folders()?})),
            "search_photos" => {
                let input: Search = serde_json::from_value(args)?;
                ensure!((1..=200).contains(&input.limit), "limit must be 1..200");
                Ok(serde_json::to_value(catalog.page(
                    &input.query,
                    input.limit,
                    input.cursor.as_ref(),
                )?)?)
            }
            "count_photos" => {
                let input: QueryInput = serde_json::from_value(args)?;
                Ok(json!({"count":catalog.count(&input.query)?}))
            }
            "get_photo" => {
                let input: Photo = serde_json::from_value(args)?;
                Ok(serde_json::to_value(catalog.detail(input.id)?)?)
            }
            "get_metadata" => {
                let input: Photo = serde_json::from_value(args)?;
                let source = catalog.source_path(input.id)?;
                let fields = crate::media::read_metadata(&source, &|| self.stop.is_cancelled())?;
                Ok(json!({"id":input.id,"metadata":fields}))
            }
            "get_metadata_facets" => {
                let input: Facets = serde_json::from_value(args)?;
                Ok(
                    json!({"facets":catalog.metadata_facets(&input.query, &input.field)?,"indexing_required":!catalog.metadata_candidates(&input.query,1)?.is_empty()}),
                )
            }
            "add_root" => {
                let input: RootInput = serde_json::from_value(args)?;
                ensure!(input.path.is_absolute(), "Use an absolute folder path");
                Ok(serde_json::to_value(catalog.add_root(input.path)?)?)
            }
            "update_photo_attributes" => {
                let input: Attributes = serde_json::from_value(args)?;
                validate_ids(&input.ids, 10000)?;
                ensure!(
                    input.rating.is_some() || input.flag.is_some() || input.label.is_some(),
                    "Provide attributes to change"
                );
                catalog.set_attributes(
                    &input.ids,
                    input.rating,
                    input.flag,
                    input.label.as_deref(),
                )?;
                Ok(json!({"updated_ids":input.ids}))
            }
            "update_annotations" => {
                let input: Annotations = serde_json::from_value(args)?;
                validate_ids(&input.ids, 500)?;
                ensure!(
                    input.patch.tags.is_some()
                        || input.patch.rating.is_some()
                        || input.patch.memo.is_some(),
                    "Provide annotation fields to change"
                );
                catalog.patch_annotations(&input.ids, &input.patch)?;
                Ok(json!({"updated_ids":input.ids}))
            }
            "get_develop_settings" => {
                let input: Photo = serde_json::from_value(args)?;
                let (settings, revision) = settings(&catalog, input.id)?;
                Ok(
                    json!({"id":input.id,"settings":settings,"revision":revision,"defaults":Settings::default()}),
                )
            }
            "update_develop_settings" => {
                let input: Edit = serde_json::from_value(args)?;
                let (current, _) = settings(&catalog, input.id)?;
                let updated = patched(&current, &input.patch)?;
                let revision = catalog.save_develop_steps(
                    input.id,
                    &[HistoryEdit {
                        recipe: updated.catalog_recipe()?,
                        label: "AIによる現像".into(),
                    }],
                    input.expected_revision,
                )?;
                Ok(json!({"updated_ids":[input.id],"revision":revision,"settings":updated}))
            }
            "list_develop_history" => {
                let input: HistoryInput = serde_json::from_value(args)?;
                catalog.detail(input.id)?;
                Ok(json!({"history":catalog.develop_history(input.id,input.before)?}))
            }
            "restore_develop_history" => {
                let input: Restore = serde_json::from_value(args)?;
                let restored: Settings =
                    serde_json::from_str(&catalog.history_recipe(input.id, input.history_id)?)?;
                let revision = catalog.save_develop_steps(
                    input.id,
                    &[HistoryEdit {
                        recipe: restored.catalog_recipe()?,
                        label: "AIによる履歴の復元".into(),
                    }],
                    input.expected_revision,
                )?;
                Ok(json!({"updated_ids":[input.id],"revision":revision,"settings":restored}))
            }
            "list_presets" => Ok(json!({"presets":catalog.saved_recipes(None)?})),
            "save_preset" => {
                let input: PresetInput = serde_json::from_value(args)?;
                patched(&Settings::default(), &input.patch)?;
                Ok(
                    json!({"preset_id":catalog.store_recipe(None,&input.name,&input.patch.to_string())?}),
                )
            }
            "apply_preset" => {
                let input: ApplyPreset = serde_json::from_value(args)?;
                let ids: Vec<_> = input.targets.iter().map(|target| target.id).collect();
                validate_ids(&ids, 500)?;
                let preset = catalog
                    .saved_recipes(None)?
                    .into_iter()
                    .find(|preset| preset.id == input.preset_id)
                    .context("Preset not found")?;
                let patch: Value = serde_json::from_str(&preset.recipe)?;
                let edits = input
                    .targets
                    .iter()
                    .map(|target| -> Result<_> {
                        let (current, revision) = settings(&catalog, target.id)?;
                        ensure!(
                            revision == target.expected_revision,
                            "Revision conflict for photo {}",
                            target.id
                        );
                        let updated = patched(&current, &patch)?;
                        Ok((
                            target.id,
                            HistoryEdit {
                                recipe: updated.catalog_recipe()?,
                                label: format!("AIプリセット #{}", preset.id),
                            },
                            target.expected_revision,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                catalog.save_develop_batch(&edits)?;
                Ok(json!({"updated_ids":ids}))
            }
            "preview_photo" => {
                let input: PreviewInput = serde_json::from_value(args)?;
                ensure!(
                    [384, 1600].contains(&input.bound),
                    "bound must be 384 or 1600"
                );
                let (mut settings, revision) = settings(&catalog, input.id)?;
                if let Some(patch) = input.patch {
                    settings = patched(&settings, &patch)?;
                }
                let source = catalog.source_path(input.id)?;
                let temporary = tempfile::tempdir_in(&self.state.cache)?;
                let image = temporary.path().join("preview.jpg");
                let recipe = temporary.path().join("recipe.json");
                std::fs::write(&recipe, serde_json::to_vec(&settings)?)?;
                let info =
                    crate::develop::agent_preview(&source, &image, &recipe, input.bound, &|| {
                        self.stop.is_cancelled()
                    })?;
                let bytes = std::fs::read(image)?;
                Ok(
                    json!({"id":input.id,"revision":revision,"info":info,"image_base64":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                )
            }
            "get_job" => {
                let input: JobInput = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.jobs.get(&input.job_id)?)?)
            }
            "cancel_job" => {
                let input: JobInput = serde_json::from_value(args)?;
                let job = self.jobs.cancel(&input.job_id)?;
                if job.kind == "scan" && job.status == "running" {
                    self.state.cancel.store(true, Ordering::SeqCst);
                }
                Ok(serde_json::to_value(job)?)
            }
            _ => bail!("Unknown operation: {name}"),
        }
    }
    async fn start_job(&self, name: &str, args: Value) -> Result<Value> {
        let operations = self.clone();
        let job = match name {
            "start_scan" => {
                let input: ScanInput = serde_json::from_value(args)?;
                ensure!(
                    !self.state.scanning.swap(true, Ordering::SeqCst),
                    "A folder scan is already running"
                );
                let scan_busy = self.state.scanning.clone();
                self.state.cancel.store(false, Ordering::SeqCst);
                let result = self
                    .jobs
                    .start("scan", self.app.clone(), move |cancel, id| {
                        let _guard = BusyGuard(operations.state.scanning.clone());
                        let mut catalog = Catalog::open(&operations.state.db)?;
                        // GUI cancellation and server shutdown stop this scan as well.
                        let shared_cancel = operations.state.cancel.clone();
                        let progress = |progress: &advanced_explorer::ScanReport| {
                            if cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled() {
                                shared_cancel.store(true, Ordering::SeqCst);
                            }
                            operations
                                .jobs
                                .progress(&id, json!(progress), operations.app.as_ref());
                        };
                        let report = match input.root_id {
                            Some(root_id) => {
                                catalog.scan_root(root_id, &shared_cancel, progress)?
                            }
                            None => catalog.scan(&shared_cancel, progress)?,
                        };
                        let previews = if report.cancelled {
                            crate::library_previews::Progress::default()
                        } else {
                            crate::library_previews::prepare_import(&operations.state, input.root_id, |preview| {
                                if cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled() {
                                    shared_cancel.store(true, Ordering::SeqCst);
                                }
                                operations.jobs.progress(&id, json!({"previews": preview}), operations.app.as_ref());
                            })?
                        };
                        if report.cancelled || previews.cancelled {
                            cancel.store(true, Ordering::SeqCst);
                        }
                        Ok(json!(crate::ImportReport { scan: report, previews }))
                    });
                if result.is_err() {
                    scan_busy.store(false, Ordering::SeqCst);
                }
                result?
            }
            "start_metadata_index" => {
                let input: QueryInput = serde_json::from_value(args)?;
                self.jobs
                    .start("metadata", self.app.clone(), move |cancel, id| {
                        let catalog = Catalog::open(&operations.state.db)?;
                        let mut processed = 0;
                        let mut errors = Vec::new();
                        loop {
                            if cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled() {
                                break;
                            }
                            let batch = catalog.metadata_candidates(&input.query, 16)?;
                            if batch.is_empty() {
                                break;
                            }
                            for item in batch {
                                if cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled() {
                                    break;
                                }
                                let result = catalog.source_path(item.id).and_then(|source| {
                                    crate::media::read_metadata(&source, &|| {
                                        cancel.load(Ordering::SeqCst)
                                            || operations.stop.is_cancelled()
                                    })
                                });
                                if cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled() {
                                    break;
                                }
                                match result {
                                    Ok(fields) => catalog.index_metadata(&item, &fields, None)?,
                                    Err(error) => {
                                        let message = format!("{}: {error:#}", item.id);
                                        catalog.index_metadata(
                                            &item,
                                            &Default::default(),
                                            Some(&message),
                                        )?;
                                        errors.push(message);
                                    }
                                }
                                processed += 1;
                                operations.jobs.progress(
                                    &id,
                                    json!({"processed":processed,"failed":errors.len()}),
                                    operations.app.as_ref(),
                                );
                            }
                        }
                        Ok(json!({"processed":processed,"errors":errors}))
                    })?
            }
            "start_export" => {
                let input: ExportInput = serde_json::from_value(args)?;
                validate_ids(&input.ids, 10000)?;
                input.options.validate()?;
                let extension = crate::batch_export::extension(&input.format)?.to_owned();
                ensure!(
                    input.destination.is_absolute() && input.destination.is_dir(),
                    "destination must be an existing absolute directory"
                );
                let db = self.state.db.clone();
                let ids = input.ids.clone();
                let snapshots = tokio::task::spawn_blocking(move || -> Result<_> {
                    let catalog = Catalog::open(db)?;
                    ids.into_iter()
                        .map(|id| Ok((id, catalog.source_path(id)?, settings(&catalog, id)?.0)))
                        .collect::<Result<Vec<_>>>()
                })
                .await??;
                ensure!(!self.stop.is_cancelled(), "MCP server is stopped");
                self.jobs.start("export", self.app.clone(), move |cancel, id| {
                    let mut files = Vec::new();
                    let mut errors = Vec::new();
                    let total = snapshots.len();
                    let cancelled = || cancel.load(Ordering::SeqCst) || operations.stop.is_cancelled();
                    for (photo_id, source, settings) in snapshots {
                        if cancelled() { break; }
                        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                        let destination = input.destination.join(format!("{stem}-{photo_id}-edited.{extension}"));
                        let result = crate::batch_export::export_to(
                            &source, &settings, &input.format, &input.options, &destination, &cancelled,
                        );
                        match result {
                            Ok(()) => files.push(destination),
                            Err(error) => errors.push(json!({"id":photo_id,"error":format!("{error:#}")})),
                        }
                        operations.jobs.progress(
                            &id, json!({"completed":files.len(),"failed":errors.len(),"total":total}), operations.app.as_ref(),
                        );
                    }
                    Ok(json!({"files":files,"errors":errors,"total":total}))
                })?
            }
            _ => unreachable!(),
        };
        Ok(json!(job))
    }
}
struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
