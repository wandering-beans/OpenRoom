//! Persistent, isolated native rendering. Drag frames never wait behind full renders.
use advanced_explorer_develop::{
    self as engine, BaseInfo, RenderInfo, RenderedImage, Settings, WorkingImage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
    time::Duration,
};

#[cfg(unix)]
use std::os::{fd::OwnedFd, unix::net::UnixStream as WorkerInput};
#[cfg(target_os = "windows")]
#[path = "rtengine_pipe.rs"]
mod pipe;
#[cfg(target_os = "windows")]
use pipe::WorkerInput;

#[derive(Serialize, Deserialize)]
struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera_saturation: Option<f32>,
    #[serde(default)]
    prepare: bool,
    source: PathBuf,
    bound: u32,
    settings: Settings,
    base: Option<BaseInfo>,
    bytes: usize,
}
#[derive(Serialize, Deserialize)]
struct Header {
    #[serde(default)]
    base: Option<BaseInfo>,
    info: Option<RenderInfo>,
    error: Option<String>,
    bytes: usize,
}
enum Response {
    Prepared(Box<WorkingImage>),
    Rendered(RenderedImage, Box<BaseInfo>),
}
type Frame = Result<Response>;
struct Worker {
    child: Child,
    input: WorkerInput,
    output: mpsc::Receiver<Frame>,
    reader: Option<std::thread::JoinHandle<()>>,
    loaded: Option<(PathBuf, u32, u64, std::time::SystemTime)>,
    source_id: Option<String>,
}
static INTERACTIVE: Mutex<Option<Worker>> = Mutex::new(None);
static FULL: Mutex<Option<Worker>> = Mutex::new(None);

