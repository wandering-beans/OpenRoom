//! Opt-in stage timings for development and import benchmarks.
use std::{io::Write, path::PathBuf, sync::OnceLock, time::Instant};

static OUTPUT: OnceLock<Option<PathBuf>> = OnceLock::new();

pub struct Stage {
    start: Instant,
    name: &'static str,
    path: &'static PathBuf,
}

pub fn start(name: &'static str) -> Option<Stage> {
    let path = OUTPUT
        .get_or_init(|| {
            std::env::var_os("AE_PREVIEW_TIMINGS")
                .map(|dir| PathBuf::from(dir).join(format!("{}.csv", std::process::id())))
        })
        .as_ref()?;
    Some(Stage {
        start: Instant::now(),
        name,
        path,
    })
}

impl Drop for Stage {
    fn drop(&mut self) {
        let micros = self.start.elapsed().as_micros();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path)
        {
            let _ = file.write_all(format!("{},{micros}\n", self.name).as_bytes());
        }
    }
}
