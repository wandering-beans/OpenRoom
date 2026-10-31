//! Sequential isolated workers bound peak memory; completed outputs survive cancellation.
use crate::{AppState, Catalog, Reply, work};
use advanced_explorer_develop::{ExportOptions, Settings};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Emitter, State};
use tauri_plugin_dialog::DialogExt;
pub struct ExportState {
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}
impl ExportState {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}
impl Default for ExportState {
    fn default() -> Self {
        Self {
            busy: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
#[derive(Clone, Serialize, Default)]
pub struct ExportReport {
    pub completed: usize,
    pub total: usize,
    pub cancelled: bool,
    pub errors: Vec<String>,
    pub folder: String,
}
#[tauri::command]
pub fn cancel_batch_export(state: State<'_, ExportState>) {
    state.cancel.store(true, Ordering::SeqCst);
}
#[tauri::command]
pub async fn choose_export_folder(
    app: tauri::AppHandle,
    window: tauri::Window,
) -> Reply<Option<String>> {
    work(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .blocking_pick_folder()
            .map(|folder| {
                folder
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
            })
            .transpose()
    })
    .await
}

fn output_stem(rule: &str, source: &Path, id: i64, sequence: usize) -> Result<String> {
    let template = rule
        .replace("{name}", "")
        .replace("{id}", "")
        .replace("{seq}", "");
    ensure!(
        !template.contains(['{', '}']),
        "命名規則には {{name}}、{{id}}、{{seq}} を使えます"
    );
    let name = source.file_stem().unwrap_or_default().to_string_lossy();
    let stem = rule
        .replace("{id}", &id.to_string())
        .replace("{seq}", &format!("{sequence:04}"))
        .replace("{name}", &name);
    ensure!(
        !stem.is_empty()
            && stem.len() <= 240
            && !stem.ends_with(['.', ' '])
            && !stem
                .chars()
                .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)),
        "ファイル名が不正です。区切り文字や使用できない記号を除いてください。"
    );
    Ok(stem)
}