fn read_frame(reader: &mut impl BufRead) -> Frame {
    let mut header = String::new();
    ensure!(
        (&mut *reader)
            .take(8 * 1024 * 1024 + 1)
            .read_line(&mut header)?
            > 0,
        "rtengine worker disconnected"
    );
    ensure!(
        header.len() <= 8 * 1024 * 1024 && header.ends_with('\n'),
        "Invalid rtengine frame header"
    );
    let header: Header = serde_json::from_str(&header)?;
    if let Some(error) = header.error {
        anyhow::bail!("{error}");
    }
    let base = header.base.context("Missing rtengine source metadata")?;
    let Some(info) = header.info else {
        return read_base(reader, base, header.bytes)
            .map(|base| Response::Prepared(Box::new(base)));
    };
    let pixels = u64::from(info.width) * u64::from(info.height);
    ensure!(
        pixels > 0 && pixels <= 64_000_000 && header.bytes as u64 == pixels * 4,
        "Invalid rtengine frame dimensions"
    );
    let mut bytes = vec![0; header.bytes];
    reader.read_exact(&mut bytes)?;
    let image = image::RgbaImage::from_raw(info.width, info.height, bytes)
        .context("Invalid rtengine frame")?;
    Ok(Response::Rendered(
        RenderedImage { image, info },
        Box::new(base),
    ))
}
fn read_base(reader: &mut impl Read, info: BaseInfo, bytes: usize) -> Result<WorkingImage> {
    let count = u64::from(info.width) * u64::from(info.height);
    ensure!(
        count > 0 && count <= 64_000_000 && bytes as u64 == count * 16,
        "Invalid resident input dimensions"
    );
    let mut samples = vec![0.; count as usize * 4];
    let mut buffer = [0u8; 16 * 1024];
    for chunk in samples.chunks_mut(buffer.len() / 4) {
        reader.read_exact(&mut buffer[..chunk.len() * 4])?;
        for (sample, bytes) in chunk.iter_mut().zip(buffer.as_chunks::<4>().0) {
            *sample = f32::from_le_bytes(*bytes);
        }
    }
    ensure!(
        samples
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p.iter().all(|v| v.is_finite()) && (0. ..=1.).contains(&p[3])),
        "Invalid resident samples"
    );
    let pixels = image::Rgba32FImage::from_raw(info.width, info.height, samples)
        .context("Invalid resident image")?;
    Ok(WorkingImage { pixels, info })
}
impl Worker {
    fn start(import: bool) -> Result<Self> {
        let _start = engine::timing::start("worker-start");
        #[cfg(unix)]
        let (input, child_input) = {
            let (input, child_input) = WorkerInput::pair()?;
            input.set_write_timeout(Some(Duration::from_millis(10)))?;
            (input, Stdio::from(OwnedFd::from(child_input)))
        };
        #[cfg(target_os = "windows")]
        let child_input = Stdio::piped();
        let mut child = Command::new(crate::media::worker_executable()?)
            .arg(if import {
                "--rtengine-import"
            } else {
                "--rtengine-resident"
            })
            .env("OMP_NUM_THREADS", "4")
            .env("OMP_MAX_ACTIVE_LEVELS", "1")
            .stdin(child_input)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        #[cfg(target_os = "windows")]
        let input = WorkerInput::new(
            child
                .stdin
                .take()
                .context("Missing rtengine worker stdin")?,
        );
        let mut stdout = BufReader::new(
            child
                .stdout
                .take()
                .context("Missing rtengine worker stdout")?,
        );
        let (send, output) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            loop {
                let frame = read_frame(&mut stdout);
                let failed = frame.is_err();
                if send.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            reader: Some(reader),
            loaded: None,
            source_id: None,
        })
    }
    fn request(
        &mut self,
        request: &mut Request,
        working: Option<&WorkingImage>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Frame {
        let metadata = std::fs::metadata(&request.source)?;
        let key = (
            request.source.clone(),
            request.bound,
            metadata.len(),
            metadata.modified()?,
        );
        let pixels = if let Some(working) = working.filter(|working| {
            self.loaded.as_ref() != Some(&key) && working.info.source_kind != "raw-rtengine-linear"
        }) {
            request.base = Some(working.info.clone());
            working
                .pixels
                .as_raw()
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        request.bytes = pixels.len();
        let started = std::time::Instant::now();
        let mut header = serde_json::to_vec(request)?;
        header.push(b'\n');
        #[cfg(target_os = "windows")]
        {
            ensure!(!cancelled(), "現像対象が変更されました。");
            self.input.send(header, pixels)?;
        }
        #[cfg(unix)]
        for mut bytes in [header.as_slice(), pixels.as_slice()] {
            while !bytes.is_empty() {
                ensure!(!cancelled(), "現像対象が変更されました。");
                ensure!(
                    started.elapsed() < Duration::from_secs(120),
                    "rtengine worker timed out"
                );
                match self.input.write(bytes) {
                    Ok(0) => anyhow::bail!("rtengine worker disconnected"),
                    Ok(written) => bytes = &bytes[written..],
                    Err(error)
                        if [
                            std::io::ErrorKind::WouldBlock,
                            std::io::ErrorKind::TimedOut,
                            std::io::ErrorKind::Interrupted,
                        ]
                        .contains(&error.kind()) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        loop {
            ensure!(!cancelled(), "現像対象が変更されました。");
            ensure!(
                started.elapsed() < Duration::from_secs(120),
                "rtengine worker timed out"
            );
            #[cfg(target_os = "windows")]
            self.input.check()?;
            match self.output.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => {
                    if result.is_ok() {
                        self.loaded = Some(key);
                    }
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Release a reader blocked on a full response channel before joining it.
        let (_, empty) = mpsc::channel();
        drop(std::mem::replace(&mut self.output, empty));
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn render(
    working: &WorkingImage,
    settings: &Settings,
    bound: u32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<RenderedImage> {
    if cfg!(test) && std::env::var_os("AE_TEST_EXECUTABLE").is_none() {
        return engine::render(working, settings, cancelled);
    }
    let Some(source) = working.info.native_source.clone() else {
        return engine::render(working, settings, cancelled);
    };
    settings.validate()?;
    match request(
        &mut Request {
            camera_saturation: working.info.camera_saturation,
            prepare: false,
            source,
            bound,
            settings: settings.clone(),
            base: None,
            bytes: 0,
        },
        Some(working),
        cancelled,
    )? {
        Response::Rendered(frame, _) => Ok(frame),
        Response::Prepared(_) => anyhow::bail!("Unexpected prepared frame"),
    }
}

pub fn prepare(
    source: &Path,
    bound: u32,
    camera_saturation: Option<f32>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<WorkingImage> {
    if cfg!(test) && std::env::var_os("AE_TEST_EXECUTABLE").is_none() {
        return engine::prepare_with_camera_saturation(source, bound, camera_saturation);
    }
    let source = source.canonicalize()?;
    match request(
        &mut Request {
            camera_saturation,
            prepare: true,
            source: source.clone(),
            bound,
            settings: Settings::default(),
            base: None,
            bytes: 0,
        },
        None,
        cancelled,
    )? {
        Response::Prepared(base) => {
            ensure!(
                base.info.native_source.as_ref() == Some(&source),
                "Unexpected prepared source"
            );
            Ok(*base)
        }
        Response::Rendered(_, _) => anyhow::bail!("Unexpected rendered frame"),
    }
}

fn request(
    request: &mut Request,
    working: Option<&WorkingImage>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Frame {
    let slot = if request.bound == 384 {
        &INTERACTIVE
    } else {
        &FULL
    };
    let mut worker = loop {
        ensure!(!cancelled(), "現像対象が変更されました。");
        match slot.try_lock() {
            Ok(worker) => break worker,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => anyhow::bail!("rtengine worker lock failed: {error}"),
        }
    };
    exchange(&mut worker, request, working, cancelled)
}

fn exchange(
    worker: &mut Option<Worker>,
    request: &mut Request,
    working: Option<&WorkingImage>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Frame {
    let source_id = crate::library_previews::file_id(&request.source);
    // A same-path replacement can preserve size and mtime in the native session's key.
    if let Some(current) = worker
        && current.loaded.as_ref().is_some_and(|(p, _, _, _)| p == &request.source)
        && current.source_id != source_id
    {
        *worker = None;
    }
    if worker.is_none() {
        *worker = Some(Worker::start(false)?);
    }
    let current = worker.as_mut().unwrap();
    current.source_id = source_id;
    let frame = current.request(request, working, cancelled).and_then(|frame| {
        ensure!(
            current.source_id == crate::library_previews::file_id(&request.source),
            "生成中に原本が変更されました。"
        );
        Ok(frame)
    });
    if frame.is_err() {
        *worker = None;
    }
    frame
}

// Warm only the selected RAW. Never queue speculative work behind an active slider request.
pub(super) fn warm_interactive(
    source: &Path,
    camera_saturation: Option<f32>,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<()> {
    if !engine::is_native_raw(source) || cancelled() {
        return Ok(());
    }
    let Ok(mut worker) = INTERACTIVE.try_lock() else {
        return Ok(());
    };
    let source = source.canonicalize()?;
    let metadata = std::fs::metadata(&source)?;
    let key = (source.clone(), 384, metadata.len(), metadata.modified()?);
    if worker.as_ref().is_some_and(|worker|
        worker.loaded.as_ref() == Some(&key)
            && worker.source_id == crate::library_previews::file_id(&source)
    ) {
        return Ok(());
    }
    let mut warmup = raw_request(source, 384, camera_saturation, settings);
    exchange(&mut worker, &mut warmup, None, cancelled)?;
    Ok(())
}

fn raw_request(source: PathBuf, bound: u32, camera_saturation: Option<f32>, settings: &Settings) -> Request {
    Request {
        camera_saturation,
        prepare: false,
        source,
        bound,
        settings: settings.clone(),
        base: None,
        bytes: 0,
    }
}

// Cold display requests publish a disk cache entry. Later adjustments can return inline.
pub(super) fn display_loaded(source: &Path) -> bool {
    let Ok(source) = source.canonicalize() else { return false };
    let Ok(metadata) = source.metadata() else { return false };
    let Ok(modified) = metadata.modified() else { return false };
    FULL.try_lock().ok().is_some_and(|slot| {
        slot.as_ref().and_then(|w| w.loaded.as_ref()) == Some(&(source, 1600, metadata.len(), modified))
    })
}

pub(super) fn raw_preview(
    source: &Path,
    bound: u32,
    camera_saturation: Option<f32>,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(RenderedImage, BaseInfo)> {
    let mut input = raw_request(source.canonicalize()?, bound, camera_saturation, settings);
    match request(&mut input, None, cancelled)? {
        Response::Rendered(frame, base) => {
            ensure!(base.native_source.as_ref() == Some(&input.source), "Unexpected RAW source");
            Ok((frame, *base))
        }
        Response::Prepared(_) => anyhow::bail!("Expected rendered RAW frame"),
    }
}

// Each batch thread owns its decoder and drops it when the batch ends.
thread_local! {
    static IMPORT: std::cell::RefCell<Option<Worker>> = const { std::cell::RefCell::new(None) };
    static IMPORT_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn begin_batch() {
    IMPORT_THREAD.set(
        (!cfg!(test) || std::env::var_os("AE_TEST_EXECUTABLE").is_some())
            && std::env::var_os("AE_BENCH_SEPARATE_WORKERS").is_none(),
    );
}

pub fn library(
    source: &Path,
    settings: &Settings,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Result<(image::RgbImage, crate::Preview, Option<f32>)>> {
    if !IMPORT_THREAD.get() {
        return None;
    }
    Some(IMPORT.with_borrow_mut(|slot| {
        let _worker = engine::timing::start("import-pipe-round-trip");
        ensure!(!cancelled(), "Library preview cancelled");
        if slot.is_none() {
            *slot = Some(Worker::start(true)?);
        }
        let reply = slot.as_mut().unwrap().request(
            &mut Request {
                camera_saturation: None,
                source: source.to_owned(),
                bound: 1600,
                settings: settings.clone(),
                prepare: false,
                base: None,
                bytes: 0,
            },
            None,
            cancelled,
        );
        let (frame, base) = match reply {
            Ok(Response::Rendered(frame, base)) => (frame, base),
            other => {
                *slot = None;
                return Err(other
                    .err()
                    .unwrap_or_else(|| anyhow::anyhow!("Missing library pixels")));
            }
        };
        ensure!(
            base.source_width > 0 && base.source_height > 0 && frame.info.width.max(frame.info.height) <= 1600,
            "Invalid library preview dimensions"
        );
        let preview = crate::Preview {
            path: String::new(),
            width: base.source_width as usize,
            height: base.source_height as usize,
            render_width: frame.info.width as usize,
            render_height: frame.info.height as usize,
        };
        Ok((
            image::DynamicImage::ImageRgba8(frame.image).into_rgb8(),
            preview,
            base.camera_saturation,
        ))
    }))
}

pub fn run(import: bool) -> Result<()> {
    let threads = if import {
        4
    } else {
        std::thread::available_parallelism()
            .map_or(4, usize::from)
            .min(8)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut resident = None;
    loop {
        let mut line = String::new();
        if (&mut input)
            .take(8 * 1024 * 1024 + 1)
            .read_line(&mut line)?
            == 0
        {
            return Ok(());
        }
        ensure!(
            line.len() <= 8 * 1024 * 1024 && line.ends_with('\n'),
            "Invalid resident request"
        );
        let request: Request = serde_json::from_str(&line)?;
        request.settings.validate()?;
        let transferred = if let Some(info) = request.base.clone() {
            ensure!(
                info.native_source.as_ref() == Some(&request.source),
                "Invalid resident source"
            );
            Some(read_base(&mut input, info, request.bytes)?)
        } else {
            ensure!(request.bytes == 0, "Unexpected resident input bytes");
            None
        };
        let result = pool.install(|| -> Result<Option<RenderedImage>> {
            ensure!(
                request.bound == 0 || request.bound <= 4096,
                "Invalid resident preview bound"
            );
            let metadata = std::fs::metadata(&request.source)?;
            let key = (
                request.source.clone(),
                request.bound,
                metadata.len(),
                metadata.modified()?,
            );
            if resident
                .as_ref()
                .is_none_or(|(previous, _)| previous != &key)
            {
                let _prepare = engine::timing::start("prepare");
                let base = match transferred {
                    Some(base) => base,
                    None => engine::prepare_with_camera_saturation(
                        &request.source, request.bound, request.camera_saturation,
                    )?,
                };
                resident = Some((key, engine::PreviewRenderer::new(base)));
            }
            if request.prepare {
                return Ok(None);
            }
            let _render = engine::timing::start("develop");
            if import {
                return engine::render(
                    resident.as_ref().unwrap().1.working(),
                    &request.settings,
                    &|| false,
                )
                .map(Some);
            }
            resident
                .as_mut()
                .unwrap()
                .1
                .render(&request.settings, &|| false)
                .map(Some)
        });
        let (header, bytes) = match result {
            Ok(None) => {
                let base = resident.as_ref().unwrap().1.working();
                serde_json::to_writer(
                    &mut output,
                    &Header {
                        base: Some(base.info.clone()),
                        info: None,
                        error: None,
                        bytes: base.pixels.as_raw().len() * 4,
                    },
                )?;
                output.write_all(b"\n")?;
                for chunk in base.pixels.as_raw().chunks(4096) {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                    output.write_all(&bytes)?;
                }
                output.flush()?;
                continue;
            }
            Ok(Some(frame)) => (
                Header {
                    base: Some(resident.as_ref().unwrap().1.working().info.clone()),
                    bytes: frame.image.as_raw().len(),
                    info: Some(frame.info),
                    error: None,
                },
                frame.image.into_raw(),
            ),
            Err(error) => (
                Header {
                    base: None,
                    bytes: 0,
                    info: None,
                    error: Some(format!("{error:#}")),
                },
                Vec::new(),
            ),
        };
        let _write = engine::timing::start("pipe-write");
        serde_json::to_writer(&mut output, &header)?;
        output.write_all(b"\n")?;
        output.write_all(&bytes)?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires AE_TEST_EXECUTABLE pointing to the shipped rtengine worker"]
    fn resident_worker_recovers_after_exit_and_cancellation() {
        assert!(std::env::var_os("AE_TEST_EXECUTABLE").is_some());
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("photo.png");
        image::RgbaImage::from_fn(96, 64, |x, y| {
            image::Rgba([x as u8 * 2, y as u8 * 3, 80, 200])
        })
        .save(&source)
        .unwrap();
        let base = prepare(&source, 384, None, &|| false).unwrap();
        assert_eq!(base.pixels, engine::prepare(&source, 384).unwrap().pixels);
        let settings = Settings {
            exposure: 0.5,
            ..Default::default()
        };
        let first = render(&base, &settings, 384, &|| false).unwrap();
        assert_eq!(
            first.image,
            engine::render(&base, &settings, &|| false).unwrap().image
        );
        {
            let mut worker = INTERACTIVE.lock().unwrap();
            let child = &mut worker.as_mut().unwrap().child;
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(render(&base, &settings, 384, &|| false).is_err());
        assert_eq!(
            render(&base, &settings, 384, &|| false).unwrap().image,
            first.image
        );
        let checks = std::sync::atomic::AtomicUsize::new(0);
        assert!(
            render(&base, &settings, 384, &|| checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                > 1)
            .is_err()
        );
        assert_eq!(
            render(&base, &settings, 384, &|| false).unwrap().image,
            first.image
        );
    }
    #[test]
    fn prepared_reader_validates_samples_dimensions_and_complete_payload() {
        let info = BaseInfo {
            auto_ca_supported: None,
            camera_saturation: None,
            native_source: Some(PathBuf::from("photo.raf")),
            raw: None,
            raster_wb_temperature: None,
            width: 1,
            height: 1,
            source_width: 1,
            source_height: 1,
            source_kind: "raw-rtengine-linear".into(),
            color_note: String::new(),
        };
        let bytes: Vec<u8> = [0.25f32, -0.1, 1.5, 1.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let base = read_base(&mut bytes.as_slice(), info.clone(), 16).unwrap();
        assert_eq!(base.pixels.as_raw(), &[0.25, -0.1, 1.5, 1.]);
        assert!(read_base(&mut &bytes[..12], info.clone(), 16).is_err());
        assert!(read_base(&mut bytes.as_slice(), info.clone(), 12).is_err());
        for values in [[f32::NAN, 0., 0., 1.], [0., 0., 0., 1.1]] {
            let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_le_bytes).collect();
            assert!(read_base(&mut bytes.as_slice(), info.clone(), 16).is_err());
        }
        let mut huge = info;
        huge.width = u32::MAX;
        huge.height = u32::MAX;
        assert!(read_base(&mut bytes.as_slice(), huge, 16).is_err());
    }
    #[test]
    #[ignore = "requires a staged AE_TEST_EXECUTABLE, AE_PREVIEW_PHOTO and AE_SECOND_PREVIEW_PHOTO"]
    fn import_worker_recovers_and_matches_a_fresh_decoder() {
        let source = PathBuf::from(std::env::var_os("AE_PREVIEW_PHOTO").unwrap());
        begin_batch();
        let pixels = || library(&source, &Settings::default(), &|| false).unwrap();
        let (expected, _, _) = pixels().unwrap();
        IMPORT.with_borrow_mut(|slot| {
            let child = &mut slot.as_mut().unwrap().child;
            child.kill().unwrap();
            child.wait().unwrap();
        });
        assert!(pixels().is_err());
        let next = PathBuf::from(std::env::var_os("AE_SECOND_PREVIEW_PHOTO").unwrap());
        assert!(std::fs::read(&next).unwrap() != std::fs::read(&source).unwrap(), "Expected two distinct RAWs");
        let (recovered, _, gain) = library(&next, &Settings::default(), &|| false).unwrap().unwrap();
        let base = engine::prepare(&next, 1600).unwrap();
        assert_eq!(gain, base.info.camera_saturation);
        let fresh = engine::render(&base, &Settings::default(), &|| false).unwrap();
        assert_eq!(recovered, image::DynamicImage::ImageRgba8(fresh.image).into_rgb8());
        assert_eq!(pixels().unwrap().0, expected);
        assert!(
            library(&source, &Settings::default(), &|| true)
                .unwrap()
                .is_err()
        );
        assert_eq!(pixels().unwrap().0, expected);
    }

    #[test]
    fn framed_reader_rejects_missing_mismatched_and_oversized_frames() {
        for bytes in [
            b"".as_slice(),
            b"{\"info\":null,\"error\":null,\"bytes\":0}\n",
            b"not-json\n",
        ] {
            assert!(read_frame(&mut std::io::Cursor::new(bytes)).is_err());
        }
        let info = RenderInfo {
            width: 2,
            height: 1,
            histogram: std::array::from_fn(|_| vec![0; 256]),
            clipped_shadows: 0,
            clipped_highlights: 0,
            elapsed_ms: 0.,
        };
        let mut bytes = serde_json::to_vec(&Header {
            base: Some(serde_json::from_value(serde_json::json!({
                "width": 2, "height": 1, "source_width": 2, "source_height": 1,
                "source_kind": "test", "color_note": ""
            })).unwrap()),
            info: Some(info),
            error: None,
            bytes: 8,
        })
        .unwrap();
        bytes.extend_from_slice(b"\n12345678");
        let Response::Rendered(frame, _) = read_frame(&mut std::io::Cursor::new(bytes)).unwrap()
        else {
            panic!("Expected rendered frame");
        };
        assert_eq!(frame.image.dimensions(), (2, 1));
    }
}
