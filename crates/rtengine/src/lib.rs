//! The pinned engine's C ABI. Sessions stay on their owning worker thread.
use anyhow::{Context, Result, ensure};
use image::Rgba32FImage;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
    ptr::NonNull,
    sync::{Mutex, OnceLock},
};

static ENGINE: Mutex<()> = Mutex::new(());
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

pub const VERSION: &str = "rtengine-5.13-19241791-v5";

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Adjustments {
    pub exposure: f32,
    pub temperature: f32,
    pub tint: f32,
    pub raw_temperature: f32,
    pub raw_tint: f32,
    pub sharpen: f32,
    pub sharpen_radius: f32,
    pub sharpen_detail: f32,
    pub sharpen_masking: f32,
    pub noise_luma: f32,
    pub noise_color: f32,
    pub scale: f32,
}
impl Default for Adjustments {
    fn default() -> Self {
        Self {
            exposure: 0.,
            temperature: 0.,
            tint: 0.,
            raw_temperature: 0.,
            raw_tint: 0.,
            sharpen: 0.,
            sharpen_radius: 1.,
            sharpen_detail: 25.,
            sharpen_masking: 0.,
            noise_luma: 0.,
            noise_color: 0.,
            scale: 1.,
        }
    }
}
#[repr(C)]
#[derive(Default)]
struct Frame {
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    temperature: f64,
    green: f64,
}
unsafe extern "C" {
    fn openroom_rt_init(data: *const c_char, cache: *const c_char) -> i32;
    fn openroom_rt_open(
        source: *const c_char,
        rgba: *const f32,
        width: u32,
        height: u32,
        bound: u32,
    ) -> *mut c_void;
    fn openroom_rt_render(
        session: *mut c_void,
        adjustments: *const Adjustments,
        frame: *mut Frame,
    ) -> i32;
    fn openroom_rt_copy(session: *mut c_void, pixels: *mut f32, count: usize) -> i32;
    fn openroom_rt_close(session: *mut c_void);
    fn openroom_rt_camera(session: *mut c_void) -> *const c_char;
    fn openroom_rt_camera_matched(session: *mut c_void) -> i32;
    fn openroom_rt_error() -> *const c_char;
    fn openroom_rt_raster_temperature(source: *const c_char) -> f64;
}

fn error() -> anyhow::Error {
    // The bridge owns a null-terminated, thread-local error string until the next call.
    anyhow::anyhow!(
        "rtengine: {}",
        unsafe { CStr::from_ptr(openroom_rt_error()) }.to_string_lossy()
    )
}
pub fn init() -> Result<()> {
    INITIALIZED
        .get_or_init(|| initialize().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))
        .copied()
}
pub fn raster_temperature(source: &Path) -> Result<Option<f32>> {
    ensure!(
        std::fs::metadata(source)?.len() <= 1024 * 1024 * 1024,
        "Metadata input exceeds 1 GiB"
    );
    init()?;
    let _guard = ENGINE
        .lock()
        .map_err(|_| anyhow::anyhow!("rtengine metadata failed"))?;
    let path = CString::new(source.to_str().context("Invalid metadata path")?)?;
    let kelvin = unsafe { openroom_rt_raster_temperature(path.as_ptr()) } as f32;
    Ok((kelvin.is_finite() && (2000. ..=60000.).contains(&kelvin)).then_some(kelvin))
}
fn initialize() -> Result<()> {
    let _guard = ENGINE
        .lock()
        .map_err(|_| anyhow::anyhow!("rtengine initialization failed"))?;
    let executable = std::env::current_exe()?;
    let bundled = executable
        .parent()
        .context("Missing executable directory")?
        .join("../Resources/rtengine");
    let data = if bundled.is_dir() {
        bundled
    } else {
        Path::new(env!("OPENROOM_RTENGINE_DATA")).to_path_buf()
    };
    ensure!(
        data.join("camconst.json").is_file(),
        "rtengine camera data are missing"
    );
    let data = CString::new(data.to_str().context("Invalid rtengine data path")?)?;
    let cache = CString::new(
        std::env::temp_dir()
            .to_str()
            .context("Invalid temporary directory")?,
    )?;
    // Called once by the serialized owner before opening any native session.
    ensure!(
        unsafe { openroom_rt_init(data.as_ptr(), cache.as_ptr()) } == 0,
        "{}",
        error()
    );
    Ok(())
}