fn available_destination(folder: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut destination = folder.join(format!("{stem}.{extension}"));
    let mut suffix = 1;
    while destination.exists() {
        destination = folder.join(format!("{stem}-{suffix}.{extension}"));
        suffix += 1;
    }
    destination
}
pub fn extension(format: &str) -> Result<&str> {
    match format {
        "jpeg" => Ok("jpg"),
        "png" => Ok("png"),
        "tiff" => Ok("tif"),
        _ => anyhow::bail!("Unsupported export format"),
    }
}
pub fn publish(temporary: &Path, destination: &Path) -> Result<()> {
    match std::fs::hard_link(temporary, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(error.into()),
        Err(_) => {
            // ExFAT and network shares may not support hard links. Still never truncate a file.
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            let result = (|| -> Result<()> {
                let mut input = std::fs::File::open(temporary)?;
                std::io::copy(&mut input, &mut output)?;
                output.sync_all()?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(destination);
            }
            result
        }
    }
}
/// Render into the destination directory and publish without replacing existing files.
pub fn export_to(
    source: &Path,
    settings: &Settings,
    format: &str,
    options: &ExportOptions,
    destination: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<()> {
    settings.validate()?;
    options.validate()?;
    extension(format)?;
    ensure!(
        !destination.exists(),
        "既存ファイルは上書きしません。別の名前を指定してください。"
    );
    ensure!(!cancelled(), "書き出しを中止しました");
    let folder = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid destination"))?;
    let temporary = tempfile::Builder::new()
        .prefix(".openroom-export-")
        .tempdir_in(folder)?;
    let output = temporary.path().join("image");
    let recipe = temporary.path().join("recipe.json");
    std::fs::write(&recipe, serde_json::to_vec(settings)?)?;
    #[cfg(not(test))]
    {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .arg("--develop-export")
            .arg(source)
            .arg(&output)
            .arg(&recipe)
            .arg(format)
            .arg(serde_json::to_string(options)?);
        let _: serde_json::Value = crate::media::run_worker(command, Some(&output), cancelled)?;
    }
    #[cfg(test)]
    {
        advanced_explorer_develop::export_source_with_options(
            source, settings, &output, format, options,
        )?;
    }
    ensure!(!cancelled(), "書き出しを中止しました");
    publish(&output, destination)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchExportRequest {
    ids: Vec<i64>,
    format: String,
    options: ExportOptions,
    folder: PathBuf,
    naming: String,
}

#[tauri::command]
pub async fn export_develop_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    exports: State<'_, ExportState>,
    request: BatchExportRequest,
) -> Reply<ExportReport> {
    if exports.busy.swap(true, Ordering::SeqCst) {
        return Err("書き出し中です".into());
    }
    let guard = BusyGuard(exports.busy.clone());
    exports.cancel.store(false, Ordering::SeqCst);
    let cancel = exports.cancel.clone();
    let db = state.db.clone();
    work(move || {
        let _guard = guard;
        let BatchExportRequest {
            ids,
            format,
            options,
            folder,
            naming,
        } = request;
        options.validate()?;
        let extension = extension(&format)?;
        ensure!(
            !ids.is_empty() && ids.len() <= 10000,
            "Select 1–10000 photos"
        );
        output_stem(&naming, Path::new("photo.jpg"), 1, 1)?;
        ensure!(
            folder.is_absolute() && folder.is_dir(),
            "書き出し先のフォルダを選択してください"
        );
        let catalog = Catalog::open(db)?;
        // Freeze settings before starting; subsequent editing cannot change this export job.
        let jobs = ids
            .into_iter()
            .map(|id| -> Result<_> {
                let source = catalog.source_path(id)?;
                let (recipe, _) = catalog.develop_recipe(id)?;
                let settings: Settings = serde_json::from_str(recipe.as_deref().unwrap_or("{}"))?;
                settings.validate()?;
                Ok((id, source, settings))
            })
            .collect::<Vec<_>>();
        let mut report = ExportReport {
            total: jobs.len(),
            folder: folder.to_string_lossy().into_owned(),
            ..Default::default()
        };
        for (index, job) in jobs.into_iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                report.cancelled = true;
                break;
            }
            let result = (|| -> Result<()> {
                let (id, source, settings) = job?;
                let stem = output_stem(&naming, &source, id, index + 1)?;
                let destination = available_destination(&folder, &stem, extension);
                export_to(&source, &settings, &format, &options, &destination, &|| {
                    cancel.load(Ordering::SeqCst)
                })
                .map_err(|error| anyhow::anyhow!("{}: {error:#}", source.display()))
            })();
            if cancel.load(Ordering::SeqCst) {
                report.cancelled = true;
                break;
            }
            match result {
                Ok(()) => report.completed += 1,
                Err(error) => report.errors.push(format!("{error:#}")),
            }
            let _ = app.emit("develop-export-progress", &report);
        }
        Ok(report)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_expands_tokens_and_rejects_paths() {
        let source = Path::new("写真.jpg");
        assert_eq!(
            output_stem("{name}-{seq}-{id}", source, 42, 3).unwrap(),
            "写真-0003-42"
        );
        for rule in [
            "",
            "../{name}",
            "a\\{name}",
            "{unknown}",
            "..",
            "a:",
            "a\n",
            "a ",
        ] {
            assert!(output_stem(rule, source, 42, 3).is_err(), "{rule:?}");
        }
    }

    #[test]
    fn naming_collision_preserves_existing_files() {
        let folder = tempfile::tempdir().unwrap();
        let original = folder.path().join("photo.jpg");
        std::fs::write(&original, b"original").unwrap();
        let next = available_destination(folder.path(), "photo", "jpg");
        assert_eq!(next, folder.path().join("photo-1.jpg"));
        assert_eq!(std::fs::read(original).unwrap(), b"original");
    }
}
