//! Desktop boundary for the portable development engine and its isolated decoder.
use crate::{AppState, Catalog, Preview, Reply, media, prepare_render, work};
use advanced_explorer::{HistoryEdit, HistoryEntry};
use advanced_explorer_develop::{self as engine, BaseInfo, RenderInfo, Settings, WorkingImage};
use anyhow::{Result, ensure};
use base64::Engine;
use serde::Serialize;
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

struct Resident {
    key: PathBuf,
    source_key: PathBuf,
    bound: u32,
    image: Arc<WorkingImage>,
    interactive: Option<Arc<WorkingImage>>,
}

fn render_frame(
    base: &WorkingImage,
    settings: &Settings,
    bound: u32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<engine::RenderedImage> {
    #[cfg(target_os = "macos")]
    {
        crate::rtengine_worker::render(base, settings, bound, cancelled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bound;
        engine::render(base, settings, cancelled)
    }
}
pub struct DevelopState {
    generation: Arc<AtomicUsize>,
    jobs: Arc<tokio::sync::Semaphore>,
    resident: Arc<Mutex<Option<Resident>>>,
    pool: Arc<rayon::ThreadPool>,
}
impl DevelopState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            generation: Arc::new(AtomicUsize::new(0)),
            jobs: Arc::new(tokio::sync::Semaphore::new(1)),
            resident: Arc::new(Mutex::new(None)),
            pool: Arc::new(rayon::ThreadPoolBuilder::new().num_threads(4).build()?),
        })
    }
}
#[derive(Serialize)]
pub struct Recipe {
    settings: Settings,
    revision: i64,
    defaults: Settings,
    undo: Vec<Settings>,
}
#[tauri::command]
pub async fn get_develop(state: State<'_, AppState>, id: i64) -> Reply<Recipe> {
    let db = state.db.clone();
    work(move || {
        let catalog = Catalog::open(db)?;
        let (json, revision) = catalog.develop_recipe(id)?;
        let settings: Settings = json
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        settings.validate()?;
        let undo = catalog
            .develop_history(id, None)?
            .into_iter()
            .skip(1)
            .take(10)
            .map(|row| {
                Ok(serde_json::from_str::<Settings>(
                    &catalog.history_recipe(id, row.id)?,
                )?)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .rev()
            .collect();
        Ok(Recipe {
            undo,
            settings,
            revision,
            defaults: Settings::default(),
        })
    })
    .await
}
#[tauri::command]
pub async fn save_develop(
    state: State<'_, AppState>,
    id: i64,
    settings: Settings,
    expected_revision: i64,
    steps: Vec<EditStep>,
    undo: Option<Vec<Settings>>,
) -> Reply<i64> {
    let state = (*state).clone();
    work(move || save_inner(&state, id, settings, expected_revision, steps, undo)).await
}
fn save_inner(
    state: &AppState,
    id: i64,
    settings: Settings,
    expected_revision: i64,
    steps: Vec<EditStep>,
    undo: Option<Vec<Settings>>,
) -> Result<i64> {
    settings.validate()?;
    ensure!(
        !steps.is_empty() && steps.len() <= 100,
        "Invalid history batch"
    );
    ensure!(
        steps.last().unwrap().settings == settings,
        "History does not match current settings"
    );
    let edits = steps
        .into_iter()
        .map(|step| {
            Ok(HistoryEdit {
                recipe: step.settings.catalog_recipe()?,
                label: step.label,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut catalog = Catalog::open(&state.db)?;
    if let Some(undo) = undo {
        ensure!(undo.len() <= 10, "Invalid undo history");
        let mut states = undo
            .into_iter()
            .map(|settings| {
                Ok(HistoryEdit {
                    recipe: settings.catalog_recipe()?,
                    label: "現像設定".into(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        states.push(edits.into_iter().last().unwrap());
        catalog.save_develop_undo(id, &states, expected_revision)
    } else {
        catalog.save_develop_steps(id, &edits, expected_revision)
    }
}
#[derive(serde::Deserialize)]
pub struct EditStep {
    settings: Settings,
    label: String,
}
#[tauri::command]
pub async fn develop_history(
    state: State<'_, AppState>,
    id: i64,
    before: Option<i64>,
) -> Reply<Vec<HistoryEntry>> {
    let db = state.db.clone();
    work(move || Catalog::open(db)?.develop_history(id, before)).await
}
#[tauri::command]
pub async fn history_settings(
    state: State<'_, AppState>,
    id: i64,
    history_id: i64,
) -> Reply<Settings> {
    let db = state.db.clone();
    work(move || {
        let settings: Settings =
            serde_json::from_str(&Catalog::open(db)?.history_recipe(id, history_id)?)?;
        settings.validate()?;
        Ok(settings)
    })
    .await
}
#[tauri::command]
pub fn cancel_develop_preview(develop: State<'_, DevelopState>) {
    develop.generation.fetch_add(1, Ordering::SeqCst);
}
#[derive(Clone, Serialize, serde::Deserialize)]
pub struct DevelopedPreview {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<String>,
    #[serde(default)]
    pub image_bytes: u64,
    pub info: RenderInfo,
    pub base: BaseInfo,
}

#[derive(Clone)]
struct Request {
    source: PathBuf,
    source_key: PathBuf,
    base: PathBuf,
    base_info: PathBuf,
    image: PathBuf,
    metadata: PathBuf,
}
fn request(
    db: PathBuf,
    cache: PathBuf,
    renderer: media::Renderer,
    id: i64,
    settings: &Settings,
    bound: u32,
) -> Result<Request> {
    settings.validate()?;
    ensure!(
        [0, 384, 1600, 4096].contains(&bound),
        "Invalid preview size"
    );
    // The shared key revalidates the source and includes modification time, size and file identity.
    let original = prepare_render(db, cache.clone(), renderer, id, 4096)?;
    // v6 also preserves extended ICC colors and scene-linear floating-point rasters.
    let base = original
        .image
        .with_extension(format!("{}-{bound}.tif", engine::RENDER_VERSION));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    // Render changes must invalidate thumbnails and previews, while retaining decoded bases.
    (
        engine::RENDER_VERSION,
        &base,
        bound,
        serde_json::to_string(settings)?,
    )
        .hash(&mut hash);
    let image = cache.join(format!("develop-{:x}.png", hash.finish()));
    let metadata = image.with_extension("json");
    Ok(Request {
        source: original.source,
        source_key: original.image,
        base_info: base.with_extension("json"),
        base,
        image,
        metadata,
    })
}
fn cached(request: &Request) -> Option<DevelopedPreview> {
    if !request.image.is_file() {
        return None;
    }
    if std::fs::metadata(&request.metadata).ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut result: DevelopedPreview =
        serde_json::from_slice(&std::fs::read(&request.metadata).ok()?).ok()?;
    result.path = request.image.to_string_lossy().into_owned();
    if result.image_bytes == 0
        || std::fs::metadata(&request.image).ok()?.len() != result.image_bytes
        || image::image_dimensions(&request.image).ok()? != (result.info.width, result.info.height)
    {
        return None;
    }
    if !engine::raw_profile_is_current(&result.base) {
        return None;
    }
    for path in [
        &request.image,
        &request.metadata,
        &request.base,
        &request.base_info,
    ] {
        let _ = std::fs::File::open(path).and_then(|file| file.set_modified(SystemTime::now()));
    }
    Some(result)
}
fn nonce() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}
fn read_cached_base(request: &Request) -> Option<WorkingImage> {
    if std::fs::metadata(&request.base_info).ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let info = serde_json::from_slice::<BaseInfo>(&std::fs::read(&request.base_info).ok()?).ok()?;
    if info
        .native_source
        .as_ref()
        .is_some_and(|source| request.source.canonicalize().ok().as_ref() != Some(source))
    {
        return None;
    }
    // A truncated or mismatched cache is disposable; retry from the unchanged original.
    engine::read_base(&request.base, info).ok()
}
fn cached_base(
    request: &Request,
    bound: u32,
    resident: &Mutex<Option<Resident>>,
    retain: bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Arc<WorkingImage>>> {
    ensure!(!cancelled(), "現像対象が変更されました。");
    {
        let mut cached = resident.lock().unwrap();
        if let Some(hit) = cached
            .as_mut()
            .filter(|entry| entry.source_key == request.source_key)
        {
            if hit.key == request.base && hit.bound == bound {
                return Ok(Some(hit.image.clone()));
            }
            if bound == 384 && (hit.bound == 0 || hit.bound > bound) {
                if hit.interactive.is_none() {
                    let pixels = engine::resize_linear(hit.image.pixels.clone(), bound)?;
                    let mut info = hit.image.info.clone();
                    info.width = pixels.width();
                    info.height = pixels.height();
                    ensure!(!cancelled(), "現像対象が変更されました。");
                    hit.interactive = Some(Arc::new(WorkingImage { pixels, info }));
                }
                return Ok(Some(hit.interactive.as_ref().unwrap().clone()));
            }
        }
    }
    let Some(image) = read_cached_base(request) else {
        return Ok(None);
    };
    remember_base(image, request, bound, resident, retain, cancelled).map(Some)
}
fn remember_base(
    mut image: WorkingImage,
    request: &Request,
    bound: u32,
    resident: &Mutex<Option<Resident>>,
    retain: bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Arc<WorkingImage>> {
    ensure!(!cancelled(), "現像対象が変更されました。");
    if bound > 0 {
        image.pixels = engine::resize_linear(image.pixels, bound)?;
    }
    image.info.width = image.pixels.width();
    image.info.height = image.pixels.height();
    let image = Arc::new(image);
    if retain {
        let mut resident = resident.lock().unwrap();
        let interactive = resident
            .as_ref()
            .filter(|previous| previous.source_key == request.source_key)
            .and_then(|previous| previous.interactive.clone());
        ensure!(!cancelled(), "現像対象が変更されました。");
        *resident = Some(Resident {
            key: request.base.clone(),
            source_key: request.source_key.clone(),
            bound,
            image: image.clone(),
            interactive,
        });
    }
    Ok(image)
}
fn load_base(
    request: &Request,
    bound: u32,
    resident: &Mutex<Option<Resident>>,
    retain: bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Arc<WorkingImage>> {
    if let Some(image) = cached_base(request, bound, resident, retain, cancelled)? {
        return Ok(image);
    }
    #[cfg(target_os = "macos")]
    if engine::is_native_raw(&request.source) {
        let image = crate::rtengine_worker::prepare(&request.source, bound, cancelled)?;
        return remember_base(image, request, bound, resident, retain, cancelled);
    }
    let temp = request.base.with_extension(format!("{}.tmp", nonce()?));
    let result = (|| -> Result<WorkingImage> {
        let mut command = std::process::Command::new(media::worker_executable()?);
        command
            .arg("--develop-base")
            .arg(&request.source)
            .arg(&temp)
            .arg(bound.to_string());
        let info: BaseInfo = media::run_worker(command, Some(&temp), cancelled)?;
        ensure!(!cancelled(), "現像対象が変更されました。");
        // Validate the complete replacement before publishing over a damaged cache.
        let image = engine::read_base(&temp, info.clone())?;
        std::fs::rename(&temp, &request.base)?;
        std::fs::write(&request.base_info, serde_json::to_vec(&info)?)?;
        Ok(image)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    remember_base(result?, request, bound, resident, retain, cancelled)
}
async fn preview_inner(
    state: &AppState,
    develop: &DevelopState,
    id: i64,
    settings: Settings,
    bound: u32,
) -> Reply<DevelopedPreview> {
    let _foreground = state.library.updates.foreground();
    let generation = develop.generation.clone();
    let version = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let (db, cache, renderer) = (
        state.db.clone(),
        state.cache.clone(),
        state.renderer.clone(),
    );
    let (request, hit) = work({
        let settings = settings.clone();
        move || {
            let request = request(db, cache, renderer, id, &settings, bound)?;
            let hit = if bound == 384 { None } else { cached(&request) };
            Ok((request, hit))
        }
    })
    .await?;
    if let Some(hit) = hit {
        // A cached display still needs a resident linear base for the first slider gesture.
        if let Ok(permit) = develop.jobs.clone().try_acquire_owned() {
            let resident = develop.resident.clone();
            let pool = develop.pool.clone();
            let _ = work(move || {
                let _permit = permit;
                let cancelled = || generation.load(Ordering::SeqCst) != version;
                pool.install(|| cached_base(&request, bound, &resident, true, &cancelled))
            })
            .await;
        }
        return Ok(hit);
    }
    if matches!(bound, 384 | 1600) {
        let resident = develop.resident.clone();
        let pool = develop.pool.clone();
        let request = request.clone();
        let generation = generation.clone();
        let settings = settings.clone();
        let frame = work(move || {
            pool.install(|| {
                let cancelled = || generation.load(Ordering::SeqCst) != version;
                let Some(base) = cached_base(&request, bound, &resident, true, &cancelled)? else {
                    return Ok(None);
                };
                // Display-size updates retain detail without waiting for the disk cache or full renders.
                let rendered = render_frame(&base, &settings, bound, &cancelled)?;
                let mut png = Vec::new();
                advanced_explorer_media::encode_preview(&mut png, &rendered.image)?;
                ensure!(!cancelled(), "現像対象が変更されました。");
                let mut info = base.info.clone();
                engine::refresh_raw_profile(&mut info);
                Ok(Some(DevelopedPreview {
                    path: String::new(),
                    inline_image: Some(format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&png)
                    )),
                    image_bytes: png.len() as u64,
                    info: rendered.info,
                    base: info,
                }))
            })
        })
        .await?;
        if let Some(frame) = frame {
            return Ok(frame);
        }
    }
    let jobs = &develop.jobs;
    let _permit = jobs
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| e.to_string())?;
    let resident = develop.resident.clone();
    let pool = develop.pool.clone();
    let copied = state.clone();
    work(move || {
        let cancelled = || generation.load(Ordering::SeqCst) != version;
        let reusable =
            pool.install(|| cached_base(&request, bound, &resident, true, &cancelled))?;
        let reserve = match &reusable {
            // RGBA8 output plus PNG and metadata overhead; the decoded base is already accounted for.
            Some(base) => u64::from(base.info.width) * u64::from(base.info.height) * 5 + 1_000_000,
            None => match bound {
                0 => resident
                    .lock()
                    .unwrap()
                    .as_ref()
                    .filter(|base| base.source_key == request.source_key)
                    .map(|base| {
                        // The fit preview already knows the original dimensions: RGBA32F base + PNG.
                        let pixels = u64::from(base.image.info.source_width)
                            * u64::from(base.image.info.source_height);
                        pixels
                            .saturating_mul(21)
                            .saturating_add(1_000_000)
                            .min(900_000_000)
                    })
                    .unwrap_or(900_000_000),
                4096 => 320_000_000,
                _ => 64_000_000,
            },
        };
        let operation = || {
            pool.install(|| {
                ensure!(!cancelled(), "現像対象が変更されました。");
                if let Some(hit) = cached(&request) {
                    return Ok(hit);
                }
                let base = match reusable {
                    Some(base) => base,
                    None => load_base(&request, bound, &resident, true, &cancelled)?,
                };
                let rendered = render_frame(&base, &settings, bound, &cancelled)?;
                ensure!(!cancelled(), "現像対象が変更されました。");
                let temp = request.image.with_extension(format!("{}.tmp", nonce()?));
                let result = (|| -> Result<DevelopedPreview> {
                    advanced_explorer_media::write_preview(&temp, &rendered.image)?;
                    ensure!(!cancelled(), "現像対象が変更されました。");
                    std::fs::rename(&temp, &request.image)?;
                    let mut info = base.info.clone();
                    engine::refresh_raw_profile(&mut info);
                    let value = DevelopedPreview {
                        path: request.image.to_string_lossy().into_owned(),
                        inline_image: None,
                        image_bytes: std::fs::metadata(&request.image)?.len(),
                        info: rendered.info,
                        base: info,
                    };
                    std::fs::write(&request.metadata, serde_json::to_vec(&value)?)?;
                    Ok(value)
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(&temp);
                }
                result
            })
        };
        copied.library.with_work_space(&copied, reserve, operation)
    })
    .await
}
#[tauri::command]
pub async fn develop_preview(
    state: State<'_, AppState>,
    develop: State<'_, DevelopState>,
    id: i64,
    settings: Settings,
    bound: u32,
) -> Reply<DevelopedPreview> {
    preview_inner(&state, &develop, id, settings, bound).await
}
pub fn library_pixels(
    state: &AppState,
    id: i64,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(image::RgbImage, Preview)> {
    let request = request(
        state.db.clone(),
        state.cache.clone(),
        state.renderer.clone(),
        id,
        settings,
        1600,
    )?;
    let temporary = tempfile::tempdir_in(&state.cache)?;
    let recipe = temporary.path().join("settings.json");
    let output = temporary.path().join("library.png");
    std::fs::write(&recipe, serde_json::to_vec(settings)?)?;
    // A cancellable short-lived worker cannot evict or block the editor's resident RAW session.
    let result = agent_preview_with_source(&request.source, &output, &recipe, 1600, cancelled)?;
    let info = result.info;
    let pixels = image::open(&output)?.into_rgb8();
    Ok((
        pixels,
        Preview {
            path: String::new(),
            width: result.source_width as usize,
            height: result.source_height as usize,
            render_width: info.width as usize,
            render_height: info.height as usize,
        },
    ))
}
#[tauri::command]
pub async fn export_develop(
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    id: i64,
    settings: Settings,
    format: String,
    options: Option<engine::ExportOptions>,
) -> Reply<Option<String>> {
    let db = state.db.clone();
    work(move || {
        settings.validate()?;
        let options = options.unwrap_or_default();
        options.validate()?;
        let extension = match format.as_str() {
            "jpeg" => "jpg",
            "png" => "png",
            "tiff" => "tif",
            _ => anyhow::bail!("Unsupported export format"),
        };
        let source = Catalog::open(&db)?.source_path(id)?;
        let name = format!(
            "{}-edited.{extension}",
            source.file_stem().unwrap_or_default().to_string_lossy()
        );
        let Some(destination) = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter(crate::preferences::text(&app, "写真", "Photos"), &[extension])
            .set_file_name(name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let destination = destination
            .into_path()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        ensure!(
            !destination.exists(),
            "既存ファイルは上書きしません。別の名前を指定してください。"
        );
        crate::batch_export::export_to(
            &source,
            &settings,
            &format,
            &options,
            &destination,
            &|| false,
        )?;
        Ok(Some(destination.to_string_lossy().into_owned()))
    })
    .await
}
/// Worker flags are handled before initializing Tauri, on every supported OS.
pub fn agent_preview(
    source: &Path,
    output: &Path,
    recipe: &Path,
    bound: u32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<RenderInfo> {
    Ok(agent_preview_with_source(source, output, recipe, bound, cancelled)?.info)
}
#[derive(serde::Serialize, serde::Deserialize)]
struct AgentPreview {
    #[serde(flatten)]
    info: RenderInfo,
    source_width: u32,
    source_height: u32,
}
fn agent_preview_with_source(
    source: &Path,
    output: &Path,
    recipe: &Path,
    bound: u32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<AgentPreview> {
    ensure!(!cancelled(), "Preview cancelled");
    if !cfg!(test) || std::env::var_os("AE_TEST_EXECUTABLE").is_some() {
        let mut command = std::process::Command::new(media::worker_executable()?);
        command
            .arg("--develop-agent-preview")
            .arg(source)
            .arg(output)
            .arg(recipe)
            .arg(bound.to_string());
        media::run_worker(command, Some(output), cancelled)
    } else {
        let settings: Settings = serde_json::from_slice(&std::fs::read(recipe)?)?;
        let base = engine::prepare(source, bound)?;
        let rendered = engine::render(&base, &settings, cancelled)?;
        image::DynamicImage::ImageRgba8(rendered.image)
            .to_rgb8()
            .save_with_format(output, image::ImageFormat::from_path(output)?)?;
        Ok(AgentPreview {
            info: rendered.info,
            source_width: base.info.source_width,
            source_height: base.info.source_height,
        })
    }
}

pub fn run_worker() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let Some(flag) = args.first().and_then(|value| value.to_str()) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    if flag == "--rtengine-resident" {
        if let Err(error) = crate::rtengine_worker::run() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return true;
    }
    if ![
        "--develop-base",
        "--develop-export",
        "--develop-agent-preview",
    ]
    .contains(&flag)
    {
        return false;
    }
    let result = (|| -> Result<()> {
        let threads = if cfg!(target_os = "macos") && flag == "--develop-export" {
            std::thread::available_parallelism().map_or(4, |n| n.get().min(8))
        } else {
            4
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        pool.install(|| -> Result<()> {
            match flag {
                "--develop-base" => {
                    ensure!(
                        (3..=4).contains(&args.len()),
                        "Invalid development arguments"
                    );
                    let bound = args
                        .get(3)
                        .map(|value| value.to_str().unwrap_or("").parse::<u32>())
                        .transpose()?
                        .unwrap_or(4096);
                    let base = engine::prepare(Path::new(&args[1]), bound)?;
                    engine::write_base(Path::new(&args[2]), &base)?;
                    println!("{}", serde_json::to_string(&base.info)?);
                }
                "--develop-agent-preview" => {
                    ensure!(args.len() == 5, "Invalid preview arguments");
                    let bound: u32 = args[4].to_str().unwrap_or("").parse()?;
                    ensure!([384, 1600].contains(&bound), "Invalid preview size");
                    let settings: Settings = serde_json::from_slice(&std::fs::read(&args[3])?)?;
                    let base = engine::prepare(Path::new(&args[1]), bound)?;
                    let rendered = engine::render(&base, &settings, &|| false)?;
                    image::DynamicImage::ImageRgba8(rendered.image)
                        .to_rgb8()
                        .save_with_format(&args[2], image::ImageFormat::from_path(&args[2])?)?;
                    println!(
                        "{}",
                        serde_json::to_string(&AgentPreview {
                            info: rendered.info,
                            source_width: base.info.source_width,
                            source_height: base.info.source_height
                        })?
                    );
                }
                _ => {
                    ensure!((5..=6).contains(&args.len()), "Invalid export arguments");
                    let settings: Settings = serde_json::from_slice(&std::fs::read(&args[3])?)?;
                    settings.validate()?;
                    let options = args
                        .get(5)
                        .map(|value| serde_json::from_str(value.to_str().unwrap_or("")))
                        .transpose()?
                        .unwrap_or_default();
                    engine::export_source_with_options(
                        Path::new(&args[1]),
                        &settings,
                        Path::new(&args[2]),
                        args[4].to_str().unwrap_or(""),
                        &options,
                    )?;
                    println!("{{\"ok\":true}}");
                }
            }
            Ok(())
        })
    })();
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
    true
}

#[cfg(test)]
mod tests {
    include!("develop_benchmark.rs");
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn explicit_library_rotation_updates_active_photo_without_releasing_other_edits() {
        let (_directory, state, id) = fixture();
        let mut catalog = Catalog::open(&state.db).unwrap();
        let source = catalog.source_path(id).unwrap();
        let original = std::fs::read(&source).unwrap();
        state.library.generate(&state, id, &|| false, true).unwrap();
        state.library.updates.active.store(id, Ordering::SeqCst);
        for (revision, turns, dimensions) in [(0, 1, (37, 67)), (1, 0, (67, 37)), (2, 3, (37, 67))] {
            let settings = Settings { quarter_turns: turns, ..Settings::default() };
            catalog.save_develop_recipe(id, &settings.catalog_recipe().unwrap(), revision).unwrap();
            assert!(!state.library.generate(&state, id, &|| false, true).unwrap());
            assert!(state.library.generate_explicit(&state, id).unwrap());
            let preview = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
            assert_eq!((preview.render_width, preview.render_height), dimensions);
            assert_eq!(catalog.library_preview(id).unwrap().unwrap().stamp, catalog.preview_stamp(id).unwrap());
            assert_eq!(catalog.preview_update(id).unwrap(), Some((revision + 1, false)));
            assert!(catalog.pending_preview_updates().unwrap().is_empty());
        }
        assert_eq!(std::fs::read(source).unwrap(), original, "rotation must not modify the original");
    }

    #[test]
    fn saving_keeps_library_images_until_another_photo_is_selected() {
        let (_directory, state, id) = fixture();
        state.library.generate(&state, id, &|| false, true).unwrap();
        let catalog = Catalog::open(&state.db).unwrap();
        let old = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        let old_pixels = std::fs::read(&old.path).unwrap();
        let settings = Settings {
            exposure: 1.,
            ..Settings::default()
        };
        // Saving must finish even while preview generation owns its workspace lock.
        let saved = state
            .library
            .with_work_space(&state, 0, || {
                let (send, receive) = std::sync::mpsc::channel();
                let state = state.clone();
                let settings = settings.clone();
                std::thread::spawn(move || {
                    let result = save_inner(
                        &state,
                        id,
                        settings.clone(),
                        0,
                        vec![EditStep {
                            settings,
                            label: "露光".into(),
                        }],
                        Some(vec![Settings::default()]),
                    );
                    let _ = send.send(result);
                });
                Ok(receive.recv_timeout(std::time::Duration::from_secs(2)))
            })
            .unwrap()
            .expect("autosave waited for the image worker")
            .unwrap();
        assert_eq!(saved, 1);
        assert_eq!(catalog.preview_update(id).unwrap(), Some((1, false)));
        assert_eq!(
            state
                .library
                .lookup(&catalog, id, 1600)
                .unwrap()
                .unwrap()
                .path,
            old.path
        );
        assert_eq!(std::fs::read(&old.path).unwrap(), old_pixels);
        assert!(!state.library.generate(&state, id, &|| false, true).unwrap());
        catalog.release_preview_updates(id + 1).unwrap();
        assert!(state.library.generate(&state, id, &|| true, true).is_err());
        assert_eq!(
            state
                .library
                .lookup(&catalog, id, 1600)
                .unwrap()
                .unwrap()
                .path,
            old.path
        );
        let updated = std::sync::Mutex::new(Vec::new());
        crate::deferred_previews::drain(&state, |id| updated.lock().unwrap().push(id)).unwrap();
        assert_eq!(*updated.lock().unwrap(), vec![id]);
        assert!(catalog.preview_update(id).unwrap().is_none());
        let new = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        assert_ne!(std::fs::read(&new.path).unwrap(), old_pixels);
        assert_eq!((new.width, new.height), (67, 37));
        let source = catalog.source_path(id).unwrap();
        let moved = source.with_extension("offline");
        std::fs::rename(&source, &moved).unwrap();
        Catalog::open(&state.db)
            .unwrap()
            .save_develop_recipe(id, "{\"version\":1,\"exposure\":2}", 1)
            .unwrap();
        catalog.release_preview_updates(id + 1).unwrap();
        assert!(
            crate::deferred_previews::drain(&state, |_| panic!("offline update published"))
                .is_err()
        );
        assert_eq!(catalog.preview_update(id).unwrap(), Some((2, true)));
        assert_eq!(
            state
                .library
                .lookup(&catalog, id, 1600)
                .unwrap()
                .unwrap()
                .path,
            new.path
        );
        std::fs::rename(moved, source).unwrap();
        crate::deferred_previews::drain(&state, |_| {}).unwrap();
        assert!(catalog.preview_update(id).unwrap().is_none());
    }

    fn preview_bytes(preview: &DevelopedPreview) -> Vec<u8> {
        match &preview.inline_image {
            Some(data) => base64::engine::general_purpose::STANDARD
                .decode(data.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap(),
            None => std::fs::read(&preview.path).unwrap(),
        }
    }

    fn fixture() -> (tempfile::TempDir, Arc<AppState>, i64) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("photos");
        let cache = directory.path().join("cache");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&cache).unwrap();
        image::RgbaImage::from_pixel(67, 37, image::Rgba([80, 120, 160, 255]))
            .save(root.join("photo.png"))
            .unwrap();
        let db = directory.path().join("catalog.db");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Default::default(), 1, None).unwrap().items[0].id;
        let state = Arc::new(AppState {
            library: Arc::new(crate::library_previews::Store::new(&db).unwrap()),
            db,
            renderer: media::Renderer::new(&cache).unwrap(),
            cache,
            scanning: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            thumbs: Arc::new(tokio::sync::Semaphore::new(1)),
            previews: Arc::new(tokio::sync::Semaphore::new(1)),
            preview_version: Arc::new(AtomicUsize::new(0)),
            metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
            metadata_version: Arc::new(AtomicUsize::new(0)),
        });
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            1600,
        )
        .unwrap();
        let base = engine::prepare(&request.source, 4096).unwrap();
        engine::write_base(&request.base, &base).unwrap();
        std::fs::write(&request.base_info, serde_json::to_vec(&base.info).unwrap()).unwrap();
        (directory, state, id)
    }

    #[test]
    #[ignore = "requires AE_PREVIEW_PHOTO and packaged AE_TEST_EXECUTABLE"]
    fn raw_library_pixels_match_editor() {
        let (_directory, state, _) = fixture();
        let original = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap());
        let source = state
            .db
            .parent()
            .unwrap()
            .join("photos")
            .join(original.file_name().unwrap());
        std::fs::copy(&original, &source).unwrap();
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog
            .page(
                &advanced_explorer::Query {
                    text: original.file_name().unwrap().to_string_lossy().into_owned(),
                    ..Default::default()
                },
                1,
                None,
            )
            .unwrap()
            .items[0]
            .id;
        let settings = Settings::default();
        let (library, _) = library_pixels(&state, id, &settings, &|| false).unwrap();
        let editor = tauri::async_runtime::block_on(preview_inner(
            &state,
            &DevelopState::new().unwrap(),
            id,
            settings,
            1600,
        ))
        .unwrap();
        let editor = image::load_from_memory(&preview_bytes(&editor))
            .unwrap()
            .into_rgb8();
        assert_eq!(library.dimensions(), editor.dimensions());
        let error: u64 = library
            .as_raw()
            .iter()
            .zip(editor.as_raw())
            .map(|(a, b)| a.abs_diff(*b) as u64)
            .sum();
        assert_eq!(
            error, 0,
            "Library and editor must use identical developed pixels before AVIF encoding"
        );
        assert_eq!(catalog.develop_recipe(id).unwrap(), (None, 0));
    }

    #[test]
    #[ignore = "requires AE_PREVIEW_PHOTO; measures the complete desktop preview path"]
    fn slider_preview_latency() {
        let (_directory, state, _) = fixture();
        let source = PathBuf::from(std::env::var("AE_PREVIEW_PHOTO").unwrap());
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog.add_root(source.parent().unwrap()).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog
            .page(
                &advanced_explorer::Query {
                    text: source.file_name().unwrap().to_string_lossy().into_owned(),
                    ..Default::default()
                },
                1,
                None,
            )
            .unwrap()
            .items[0]
            .id;
        assert_eq!(catalog.source_path(id).unwrap(), source);
        let develop = DevelopState::new().unwrap();
        let mut measurements = Vec::new();
        for bound in [1600, 0, 384] {
            let request = request(
                state.db.clone(),
                state.cache.clone(),
                state.renderer.clone(),
                id,
                &Settings::default(),
                bound,
            )
            .unwrap();
            if bound != 384 {
                let base = engine::prepare(&source, bound).unwrap();
                engine::write_base(&request.base, &base).unwrap();
                std::fs::write(&request.base_info, serde_json::to_vec(&base.info).unwrap())
                    .unwrap();
            }
            tauri::async_runtime::block_on(preview_inner(
                &state,
                &develop,
                id,
                Settings::default(),
                bound,
            ))
            .unwrap();
            let mut elapsed = Vec::new();
            let mut render_times = Vec::new();
            for index in 0..20 {
                let settings = Settings {
                    exposure: index as f32 * 0.05,
                    contrast: 12.,
                    temperature: 20.,
                    ..Default::default()
                };
                let started = std::time::Instant::now();
                let preview = tauri::async_runtime::block_on(preview_inner(
                    &state, &develop, id, settings, bound,
                ))
                .unwrap();
                elapsed.push(started.elapsed().as_secs_f64() * 1000.);
                render_times.push(preview.info.elapsed_ms);
            }
            elapsed.sort_by(f64::total_cmp);
            render_times.sort_by(f64::total_cmp);
            measurements.push(serde_json::json!({"bound":bound, "median_ms":elapsed[10], "p95_ms":elapsed[18], "render_median_ms":render_times[10]}));
        }
        let report = serde_json::to_string_pretty(&measurements).unwrap();
        println!("{report}");
        if let Some(path) = std::env::var_os("AE_LATENCY_REPORT") {
            std::fs::write(path, report).unwrap();
        }
    }

    #[test]
    #[ignore = "real PNG render soak and concurrent cancellation under cache pressure"]
    fn development_soak_keeps_cache_bounded_and_recovers_after_cancellation_and_restart() {
        let (_directory, state, id) = fixture();
        let source = Catalog::open(&state.db).unwrap().source_path(id).unwrap();
        let full = std::env::var_os("AE_STRESS_FULL").is_some();
        let (width, height, renders, reserved_bytes) = if full {
            (1536, 1024, 320, 0)
        } else {
            (512, 256, 200, 985_000_000)
        };
        image::RgbaImage::from_fn(width, height, |x, y| {
            let value =
                (x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263)).wrapping_mul(1274126177);
            image::Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
        })
        .save(&source)
        .unwrap();
        let original = std::fs::read(&source).unwrap();
        let mut photo_ids = vec![id];
        if full {
            for index in 1..4 {
                std::fs::copy(&source, source.with_file_name(format!("soak-{index}.png"))).unwrap();
            }
            let mut catalog = Catalog::open(&state.db).unwrap();
            catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
            photo_ids = catalog
                .page(&Default::default(), 10, None)
                .unwrap()
                .items
                .into_iter()
                .map(|photo| photo.id)
                .collect();
        }
        let minimum_seconds = std::env::var("AE_STRESS_SECONDS")
            .map(|value| value.parse::<u64>().unwrap())
            .unwrap_or(0);
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            1600,
        )
        .unwrap();
        let base = engine::prepare(&source, 1600).unwrap();
        engine::write_base(&request.base, &base).unwrap();
        std::fs::write(&request.base_info, serde_json::to_vec(&base.info).unwrap()).unwrap();
        // The quick mode reserves space sparsely; full mode fills the cache with real PNGs.
        let reserved = state.cache.join("renderer-reserved-test-space");
        std::fs::File::create(&reserved)
            .unwrap()
            .set_len(reserved_bytes)
            .unwrap();
        let develop = Arc::new(DevelopState::new().unwrap());
        let start = std::time::Instant::now();
        let mut peak = 0;
        let mut index = 0;
        let mut library_updates = 0;
        while index < renders || start.elapsed().as_secs() < minimum_seconds {
            let photo_id = photo_ids[(index / 25) % photo_ids.len()];
            let settings = Settings {
                contrast: (index % 100_000) as f32 / 100_000. * 80. - 40.,
                ..Default::default()
            };
            let preview = tauri::async_runtime::block_on(preview_inner(
                &state, &develop, photo_id, settings, 1600,
            ))
            .unwrap();
            assert_eq!(
                image::load_from_memory(&preview_bytes(&preview))
                    .unwrap()
                    .into_rgba8()
                    .dimensions(),
                (width, height)
            );
            if full && index % 25 == 24 {
                let catalog = Catalog::open(&state.db).unwrap();
                if let Some(preview) = state.library.lookup(&catalog, photo_id, 1600).unwrap() {
                    std::fs::remove_file(preview.path).unwrap();
                }
                assert!(
                    state
                        .library
                        .generate(&state, photo_id, &|| false, true)
                        .unwrap()
                );
                library_updates += 1;
            }
            let bytes = std::fs::read_dir(&state.cache)
                .unwrap()
                .map(|entry| entry.unwrap().metadata().unwrap().len())
                .sum::<u64>();
            peak = peak.max(bytes);
            assert!(bytes <= 1_000_000_000, "working cache grew to {bytes}");
            index += 1;
            if index % 100 == 0 {
                println!(
                    "soak: {index} renders, {:.1}s, {bytes} working bytes",
                    start.elapsed().as_secs_f64()
                );
            }
        }
        let renders = index;
        let mut jobs = Vec::new();
        for index in 0..20 {
            let (copy_state, copy_develop) = (state.clone(), develop.clone());
            jobs.push(tauri::async_runtime::spawn(async move {
                preview_inner(
                    &copy_state,
                    &copy_develop,
                    id,
                    Settings {
                        exposure: index as f32 / 20.,
                        ..Default::default()
                    },
                    1600,
                )
                .await
            }));
        }
        tauri::async_runtime::block_on(async {
            for job in jobs {
                let _ = job.await.unwrap();
            }
        });
        let final_preview = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            1600,
        ))
        .unwrap();
        let expected = preview_bytes(&final_preview);
        let restarted = DevelopState::new().unwrap();
        let mut reopened = (*state).clone();
        reopened.library = Arc::new(crate::library_previews::Store::new(&state.db).unwrap());
        let again = tauri::async_runtime::block_on(preview_inner(
            &reopened,
            &restarted,
            id,
            Settings::default(),
            1600,
        ))
        .unwrap();
        assert_eq!(expected, preview_bytes(&again));
        for photo_id in &photo_ids {
            let catalog = Catalog::open(&state.db).unwrap();
            assert_eq!(
                original,
                std::fs::read(catalog.source_path(*photo_id).unwrap()).unwrap()
            );
            assert_eq!(catalog.develop_recipe(*photo_id).unwrap().1, 0);
        }
        assert_eq!(
            Catalog::open(&state.db)
                .unwrap()
                .develop_recipe(id)
                .unwrap()
                .1,
            0
        );
        let files = std::fs::read_dir(&state.cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert!(
            files
                .iter()
                .all(|path| path.extension().is_none_or(|extension| extension != "tmp"))
        );
        let rendered = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("develop-")
                    && path.extension().is_some_and(|extension| extension == "png")
            })
            .count();
        assert!(rendered < renders, "old outputs were never evicted");
        let report = serde_json::json!({"renders":renders,"photos":photo_ids.len(),"library_updates":library_updates,"dimensions":[width,height],"reserved_sparse_bytes":reserved_bytes,"concurrent_requests":20,"seconds":start.elapsed().as_secs_f64(),"peak_working_bytes":peak,"retained_pngs":rendered,"original_unchanged":true,"recipe_unchanged":true,"restart_pixels_identical":true,"temporary_files":0});
        println!("{report}");
        if let Some(path) = std::env::var_os("AE_STRESS_REPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }

    #[test]
    fn truncated_render_cache_is_rebuilt_without_changing_saved_settings() {
        let (_directory, state, id) = fixture();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            4096,
        )
        .unwrap();
        let base = engine::prepare(&request.source, 4096).unwrap();
        engine::write_base(&request.base, &base).unwrap();
        std::fs::write(&request.base_info, serde_json::to_vec(&base.info).unwrap()).unwrap();
        let develop = DevelopState::new().unwrap();
        let first = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            4096,
        ))
        .unwrap();
        let expected = std::fs::read(&first.path).unwrap();
        std::fs::write(&first.path, b"broken PNG").unwrap();
        let rebuilt = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            4096,
        ))
        .unwrap();
        assert_eq!(rebuilt.path, first.path);
        assert_eq!(std::fs::read(rebuilt.path).unwrap(), expected);
        assert_eq!(
            Catalog::open(&state.db)
                .unwrap()
                .develop_recipe(id)
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn full_size_edits_reuse_a_recent_base_without_reserving_another_decode() {
        let (_directory, state, id) = fixture();
        let source = Catalog::open(&state.db).unwrap().source_path(id).unwrap();
        let original = std::fs::read(&source).unwrap();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            0,
        )
        .unwrap();
        let base = engine::prepare(&source, 0).unwrap();
        engine::write_base(&request.base, &base).unwrap();
        std::fs::write(&request.base_info, serde_json::to_vec(&base.info).unwrap()).unwrap();
        // Match the disk footprint of an 18 MP original without allocating that image in the test.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&request.base)
            .unwrap()
            .set_len(286_000_000)
            .unwrap();
        let develop = DevelopState::new().unwrap();
        let mut previous = None;
        for exposure in [0., 0.5, 1.] {
            let result = tauri::async_runtime::block_on(preview_inner(
                &state,
                &develop,
                id,
                Settings {
                    exposure,
                    ..Default::default()
                },
                0,
            ))
            .unwrap();
            let pixels = image::open(&result.path).unwrap().into_rgba8();
            assert_eq!(pixels.dimensions(), (67, 37));
            assert_ne!(previous.as_ref(), Some(pixels.as_raw()));
            previous = Some(pixels.into_raw());
            assert!(request.base.exists());
        }
        let occupied = state.cache.join("recent-other-preview.png");
        std::fs::File::create(&occupied)
            .unwrap()
            .set_len(999_500_000)
            .unwrap();
        let error = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings {
                exposure: 2.,
                ..Default::default()
            },
            0,
        ))
        .err()
        .expect("the working storage limit must still be enforced");
        assert!(error.contains("作業領域"), "{error}");
        assert!(occupied.exists());
        assert_eq!(original, std::fs::read(&source).unwrap());
        assert_eq!(
            Catalog::open(&state.db)
                .unwrap()
                .develop_recipe(id)
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn interactive_base_is_reused_without_evicting_the_full_preview() {
        let (_directory, state, id) = fixture();
        let make_request = |bound| {
            request(
                state.db.clone(),
                state.cache.clone(),
                state.renderer.clone(),
                id,
                &Settings::default(),
                bound,
            )
            .unwrap()
        };
        let full_request = make_request(1600);
        let draft_request = make_request(384);
        let resident = Mutex::new(None);
        let full = load_base(&full_request, 1600, &resident, true, &|| false).unwrap();
        let draft = load_base(&draft_request, 384, &resident, true, &|| false).unwrap();
        assert!(draft.info.width.max(draft.info.height) <= 384);
        assert!(
            !draft_request.base.exists(),
            "the draft must reuse the decoded original"
        );
        let again = load_base(&draft_request, 384, &resident, true, &|| false).unwrap();
        assert!(Arc::ptr_eq(&draft, &again));
        assert!(Arc::ptr_eq(
            &full,
            &load_base(&full_request, 1600, &resident, true, &|| false).unwrap()
        ));
        let mut changed_source = draft_request;
        changed_source.source_key = PathBuf::from("different-original");
        assert!(load_base(&changed_source, 384, &resident, true, &|| true).is_err());
    }

    #[test]
    fn interactive_frames_are_lossless_and_do_not_wait_for_disk_cache() {
        let (_directory, state, id) = fixture();
        let develop = Arc::new(DevelopState::new().unwrap());
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            1600,
        )
        .unwrap();
        load_base(&request, 1600, &develop.resident, true, &|| false).unwrap();
        let files = || {
            let mut entries: Vec<_> = std::fs::read_dir(&state.cache)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.path(), entry.metadata().unwrap().len())
                })
                .collect();
            entries.sort();
            entries
        };
        let before = files();
        let _busy_full_render =
            tauri::async_runtime::block_on(develop.jobs.clone().acquire_owned()).unwrap();
        let (locked_send, locked_receive) = std::sync::mpsc::channel();
        let (release_send, release_receive) = std::sync::mpsc::channel();
        let copy = state.clone();
        let writer = std::thread::spawn(move || {
            copy.library
                .with_work_space(&copy, 0, || {
                    locked_send.send(()).unwrap();
                    release_receive
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    Ok(())
                })
                .unwrap()
        });
        locked_receive.recv().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let copy = state.clone();
        let copied_develop = develop.clone();
        let renderer = std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                for index in 0..24 {
                    let bound = if index % 2 == 0 { 1600 } else { 384 };
                    let settings = Settings {
                        exposure: index as f32 * 0.05,
                        ..Default::default()
                    };
                    let frame = tauri::async_runtime::block_on(preview_inner(
                        &copy,
                        &copied_develop,
                        id,
                        settings.clone(),
                        bound,
                    ))
                    .map_err(anyhow::Error::msg)?;
                    let png = base64::engine::general_purpose::STANDARD.decode(
                        frame
                            .inline_image
                            .unwrap()
                            .strip_prefix("data:image/png;base64,")
                            .unwrap(),
                    )?;
                    let pixels = image::load_from_memory(&png)?.into_rgba8();
                    let resident = copied_develop.resident.lock().unwrap();
                    let resident = resident.as_ref().unwrap();
                    let base = if bound == 384 {
                        resident.interactive.as_ref().unwrap()
                    } else {
                        &resident.image
                    };
                    assert_eq!(pixels, engine::render(base, &settings, &|| false)?.image);
                }
                Ok(())
            })();
            send.send(result).unwrap();
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        release_send.send(()).unwrap();
        writer.join().unwrap();
        renderer.join().unwrap();
        result
            .expect("interactive frames waited for the disk cache lock")
            .unwrap();
        assert_eq!(files(), before, "drag frames must not grow the disk cache");
        std::fs::remove_file(&request.source).unwrap();
        assert!(
            tauri::async_runtime::block_on(preview_inner(
                &state,
                &develop,
                id,
                Settings::default(),
                384
            ))
            .is_err()
        );
    }

    #[test]
    fn background_thumbnail_keeps_foreground_resident_and_corrupt_base_is_a_miss() {
        let (_directory, state, id) = fixture();
        let request = request(
            state.db.clone(),
            state.cache.clone(),
            state.renderer.clone(),
            id,
            &Settings::default(),
            1600,
        )
        .unwrap();
        let resident = Mutex::new(None);
        let foreground = load_base(&request, 1600, &resident, true, &|| false).unwrap();
        load_base(&request, 384, &resident, false, &|| false).unwrap();
        std::fs::write(&request.base, b"interrupted cache write").unwrap();
        assert!(read_cached_base(&request).is_none());
        let hit = load_base(&request, 1600, &resident, true, &|| false).unwrap();
        assert!(Arc::ptr_eq(&foreground, &hit));
    }

    #[test]
    fn foreground_preview_does_not_wait_for_thumbnail_slot() {
        let (_directory, state, id) = fixture();
        let develop = Arc::new(DevelopState::new().unwrap());
        let permit = tauri::async_runtime::block_on(state.thumbs.clone().acquire_owned()).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = send.send(tauri::async_runtime::block_on(preview_inner(
                &state,
                &develop,
                id,
                Settings::default(),
                1600,
            )));
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(2));
        drop(permit);
        worker.join().unwrap();
        assert!(
            result
                .expect("foreground preview waited for a thumbnail")
                .is_ok()
        );
    }

    #[test]
    fn warm_development_preview_bypasses_busy_jobs_and_revalidates_source() {
        let (_directory, state, id) = fixture();
        let develop = Arc::new(DevelopState::new().unwrap());
        let first = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            1600,
        ))
        .unwrap();
        let cold = DevelopState::new().unwrap();
        tauri::async_runtime::block_on(preview_inner(&state, &cold, id, Settings::default(), 1600))
            .unwrap();
        let resident = cold.resident.lock().unwrap();
        assert_eq!(resident.as_ref().unwrap().bound, 1600);
        assert!(
            resident.as_ref().unwrap().interactive.is_none(),
            "display-size editing must not preallocate a draft"
        );
        drop(resident);
        let permit = tauri::async_runtime::block_on(develop.jobs.clone().acquire_owned()).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let (copy_state, copy_develop) = (state.clone(), develop.clone());
        let worker = std::thread::spawn(move || {
            let _ = send.send(tauri::async_runtime::block_on(preview_inner(
                &copy_state,
                &copy_develop,
                id,
                Settings::default(),
                1600,
            )));
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(2));
        drop(permit);
        worker.join().unwrap();
        assert_eq!(
            preview_bytes(
                &result
                    .expect("warm preview queued behind image work")
                    .unwrap()
            ),
            preview_bytes(&first)
        );
        std::fs::remove_file(Catalog::open(&state.db).unwrap().source_path(id).unwrap()).unwrap();
        assert!(
            tauri::async_runtime::block_on(preview_inner(
                &state,
                &develop,
                id,
                Settings::default(),
                1600,
            ))
            .is_err()
        );
    }

    #[test]
    #[ignore = "requires AE_TEST_EXECUTABLE pointing to the built desktop worker"]
    fn original_worker_is_used_after_import_and_edits_refresh_library_images() {
        let (_dir, state, id) = fixture();
        assert!(std::env::var_os("AE_TEST_EXECUTABLE").is_some());
        let source = Catalog::open(&state.db).unwrap().source_path(id).unwrap();
        let original = std::fs::read(&source).unwrap();
        let report = crate::library_previews::prepare(&state, None, |_| {}).unwrap();
        assert_eq!(report.generated, 1);
        let catalog = Catalog::open(&state.db).unwrap();
        let library = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        std::fs::write(&library.path, b"not development input").unwrap();
        let develop = DevelopState::new().unwrap();
        tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            1600,
        ))
        .unwrap();
        std::fs::File::create(state.cache.join("recent-other-preview.png"))
            .unwrap()
            .set_len(200_000_000)
            .unwrap();
        let full = tauri::async_runtime::block_on(preview_inner(
            &state,
            &develop,
            id,
            Settings::default(),
            0,
        ))
        .unwrap();
        assert!(full.path.ends_with(".png"));
        assert_ne!(full.path, library.path);
        assert_eq!((full.base.source_width, full.base.source_height), (67, 37));
        assert_eq!((full.info.width, full.info.height), (67, 37));
        let settings = Settings {
            exposure: 1.,
            ..Default::default()
        };
        let mut catalog = Catalog::open(&state.db).unwrap();
        catalog
            .save_develop_recipe(id, &settings.catalog_recipe().unwrap(), 0)
            .unwrap();
        state.library.generate(&state, id, &|| false, true).unwrap();
        let edited = state.library.lookup(&catalog, id, 1600).unwrap().unwrap();
        assert_ne!(edited.path, library.path);
        assert_eq!(original, std::fs::read(&source).unwrap());
        std::fs::remove_file(&source).unwrap();
        assert!(state.library.lookup(&catalog, id, 1600).unwrap().is_some());
        assert!(
            tauri::async_runtime::block_on(preview_inner(&state, &develop, id, settings, 0))
                .is_err()
        );
    }
}
