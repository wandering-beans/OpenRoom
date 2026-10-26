//! Persistent, isolated native rendering. Drag frames never wait behind full renders.
use advanced_explorer_develop::{
    self as engine, BaseInfo, RenderInfo, RenderedImage, Settings, WorkingImage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Request {
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
    Rendered(RenderedImage),
}
type Frame = Result<Response>;
struct Worker {
    child: Child,
    input: UnixStream,
    output: mpsc::Receiver<Frame>,
    reader: Option<std::thread::JoinHandle<()>>,
    loaded: Option<(PathBuf, u32, u64, std::time::SystemTime)>,
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
    if let Some(info) = header.base {
        ensure!(header.info.is_none(), "Conflicting rtengine frame metadata");
        return read_base(reader, info, header.bytes)
            .map(|base| Response::Prepared(Box::new(base)));
    }
    let info = header.info.context("Missing rtengine frame metadata")?;
    let pixels = u64::from(info.width) * u64::from(info.height);
    ensure!(
        pixels > 0 && pixels <= 64_000_000 && header.bytes as u64 == pixels * 4,
        "Invalid rtengine frame dimensions"
    );
    let mut bytes = vec![0; header.bytes];
    reader.read_exact(&mut bytes)?;
    let image = image::RgbaImage::from_raw(info.width, info.height, bytes)
        .context("Invalid rtengine frame")?;
    Ok(Response::Rendered(RenderedImage { image, info }))
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
    fn start() -> Result<Self> {
        let (input, child_input) = UnixStream::pair()?;
        input.set_write_timeout(Some(Duration::from_millis(10)))?;
        let child_input: OwnedFd = child_input.into();
        let mut child = Command::new(crate::media::worker_executable()?)
            .arg("--rtengine-resident")
            .env("OMP_NUM_THREADS", "4")
            .env("OMP_MAX_ACTIVE_LEVELS", "1")
            .stdin(Stdio::from(child_input))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
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
        Response::Rendered(frame) => Ok(frame),
        Response::Prepared(_) => anyhow::bail!("Unexpected prepared frame"),
    }
}

pub fn prepare(
    source: &Path,
    bound: u32,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<WorkingImage> {
    if cfg!(test) && std::env::var_os("AE_TEST_EXECUTABLE").is_none() {
        return engine::prepare(source, bound);
    }
    let source = source.canonicalize()?;
    match request(
        &mut Request {
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
        Response::Rendered(_) => anyhow::bail!("Unexpected rendered frame"),
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
    if worker.is_none() {
        *worker = Some(Worker::start()?);
    }
    let result = worker
        .as_mut()
        .unwrap()
        .request(request, working, cancelled);
    if result.is_err() {
        *worker = None;
    }
    result
}

pub fn run() -> Result<()> {
    let threads = std::thread::available_parallelism()
        .map_or(4, usize::from)
        .min(8);
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
                let base = match transferred {
                    Some(base) => base,
                    None => engine::prepare(&request.source, request.bound)?,
                };
                resident = Some((key, engine::PreviewRenderer::new(base)));
            }
            if request.prepare {
                return Ok(None);
            }
            resident.as_mut().unwrap().1.render(&request.settings, &|| false).map(Some)
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
                    base: None,
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
        let base = prepare(&source, 384, &|| false).unwrap();
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
            base: None,
            info: Some(info),
            error: None,
            bytes: 8,
        })
        .unwrap();
        bytes.extend_from_slice(b"\n12345678");
        let Response::Rendered(frame) = read_frame(&mut std::io::Cursor::new(bytes)).unwrap()
        else {
            panic!("Expected rendered frame");
        };
        assert_eq!(frame.image.dimensions(), (2, 1));
    }
}