pub struct Session {
    handle: NonNull<c_void>,
}
// Native sessions have no thread affinity. The caller serializes all engine calls,
// including destruction, and the bridge waits for its asynchronous update to finish.
unsafe impl Send for Session {}
pub struct Rendered {
    pub pixels: Rgba32FImage,
    pub source_width: u32,
    pub source_height: u32,
    pub temperature: f32,
    pub green: f32,
    pub camera: String,
    pub camera_matched: bool,
}
impl Session {
    pub fn open(source: &Path, pixels: Option<&Rgba32FImage>, bound: u32) -> Result<Self> {
        init()?;
        let _guard = ENGINE
            .lock()
            .map_err(|_| anyhow::anyhow!("rtengine session failed"))?;
        ensure!(
            bound == 0 || bound <= 4096,
            "Invalid rtengine preview bound"
        );
        let source = CString::new(source.to_str().context("Invalid photo path")?)?;
        let (pointer, width, height) = pixels.map_or((std::ptr::null(), 0, 0), |p| {
            (p.as_raw().as_ptr(), p.width(), p.height())
        });
        if let Some(pixels) = pixels {
            ensure!(
                width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 64_000_000,
                "Invalid native image dimensions"
            );
            ensure!(
                pixels.as_raw().iter().all(|v| v.is_finite()),
                "Invalid native image samples"
            );
        }
        // The bridge copies raster samples synchronously and retains no Rust pointers.
        let handle = NonNull::new(unsafe {
            openroom_rt_open(source.as_ptr(), pointer, width, height, bound)
        })
        .ok_or_else(error)?;
        Ok(Self { handle })
    }
    pub fn render(&mut self, adjustments: &Adjustments) -> Result<Rendered> {
        let _guard = ENGINE
            .lock()
            .map_err(|_| anyhow::anyhow!("rtengine session failed"))?;
        ensure!(
            [
                adjustments.exposure,
                adjustments.temperature,
                adjustments.tint,
                adjustments.raw_temperature,
                adjustments.raw_tint,
                adjustments.sharpen,
                adjustments.sharpen_radius,
                adjustments.sharpen_detail,
                adjustments.sharpen_masking,
                adjustments.noise_luma,
                adjustments.noise_color,
                adjustments.scale
            ]
            .iter()
            .all(|v| v.is_finite()),
            "Invalid rtengine adjustments"
        );
        ensure!(
            (-5. ..=5.).contains(&adjustments.exposure) && (0. ..=1.).contains(&adjustments.scale),
            "Invalid native exposure or scale"
        );
        let mut frame = Frame::default();
        // Frame data remain bridge-owned until the next render or session destruction.
        ensure!(
            unsafe { openroom_rt_render(self.handle.as_ptr(), adjustments, &mut frame) } == 0,
            "{}",
            error()
        );
        ensure!(
            frame.width > 0
                && frame.height > 0
                && u64::from(frame.width) * u64::from(frame.height) <= 64_000_000,
            "Invalid rtengine output dimensions"
        );
        // Write directly into Rust's owned buffer while the engine mutex protects the frame.
        let mut samples = vec![0.; frame.width as usize * frame.height as usize * 4];
        ensure!(
            unsafe { openroom_rt_copy(self.handle.as_ptr(), samples.as_mut_ptr(), samples.len()) }
                == 0,
            "{}",
            error()
        );
        ensure!(
            samples.iter().all(|v| v.is_finite()),
            "rtengine returned invalid floating-point samples"
        );
        let pixels = Rgba32FImage::from_raw(frame.width, frame.height, samples)
            .context("Invalid rtengine image")?;
        // Camera text is session-owned and copied before the next native call.
        let camera = unsafe { CStr::from_ptr(openroom_rt_camera(self.handle.as_ptr())) }
            .to_string_lossy()
            .into_owned();
        Ok(Rendered {
            pixels,
            source_width: frame.source_width,
            source_height: frame.source_height,
            temperature: frame.temperature as f32,
            green: frame.green as f32,
            camera,
            camera_matched: unsafe { openroom_rt_camera_matched(self.handle.as_ptr()) } != 0,
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _guard = ENGINE.lock().unwrap_or_else(|error| error.into_inner());
        // The session cannot outlive, or be used concurrently with, its owner thread.
        unsafe { openroom_rt_close(self.handle.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_denoising_matches_fresh_sessions_after_detail_and_white_balance_changes() {
        let pixels = Rgba32FImage::from_fn(96, 64, |x, y| {
            let noise = ((x * 73 + y * 37) % 101) as f32 / 300.;
            image::Rgba([x as f32 / 100. + noise, y as f32 / 80., noise, 1.])
        });
        let source = Path::new("native-detail-cache");
        let base = Adjustments {
            sharpen: 40.,
            noise_luma: 15.,
            noise_color: 10.,
            ..Default::default()
        };
        let mut cached = Session::open(source, Some(&pixels), 96).unwrap();
        for settings in [
            base,
            Adjustments {
                sharpen: 70.,
                ..base
            },
            Adjustments {
                sharpen_radius: 2.,
                sharpen_masking: 25.,
                ..base
            },
            Adjustments {
                noise_luma: 25.,
                ..base
            },
            Adjustments {
                noise_color: 30.,
                ..base
            },
            Adjustments {
                temperature: 20.,
                tint: -10.,
                ..base
            },
            Adjustments {
                exposure: -1.,
                ..base
            },
            Adjustments {
                sharpen: 0.,
                ..base
            },
            Adjustments::default(),
            base,
        ] {
            let actual = cached.render(&settings).unwrap().pixels;
            let mut fresh = Session::open(source, Some(&pixels), 96).unwrap();
            assert_eq!(
                actual,
                fresh.render(&settings).unwrap().pixels,
                "stale detail cache: {settings:?}"
            );
        }
    }

    #[test]
    #[ignore = "requires AE_RAW_PHOTO pointing to an X-Trans RAF fixture"]
    fn repeated_raw_sessions_produce_identical_pixels() {
        init().unwrap();
        let source = std::path::PathBuf::from(std::env::var_os("AE_RAW_PHOTO").unwrap());
        for bound in [0, 1600, 384] {
            let mut reference = None;
            let mut detailed_reference = None;
            for _ in 0..12 {
                let mut session = Session::open(&source, None, bound).unwrap();
                let frame = session.render(&Adjustments::default()).unwrap();
                if let Some(expected) = reference.as_ref() {
                    assert!(
                        frame.pixels == *expected,
                        "RAF pixels changed at bound {bound}"
                    );
                } else {
                    reference = Some(frame.pixels);
                }
                let detailed = session
                    .render(&Adjustments {
                        sharpen: 40.,
                        noise_luma: 15.,
                        noise_color: 15.,
                        ..Default::default()
                    })
                    .unwrap();
                if let Some(expected) = detailed_reference.as_ref() {
                    assert!(
                        detailed.pixels == *expected,
                        "Detailed RAF pixels changed at bound {bound}"
                    );
                } else {
                    detailed_reference = Some(detailed.pixels);
                }
            }
        }
    }
    #[test]
    #[ignore = "requires AE_RAW_PHOTO with an embedded camera JPEG"]
    fn camera_tone_is_stable_and_preserves_highlight_headroom() {
        let source = std::path::PathBuf::from(std::env::var_os("AE_RAW_PHOTO").unwrap());
        let mut session = Session::open(&source, None, 1600).unwrap();
        let initial = session.render(&Adjustments::default()).unwrap();
        assert!(initial.camera_matched);
        for adjustments in [
            Adjustments {
                exposure: 5.,
                ..Default::default()
            },
            Adjustments {
                raw_temperature: 5500.,
                ..Default::default()
            },
            Adjustments {
                exposure: -2.,
                ..Default::default()
            },
        ] {
            let changed = session.render(&adjustments).unwrap();
            assert!(changed.camera_matched);
            assert_ne!(changed.pixels, initial.pixels);
            if adjustments.exposure == 5. {
                assert!(
                    changed
                        .pixels
                        .pixels()
                        .any(|p| p.0[..3].iter().any(|v| *v > 1.1))
                );
            }
            assert_eq!(
                session.render(&Adjustments::default()).unwrap().pixels,
                initial.pixels
            );
        }
        let mut fresh = Session::open(&source, None, 1600).unwrap();
        assert_eq!(
            fresh.render(&Adjustments::default()).unwrap().pixels,
            initial.pixels
        );
    }

    #[test]
    fn native_float_updates_preserve_headroom_and_apply_detail() {
        init().unwrap();
        let image = Rgba32FImage::from_fn(96, 64, |x, y| {
            image::Rgba([
                0.1 + (x % 7) as f32 * 0.02,
                0.2 + (y % 5) as f32 * 0.01,
                0.3,
                1.,
            ])
        });
        let mut session = Session::open(Path::new("native-memory"), Some(&image), 96).unwrap();
        let neutral = session.render(&Adjustments::default()).unwrap();
        for (actual, expected) in neutral.pixels.as_raw().iter().zip(image.as_raw()) {
            assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
        }
        let bright = session
            .render(&Adjustments {
                exposure: 1.,
                ..Default::default()
            })
            .unwrap();
        assert!((bright.pixels.get_pixel(0, 0)[0] - 0.2).abs() < 0.0001);
        let detailed = session
            .render(&Adjustments {
                sharpen: 60.,
                noise_luma: 15.,
                noise_color: 15.,
                ..Default::default()
            })
            .unwrap();
        assert_ne!(detailed.pixels, neutral.pixels);
        assert_eq!(
            session.render(&Adjustments::default()).unwrap().pixels,
            neutral.pixels
        );
        let warmer = session
            .render(&Adjustments {
                temperature: 20.,
                ..Default::default()
            })
            .unwrap();
        assert_ne!(warmer.pixels, neutral.pixels);
        assert_eq!(
            session.render(&Adjustments::default()).unwrap().pixels,
            neutral.pixels
        );
        let invalid = Adjustments {
            exposure: f32::NAN,
            ..Default::default()
        };
        assert!(session.render(&invalid).is_err());
        let hdr = Rgba32FImage::from_pixel(32, 32, image::Rgba([4., 2., 1., 0.5]));
        let mut session = Session::open(Path::new("native-hdr"), Some(&hdr), 32).unwrap();
        let recovered = session
            .render(&Adjustments {
                exposure: -2.,
                ..Default::default()
            })
            .unwrap();
        assert!((recovered.pixels.get_pixel(0, 0)[0] - 1.).abs() < 0.001);
        assert!((recovered.pixels.get_pixel(0, 0)[1] - 0.5).abs() < 0.001);
    }
}
