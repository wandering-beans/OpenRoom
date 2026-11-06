//! Rendering boundary. Search, caching and UI never depend on native frameworks.
use crate::Preview;
use std::path::Path;
pub fn worker_executable() -> std::io::Result<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("AE_TEST_EXECUTABLE") {
        return Ok(path.into());
    }
    std::env::current_exe()
}
#[cfg(target_os = "macos")]
mod macos;

#[derive(Clone)]
pub struct Renderer {
    #[cfg(target_os = "macos")]
    native: macos::Renderer,
}
impl Renderer {
    pub fn new(_cache: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            #[cfg(target_os = "macos")]
            native: macos::Renderer::new(_cache)?,
        })
    }
    pub fn cache_key(&self) -> String {
        #[cfg(target_os = "macos")]
        {
            format!("{}-media-v4", self.native.cache_key())
        }
        #[cfg(not(target_os = "macos"))]
        {
            "portable-image-media-v4".into()
        }
    }
    pub fn render(
        &self,
        source: &Path,
        destination: &Path,
        bound: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> anyhow::Result<Preview> {
        anyhow::ensure!(!cancelled(), "表示対象が変更されました。");
        let ext = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        anyhow::ensure!(
            advanced_explorer::supported(&ext),
            "写真ライブラリの対応形式ではありません。"
        );
        #[cfg(target_os = "macos")]
        if ext == "dng" {
            return self.native.render(source, destination, bound, cancelled);
        }
        if advanced_explorer::RAW_EXTENSIONS.contains(&ext.as_str())
            || ["heic", "heif", "hif", "tif", "tiff", "avif", "psd", "psb"].contains(&ext.as_str())
        {
            #[cfg(test)]
            if std::env::var_os("AE_TEST_EXECUTABLE").is_none() {
                let info = advanced_explorer_media::render(source, destination, bound)?;
                let mut value = serde_json::to_value(info)?;
                value["path"] = "".into();
                return Ok(serde_json::from_value(value)?);
            }
            let mut command = std::process::Command::new(worker_executable()?);
            command
                .arg("--render-media")
                .arg(source)
                .arg(destination)
                .arg(bound.to_string());
            let mut value: serde_json::Value = run_worker(command, Some(destination), cancelled)?;
            {
                value["path"] = "".into();
                return Ok(serde_json::from_value(value)?);
            }
        }
        #[cfg(target_os = "macos")]
        {
            self.native.render(source, destination, bound, cancelled)
        }
        #[cfg(not(target_os = "macos"))]
        {
            render_portable(source, destination, bound)
        }
    }
}

// Compiled and tested on macOS too, so the portable backend cannot silently rot.
#[cfg(any(not(target_os = "macos"), test))]
fn render_portable(source: &Path, destination: &Path, bound: u32) -> anyhow::Result<Preview> {
    use image::ImageDecoder;
    anyhow::ensure!((1..=4096).contains(&bound), "Invalid size");
    let reader = image::ImageReader::open(source)?.with_guessed_format()?;
    anyhow::ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
        ),
        "この描画経路ではPNG/JPEGのみ表示できます。"
    );
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let (width, height) = (image.width(), image.height());
    let thumbnail = image.thumbnail(bound, bound);
    advanced_explorer_media::write_preview(destination, &thumbnail.to_rgba8())?;
    Ok(Preview {
        path: String::new(),
        width: width as usize,
        height: height as usize,
        render_width: thumbnail.width() as usize,
        render_height: thumbnail.height() as usize,
    })
}

pub(super) fn run_worker<T: serde::de::DeserializeOwned>(
    mut command: std::process::Command,
    destination: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<T> {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let output = std::thread::scope(|scope| -> anyhow::Result<std::process::Output> {
        use std::io::Read;
        // Drain both pipes while the child runs; Windows pipe buffers can fill before exit.
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stdout = scope.spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr = scope.spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let start = Instant::now();
        let status = (|| -> anyhow::Result<std::process::ExitStatus> {
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status);
                }
                anyhow::ensure!(
                    !cancelled() && start.elapsed() <= Duration::from_secs(60),
                    "表示対象の変更またはタイムアウトにより処理を中止しました。"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        })();
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(path) = destination {
                let _ = std::fs::remove_file(path);
            }
        }
        let stdout = stdout
            .join()
            .map_err(|_| anyhow::anyhow!("Worker stdout reader panicked"))??;
        let stderr = stderr
            .join()
            .map_err(|_| anyhow::anyhow!("Worker stderr reader panicked"))??;
        Ok(std::process::Output {
            status: status?,
            stdout,
            stderr,
        })
    })?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Each caller supplies its own cancellation signal; selection changes do not cancel MCP reads.
pub fn read_metadata(
    source: &Path,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<advanced_explorer_media::metadata::PhotoMetadata> {
    anyhow::ensure!(!cancelled(), "Metadata request cancelled");
    #[cfg(not(test))]
    {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg("--photo-metadata").arg(source);
        run_worker(command, None, cancelled)
    }
    #[cfg(test)]
    {
        advanced_explorer_media::metadata::read(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "windows")]
    #[test]
    fn worker_drains_large_pipes_and_cancels_on_windows() {
        use std::time::{Duration, Instant};
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command",
            "[Console]::Error.Write(('e' * 262144)); [Console]::Out.Write(('\"' + ('x' * 262144) + '\"'))"]);
        let start = Instant::now();
        let response: String =
            run_worker(command, None, &|| start.elapsed() > Duration::from_secs(5)).unwrap();
        assert_eq!(response, "x".repeat(262144));

        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("partial.tmp");
        std::fs::write(&partial, b"partial output").unwrap();
        let mut command = std::process::Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]);
        let start = Instant::now();
        let result: anyhow::Result<String> = run_worker(command, Some(&partial), &|| {
            start.elapsed() > Duration::from_millis(300)
        });
        assert!(result.is_err());
        assert!(!partial.exists());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn portable_formats_dimensions_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let source = dir.path().join("source");
            image::RgbImage::from_pixel(800, 400, image::Rgb([12, 34, 56]))
                .save_with_format(&source, format)
                .unwrap();
            let before = std::fs::read(&source).unwrap();
            let destination = dir.path().join("output.tmp");
            let result = render_portable(&source, &destination, 384).unwrap();
            assert_eq!((result.width, result.height), (800, 400));
            assert_eq!((result.render_width, result.render_height), (384, 192));
            assert_eq!(
                image::ImageReader::open(&destination)
                    .unwrap()
                    .with_guessed_format()
                    .unwrap()
                    .decode()
                    .unwrap()
                    .width(),
                384
            );
            assert_eq!(before, std::fs::read(&source).unwrap());
            assert!(render_portable(&source, &destination, 0).is_err());
            assert!(render_portable(&source, &destination, 384).is_err());
            std::fs::remove_file(&destination).unwrap();
        }
        let bad = dir.path().join("broken.png");
        std::fs::write(&bad, b"broken").unwrap();
        assert!(render_portable(&bad, &dir.path().join("bad.tmp"), 384).is_err());
    }
}
