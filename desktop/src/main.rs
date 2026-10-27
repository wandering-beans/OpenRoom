#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod batch_export;
mod catalogs;
mod deferred_previews;
mod develop;
mod library;
mod library_previews;
mod preview_queue;
mod mcp;
mod media;
mod preferences;
#[cfg(target_os = "macos")]
mod rtengine_worker;
mod workflow;
use advanced_explorer::{Annotation, Catalog, Cursor, Detail, Folder, Query, Root, ScanReport};
use media::Renderer;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::time::SystemTime;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::UNIX_EPOCH,
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

type Reply<T> = Result<T, String>;
struct CloseGuard(AtomicBool);
#[tauri::command]
fn arm_close_guard(guard: State<'_, CloseGuard>) {
    guard.0.store(true, Ordering::SeqCst);
}
#[tauri::command]
async fn close_window(app: tauri::AppHandle, guard: State<'_, CloseGuard>) -> Reply<()> {
    mcp::stop_for_exit(&app).await?;
    guard.0.store(false, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

#[derive(Clone)]
struct AppState {
    db: PathBuf,
    cache: PathBuf,
    library: Arc<library_previews::Store>,
    renderer: Renderer,
    scanning: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    thumbs: Arc<tokio::sync::Semaphore>,
    previews: Arc<tokio::sync::Semaphore>,
    preview_version: Arc<AtomicUsize>,
    metadata_jobs: Arc<tokio::sync::Semaphore>,
    metadata_version: Arc<AtomicUsize>,
}
async fn work<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Reply<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn roots(state: State<'_, AppState>) -> Reply<Vec<Root>> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.roots()).await
}
#[tauri::command]
async fn folders(state: State<'_, AppState>) -> Reply<Vec<Folder>> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.folders()).await
}
#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    query: Query,
    cursor: Option<Cursor>,
) -> Reply<library::LibraryPage> {
    let db = state.db.clone();
    work(move || library::photo_page(&Catalog::open(db)?, &query, cursor.as_ref())).await
}
#[tauri::command]
async fn count(state: State<'_, AppState>, query: Query) -> Reply<i64> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.count(&query)).await
}
#[tauri::command]
async fn detail(state: State<'_, AppState>, id: i64) -> Reply<Detail> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.detail(id)).await
}
#[tauri::command]
async fn annotate(state: State<'_, AppState>, ids: Vec<i64>, annotation: Annotation) -> Reply<()> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.annotate(&ids, &annotation)).await
}
#[tauri::command]
fn catalog_path(state: State<'_, AppState>) -> String {
    state.db.to_string_lossy().into_owned()
}
#[tauri::command]
async fn reveal_catalog(state: State<'_, AppState>) -> Reply<()> {
    let db = state.db.clone();
    work(move || {
        tauri_plugin_opener::reveal_item_in_dir(db)?;
        Ok(())
    })
    .await
}
#[tauri::command]
async fn save_catalog_copy(
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Reply<Option<String>> {
    let db = state.db.clone();
    let library = state.library.clone();
    work(move || {
        let Some(destination) = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter(preferences::text(&app, "OpenRoomカタログ", "OpenRoom Catalog"), &["openroom"])
            .set_file_name("catalog-copy.openroom")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let destination = destination
            .into_path()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        anyhow::ensure!(
            destination
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("openroom")),
            "カタログの拡張子は.openroomにしてください。"
        );
        library.backup(&db, &destination)?;
        Ok(Some(destination.to_string_lossy().into_owned()))
    })
    .await
}
#[tauri::command]
fn cancel_preview(state: State<'_, AppState>) {
    state.preview_version.fetch_add(1, Ordering::SeqCst);
}
#[tauri::command]
async fn photo_metadata(
    state: State<'_, AppState>,
    id: i64,
) -> Reply<advanced_explorer_media::metadata::PhotoMetadata> {
    let version = state.metadata_version.fetch_add(1, Ordering::SeqCst) + 1;
    let generation = state.metadata_version.clone();
    let (db, cache, renderer) = (
        state.db.clone(),
        state.cache.clone(),
        state.renderer.clone(),
    );
    let request = work(move || prepare_render(db, cache, renderer, id, 384)).await?;
    let metadata_path = request.metadata.with_extension("photo.json");
    let path = metadata_path.clone();
    if let Some(value) = work(move || {
        Ok(std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok()))
    })
    .await?
    {
        return Ok(value);
    }
    let _permit = state
        .metadata_jobs
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| e.to_string())?;
    let db = state.db.clone();
    work(move || {
        let cancelled = || generation.load(Ordering::SeqCst) != version;
        anyhow::ensure!(!cancelled(), "選択対象が変更されました。");
        Catalog::open(db)?.detail(id)?;
        let result = media::read_metadata(&request.source, &cancelled)?;
        std::fs::write(metadata_path, serde_json::to_vec(&result)?)?;
        Ok(result)
    })
    .await
}
#[tauri::command]
async fn choose_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> Reply<Vec<Root>> {
    let db = state.db.clone();
    work(move || {
        let Some(paths) = app.dialog().file().blocking_pick_folders() else {
            return Ok(Vec::new());
        };
        let paths = paths
            .into_iter()
            .map(|path| path.into_path().map_err(|e| anyhow::anyhow!(e.to_string())))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Catalog::open(db)?.add_roots(&paths)
    })
    .await
}
#[tauri::command]
async fn scan(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    root_id: Option<i64>,
) -> Reply<ImportReport> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("フォルダを読み込み中です。".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let db = state.db.clone();
    let cancel = state.cancel.clone();
    let scanning = state.scanning.clone();
    let preview_state = (*state).clone();
    let result = work(move || {
        let mut catalog = Catalog::open(db)?;
        let progress = |report: &ScanReport| {
            let _ = app.emit("scan-progress", report);
        };
        let report = match root_id {
            Some(id) => catalog.scan_root(id, &cancel, progress),
            None => catalog.scan(&cancel, progress),
        }?;
        let previews = if report.cancelled {
            library_previews::Progress::default()
        } else {
            library_previews::prepare_import(&preview_state, root_id, |report| {
                let _ = app.emit("preview-progress", report);
            })?
        };
        Ok(ImportReport {
            scan: report,
            previews,
        })
    })
    .await;
    scanning.store(false, Ordering::SeqCst);
    result
}
#[derive(Serialize)]
struct ImportReport {
    #[serde(flatten)]
    scan: ScanReport,
    previews: library_previews::Progress,
}
#[tauri::command]
async fn prepare_library_previews(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    avif_supported: bool,
) -> Reply<library_previews::Progress> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("読み込みが進行中です。".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    state.library.set_avif_supported(avif_supported);
    let copied = (*state).clone();
    let result = work(move || {
        library_previews::prepare(&copied, None, |report| {
            let _ = app.emit("preview-progress", report);
        })
    })
    .await;
    state.scanning.store(false, Ordering::SeqCst);
    result
}
#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Preview {
    path: String,
    width: usize,
    height: usize,
    render_width: usize,
    render_height: usize,
}
fn source(catalog: &Catalog, id: i64) -> anyhow::Result<PathBuf> {
    catalog.source_path(id).map_err(|error| {
        anyhow::anyhow!("原本を読み込めません。NAS接続と保存先を確認してください。\n{error:#}")
    })
}
fn prune(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("png" | "json" | "tif" | "tmp")
            ) {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), p))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let limit = library_previews::WORK_LIMIT - library_previews::IMPORT_RESERVE;
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.0);
    // Startup has no active render in this process; reserve enough room for library preparation.
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}
struct RenderRequest {
    source: PathBuf,
    image: PathBuf,
    metadata: PathBuf,
    #[cfg(test)]
    size: u32,
}
fn prepare_render(
    db: PathBuf,
    cache: PathBuf,
    renderer: Renderer,
    id: i64,
    size: u32,
) -> anyhow::Result<RenderRequest> {
    let path = source(&Catalog::open(db)?, id)?;
    let m = std::fs::metadata(&path)?;
    let mut hash = DefaultHasher::new();
    (
        &path,
        m.len(),
        m.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
        size,
    )
        .hash(&mut hash);
    renderer.cache_key().hash(&mut hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (m.dev(), m.ino()).hash(&mut hash);
    }
    let key = format!("{:x}", hash.finish());
    Ok(RenderRequest {
        source: path,
        image: cache.join(format!("{key}.png")),
        metadata: cache.join(format!("{key}.json")),
        #[cfg(test)]
        size,
    })
}
#[cfg(test)]
fn cached_preview(request: &RenderRequest) -> Option<Preview> {
    let value: Preview = serde_json::from_slice(&std::fs::read(&request.metadata).ok()?).ok()?;
    if !request.image.is_file() {
        return None;
    }
    let _ = std::fs::File::open(&request.image).and_then(|f| f.set_modified(SystemTime::now()));
    let _ = std::fs::File::open(&request.metadata).and_then(|f| f.set_modified(SystemTime::now()));
    Some(Preview {
        path: request.image.to_string_lossy().into_owned(),
        ..value
    })
}
#[cfg(test)]
fn finish_render(
    request: &RenderRequest,
    renderer: &Renderer,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<Preview> {
    anyhow::ensure!(!cancelled(), "表示対象が変更されました。");
    if let Some(preview) = cached_preview(request) {
        return Ok(preview);
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = request.image.with_extension(format!("{nonce}.tmp"));
    let result = (|| -> anyhow::Result<Preview> {
        let mut preview = renderer.render(&request.source, &temp, request.size, cancelled)?;
        anyhow::ensure!(!cancelled(), "表示対象が変更されました。");
        preview.path = request.image.to_string_lossy().into_owned();
        std::fs::rename(&temp, &request.image)?;
        std::fs::write(&request.metadata, serde_json::to_vec(&preview)?)?;
        Ok(preview)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
#[cfg(test)]
fn render(
    db: PathBuf,
    cache: PathBuf,
    renderer: Renderer,
    id: i64,
    large: bool,
) -> anyhow::Result<Preview> {
    let request = prepare_render(
        db,
        cache,
        renderer.clone(),
        id,
        if large { 4096 } else { 384 },
    )?;
    finish_render(&request, &renderer, &|| false)
}
#[tauri::command]
async fn preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    large: bool,
    bound: Option<u32>,
) -> Reply<serde_json::Value> {
    let size = if large { bound.unwrap_or(1600) } else { 384 };
    if ![384, 1600].contains(&size) {
        return Err("Invalid preview size".into());
    }
    let request = preview_inner(&state, id, large, bound);
    tokio::pin!(request);
    let result = loop {
        let ready = state.library.ready.notified();
        tokio::pin!(ready);
        ready.as_mut().enable();
        // Also accept pixels published before this request joined an existing job.
        let copied = (*state).clone();
        if let Some(image) = work(move || {
            copied.library.early_preview(&Catalog::open(&copied.db)?, id, size == 1600)
        }).await? {
            return Ok(image);
        }
        tokio::select! {
            result = &mut request => break result?,
            _ = &mut ready => {}
        }
    };
    // Source validation must never delay showing an already saved library image.
    if large && let Ok(permit) = state.previews.clone().try_acquire_owned() {
        let copied = (*state).clone();
        let version = copied.preview_version.load(Ordering::SeqCst);
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            let updated = (|| -> anyhow::Result<bool> {
                let catalog = Catalog::open(&copied.db)?;
                if !catalog.refresh_preview_source(id)? {
                    return Ok(false);
                }
                copied.library.generate(
                    &copied,
                    id,
                    &|| copied.preview_version.load(Ordering::SeqCst) != version,
                    true,
                )
            })();
            if matches!(updated, Ok(true)) {
                let _ = app.emit("library-preview-updated", id);
            }
        });
    }
    serde_json::to_value(result).map_err(|error| error.to_string())
}
async fn preview_inner(
    state: &AppState,
    id: i64,
    large: bool,
    bound: Option<u32>,
) -> Reply<Preview> {
    let size = if large { bound.unwrap_or(1600) } else { 384 };
    if ![384, 1600].contains(&size) {
        return Err("Invalid preview size".into());
    }
    if large {
        state.preview_version.fetch_add(1, Ordering::SeqCst);
    }
    let copied = state.clone();
    let hit = work(move || copied.library.lookup(&Catalog::open(&copied.db)?, id, size)).await?;
    if let Some(hit) = hit {
        if large {
            state.library.protect(&hit);
        }
        return Ok(hit);
    }
    let permits = if large {
        state.previews.clone()
    } else {
        state.thumbs.clone()
    };
    let permit = permits.acquire_owned().await.map_err(|e| e.to_string())?;
    let copied = state.clone();
    let result = work(move || {
        let _permit = permit;
        let catalog = Catalog::open(&copied.db)?;
        if let Some(hit) = copied.library.lookup(&catalog, id, size)? {
            return Ok(hit);
        }
        copied.library.generate_visible(&copied, id, large)?;
        copied
            .library
            .lookup(&catalog, id, size)?
            .ok_or_else(|| anyhow::anyhow!("プレビューを準備できませんでした。"))
    })
    .await?;
    if large {
        state.library.protect(&result);
    }
    Ok(result)
}
#[tauri::command]
async fn refresh_library_preview(state: State<'_, AppState>, id: i64) -> Reply<()> {
    let copied = (*state).clone();
    work(move || {
        let _foreground = copied.library.updates.foreground();
        let catalog = Catalog::open(&copied.db)?;
        copied.library.generate_explicit(&copied, id)?;
        anyhow::ensure!(catalog.library_preview(id)?.is_some_and(|record|
            catalog.preview_stamp(id).is_ok_and(|stamp| record.stamp == stamp)),
            "一覧画像を更新できませんでした。");
        Ok(())
    }).await
}
#[tauri::command]
async fn invalidate_library_preview(state: State<'_, AppState>, id: i64, bound: u32) -> Reply<()> {
    let copied = (*state).clone();
    work(move || {
        copied
            .library
            .invalidate(&Catalog::open(&copied.db)?, id, bound)
    })
    .await
}
#[tauri::command]
async fn reveal(state: State<'_, AppState>, id: i64) -> Reply<()> {
    let db = state.db.clone();
    work(move || {
        let path = source(&Catalog::open(db)?, id)?;
        tauri_plugin_opener::reveal_item_in_dir(path)?;
        Ok(())
    })
    .await
}
fn main() {
    if develop::run_worker() {
        return;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--photo-metadata")) {
        let result = (|| -> anyhow::Result<()> {
            let args: Vec<_> = std::env::args_os().skip(2).collect();
            anyhow::ensure!(args.len() == 1, "Invalid metadata arguments");
            println!(
                "{}",
                serde_json::to_string(&advanced_explorer_media::metadata::read(Path::new(
                    &args[0]
                ))?)?
            );
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }

    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--render-media")) {
        let args: Vec<_> = std::env::args_os().skip(2).collect();
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(args.len() == 3, "Invalid render arguments");
            let size = args[2].to_str().unwrap_or("").parse()?;
            let info =
                advanced_explorer_media::render(Path::new(&args[0]), Path::new(&args[1]), size)?;
            println!("{}", serde_json::to_string(&info)?);
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(CloseGuard(AtomicBool::new(false)))
        .menu(|app| {
            let menu = tauri::menu::Menu::default(app)?;
            if let Some(item) = menu.items()?.first()
                && let Some(submenu) = item.as_submenu()
            {
                submenu.insert(&tauri::menu::MenuItem::with_id(
                    app, "preferences", preferences::text(app, "環境設定…", "Settings…"), true, Some("CmdOrCtrl+,"),
                )?, 0)?;
            }
            #[cfg(target_os = "macos")]
            {
                // Native Quit bypasses Tauri's ExitRequested on macOS. Keep the standard
                // menus, replacing only its final app-menu item with an ordinary event.
                if let Some(item) = menu.items()?.first()
                    && let Some(submenu) = item.as_submenu()
                {
                    let position = submenu.items()?.len() - 1;
                    submenu.remove_at(position)?;
                    submenu.insert(
                        &tauri::menu::MenuItem::with_id(
                            app,
                            "quit-guarded",
                            preferences::text(app, "OpenRoomを終了", "Quit OpenRoom"),
                            true,
                            Some("CmdOrCtrl+Q"),
                        )?,
                        position,
                    )?;
                }
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "preferences" {
                let _ = app.emit("open-preferences", ());
            }
            if event.id().as_ref() == "quit-guarded" {
                if app.state::<CloseGuard>().0.load(Ordering::SeqCst) {
                    let _ = app.emit("close-requested", ());
                } else {
                    app.exit(0);
                }
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.state::<CloseGuard>().0.load(Ordering::SeqCst)
            {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
        })
        .setup(|app| {
            let data = std::env::var_os("AE_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            std::fs::create_dir_all(&data)?;
            let db = match catalogs::startup_catalog(&data).and_then(|path| {
                if let Some(path) = &path {
                    Catalog::open(path)?;
                }
                Ok(path)
            }) {
                Ok(path) => path,
                Err(error) => {
                    let message = preferences::text(app.handle(), "選択したカタログを開けませんでした。別のカタログを選択するか、新規作成してください。", "Could not open the selected catalog. Select another catalog or create a new one.");
                    app.dialog().message(format!("{message}\n{error}")).show(|_| {});
                    let default = data.join("catalog.db");
                    default.is_file().then_some(default)
                }
            };
            // Keep the existing command state temporary until the user names their first catalog.
            let (db, temporary) = match db {
                Some(path) => (path, None),
                None => {
                    let directory = tempfile::tempdir()?;
                    (directory.path().join("setup.db"), Some(directory))
                }
            };
            app.manage(mcp::McpState::new(data.clone()));
            app.manage(catalogs::CatalogSelection { data, temporary });
            Catalog::open(&db)?;
            let library = Arc::new(library_previews::Store::new(&db)?);
            let cache = library.directory.join("working");
            std::fs::create_dir_all(&cache)?;
            let renderer = Renderer::new(&cache)?;
            app.asset_protocol_scope().allow_directory(&library.directory,true)?;
            prune(&cache);
            app.manage(develop::DevelopState::new()?);
            app.manage(batch_export::ExportState::default());
            app.manage(AppState {
                db,
                cache,
                library,
                renderer,
                scanning: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(AtomicBool::new(false)),
                thumbs: Arc::new(tokio::sync::Semaphore::new(3)),
                previews: Arc::new(tokio::sync::Semaphore::new(1)),
                preview_version: Arc::new(AtomicUsize::new(0)),
                metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
                metadata_version: Arc::new(AtomicUsize::new(0)),
            });
            Ok(())
        })
        .manage(library::IndexState::default())
        .invoke_handler(tauri::generate_handler![
            mcp::mcp_ready, mcp::mcp_status, mcp::mcp_configure, mcp::mcp_token, mcp::mcp_rotate,
            mcp::mcp_claim, mcp::mcp_execute, mcp::mcp_complete,
            library::set_photo_attributes, library::metadata_facets, library::index_metadata_batch, library::cancel_metadata_index,
            library::remove_from_catalog,
            preferences::get_language, preferences::set_language,
            arm_close_guard,
            close_window,
            batch_export::export_develop_batch,
            batch_export::choose_export_folder,
            batch_export::cancel_batch_export,
            workflow::develop_workflow,
            workflow::preset_file,
            deferred_previews::set_preview_photo,
            develop::get_develop,
            develop::save_develop,
            refresh_library_preview,
            develop::develop_history,
            develop::history_settings,
            catalogs::choose_catalog,
            catalogs::catalog_needs_creation,
            catalogs::restart_catalog,
            develop::develop_preview,
            develop::cancel_develop_preview,
            develop::prefetch::set_develop_prefetch,
            develop::export_develop,
            roots,
            folders,
            search,
            count,
            detail,
            annotate,
            catalog_path,
            reveal_catalog,
            save_catalog_copy,
            photo_metadata,
            cancel_preview,
            choose_folder,
            scan,
            prepare_library_previews,
            invalidate_library_preview,
            cancel_scan,
            preview,
            reveal
        ])
        .build(tauri::generate_context!())
        .expect("desktop app failed")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<mcp::McpState>().shutdown();
                app.state::<develop::DevelopState>().shutdown();
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event
                && app.state::<CloseGuard>().0.load(Ordering::SeqCst)
            {
                api.prevent_exit();
                let _ = app.emit("close-requested", ());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_reclaims_recent_working_files_and_leaves_import_reserve() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..4 {
            std::fs::File::create(dir.path().join(format!("develop-{index}.png")))
                .unwrap()
                .set_len(300_000_000)
                .unwrap();
        }
        let renderer = dir.path().join("renderer-bundled");
        std::fs::write(&renderer, b"renderer").unwrap();
        prune(dir.path());
        let size = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum::<u64>();
        assert!(size + library_previews::IMPORT_RESERVE <= library_previews::WORK_LIMIT);
        assert_eq!(std::fs::read(renderer).unwrap(), b"renderer");
    }
    #[test]
    #[ignore = "requires AE_FORMAT_ASSETS or AE_RAW_ASSETS"]
    fn media_cache_and_catalog() {
        let assets = PathBuf::from(
            std::env::var("AE_FORMAT_ASSETS")
                .or_else(|_| std::env::var("AE_RAW_ASSETS"))
                .unwrap(),
        );
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("catalog.db");
        let cache = temp.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let renderer = Renderer::new(&cache).unwrap();
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&assets).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let items = catalog.page(&Query::default(), 100, None).unwrap().items;
        assert!(items.len() >= 8);
        for item in items {
            let first =
                render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
            let before = std::fs::read(&first.path).unwrap();
            let mut timings = Vec::new();
            for _ in 0..30 {
                let started = std::time::Instant::now();
                let second =
                    render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
                timings.push(started.elapsed().as_secs_f64() * 1000.);
                assert_eq!(first.path, second.path);
            }
            timings.sort_by(f64::total_cmp);
            println!("cached {} p95={:.3}ms", item.name, timings[28]);
            assert_eq!(before, std::fs::read(first.path).unwrap());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires AE_TEST_ASSETS from the native fixture generator"]
    fn rendering_cache_and_source_boundary() {
        let assets =
            PathBuf::from(std::env::var("AE_TEST_ASSETS").expect("fixture directory required"));
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("catalog.db");
        let cache = temp.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let renderer = Renderer::new(&cache).unwrap();
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&assets).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let mut results = Vec::new();
        for ext in ["dng", "png", "jpeg"] {
            let item = catalog
                .page(
                    &Query {
                        extensions: vec![ext.into()],
                        ..Default::default()
                    },
                    1,
                    None,
                )
                .unwrap()
                .items
                .remove(0);
            let start = std::time::Instant::now();
            let first =
                render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
            let uncached_ms = start.elapsed().as_secs_f64() * 1000.0;
            let bytes_before = std::fs::read(&first.path).unwrap();
            let start = std::time::Instant::now();
            let second =
                render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
            let cached_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(first.path, second.path);
            assert_eq!(bytes_before, std::fs::read(&second.path).unwrap());
            results.push(serde_json::json!({"format":ext,"first_thumbnail_ms":uncached_ms,"cached_thumbnail_ms":cached_ms}));
        }
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
        // A file replaced by an out-of-root symlink must not expose the target.
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let path = root.join("safe.png");
        std::fs::copy(assets.join("風景-00.png"), &path).unwrap();
        catalog.add_root(&root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let item = catalog
            .page(
                &Query {
                    text: "safe".into(),
                    ..Default::default()
                },
                1,
                None,
            )
            .unwrap()
            .items
            .remove(0);
        let cached = render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(1))
            .unwrap();
        let updated = render(db.clone(), cache.clone(), renderer.clone(), item.id, false).unwrap();
        assert_ne!(cached.path, updated.path);
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&db, &path).unwrap();
        assert!(source(&catalog, item.id).is_err());
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    fn catalog_and_decoders_accept_the_same_raw_formats() {
        assert_eq!(
            advanced_explorer::RAW_EXTENSIONS,
            advanced_explorer_media::RAW_EXTENSIONS
        );
        for ext in advanced_explorer::RAW_EXTENSIONS {
            assert!(advanced_explorer::supported(ext));
            assert!(advanced_explorer_media::is_raw(Path::new(&format!(
                "camera.{}",
                ext.to_ascii_uppercase()
            ))));
        }
    }
    #[test]
    #[ignore = "30,000 file desktop query benchmark"]
    fn desktop_queries_at_30000() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assets");
        std::fs::create_dir(&root).unwrap();
        for i in 0..30000 {
            let ext = advanced_explorer::SUPPORTED_EXTENSIONS
                [i % advanced_explorer::SUPPORTED_EXTENSIONS.len()];
            std::fs::write(root.join(format!("日本素材-{i:05}.{ext}")), b"fixture").unwrap();
        }
        let mut catalog = Catalog::open(temp.path().join("catalog.db")).unwrap();
        catalog.add_root(root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        for (name, q, budget) in [
            ("list", Query::default(), 50.0),
            (
                "japanese_short",
                Query {
                    text: "日本".into(),
                    ..Default::default()
                },
                200.0,
            ),
            (
                "dng_filter",
                Query {
                    extensions: vec!["dng".into()],
                    ..Default::default()
                },
                50.0,
            ),
            (
                "no_match",
                Query {
                    text: "龍".into(),
                    ..Default::default()
                },
                200.0,
            ),
        ] {
            let mut timings = Vec::new();
            for i in 0..110 {
                let start = std::time::Instant::now();
                std::hint::black_box(catalog.page(&q, 200, None).unwrap());
                if i >= 10 {
                    timings.push(start.elapsed().as_secs_f64() * 1000.0)
                }
            }
            timings.sort_by(f64::total_cmp);
            println!(
                "desktop {name}: p95={:.3}ms max={:.3}ms",
                timings[94], timings[99]
            );
            assert!(timings[94] < budget);
        }
    }
}

#[cfg(test)]
mod display_regressions {
    use super::*;
    #[test]
    fn imported_preview_bypasses_busy_decoder_and_survives_offline_source() {
        let temp = tempfile::tempdir().unwrap();
        let assets = temp.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        let path = assets.join("photo.png");
        image::RgbImage::from_pixel(100, 80, image::Rgb([1, 2, 3]))
            .save(&path)
            .unwrap();
        let db = temp.path().join("catalog.db");
        let cache = temp.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&assets).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        let renderer = Renderer::new(&cache).unwrap();
        let state = Arc::new(AppState {
            library: Arc::new(crate::library_previews::Store::new(&db).unwrap()),
            db,
            cache,
            renderer,
            scanning: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            thumbs: Arc::new(tokio::sync::Semaphore::new(0)),
            previews: Arc::new(tokio::sync::Semaphore::new(1)),
            preview_version: Arc::new(AtomicUsize::new(0)),
            metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
            metadata_version: Arc::new(AtomicUsize::new(0)),
        });
        state
            .library
            .generate(&state, id, &|| false, false)
            .unwrap();
        let warm = state.library.lookup(&catalog, id, 384).unwrap().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let copied = state.clone();
        std::thread::spawn(move || {
            let result = tauri::async_runtime::block_on(preview_inner(&copied, id, false, None));
            let _ = send.send(result);
        });
        let result = receive.recv_timeout(Duration::from_secs(2));
        state.thumbs.add_permits(1); // Release a regressed waiter even on assertion failure.
        assert_eq!(
            result
                .expect("warm image queued behind cold RAW decoder")
                .unwrap()
                .path,
            warm.path
        );
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(preview_inner(&state, id, false, None))
                .unwrap()
                .path,
            warm.path
        );
        let mut timings = Vec::new();
        for _ in 0..32 {
            let start = std::time::Instant::now();
            tauri::async_runtime::block_on(preview_inner(&state, id, true, Some(1600))).unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.);
        }
        timings.sort_by(f64::total_cmp);
        println!("offline stored library preview p95={:.3}ms", timings[30]);
        assert!(timings[30] < 150., "cached backend reply exceeds 150ms");
        assert!(
            tauri::async_runtime::block_on(preview_inner(&state, id, true, Some(4096))).is_err()
        );
        assert!(
            tauri::async_runtime::block_on(preview_inner(&state, id, true, Some(999999))).is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn obsolete_worker_is_killed_promptly() {
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("10");
        let start = std::time::Instant::now();
        let result: anyhow::Result<serde_json::Value> = media::run_worker(command, None, &|| {
            start.elapsed() > Duration::from_millis(30)
        });
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
