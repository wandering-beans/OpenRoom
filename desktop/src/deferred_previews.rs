//! Edited library images are released only when the selected photo changes.
use crate::{AppState, Catalog, Reply, work};
use std::sync::{
    Mutex,
    atomic::{AtomicI64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{Emitter, State};

pub struct Updates {
    pub active: AtomicI64,
    pub generation: AtomicUsize,
    last_input: Mutex<Instant>,
    running: Mutex<bool>,
    selection: AtomicUsize,
    foreground: AtomicUsize,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            active: AtomicI64::new(0),
            generation: AtomicUsize::new(0),
            last_input: Mutex::new(Instant::now()),
            running: Mutex::new(false),
            selection: AtomicUsize::new(0),
            foreground: AtomicUsize::new(0),
        }
    }
}
pub struct Foreground<'a>(&'a Updates);
impl Drop for Foreground<'_> {
    fn drop(&mut self) {
        self.0.interrupt();
        self.0.foreground.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Updates {
    pub fn foreground(&self) -> Foreground<'_> {
        self.foreground.fetch_add(1, Ordering::SeqCst);
        self.interrupt();
        Foreground(self)
    }
    pub(crate) fn busy(&self) -> bool {
        self.foreground.load(Ordering::SeqCst) > 0
            || self.last_input.lock().unwrap().elapsed() < Duration::from_millis(1000)
    }
    pub fn interrupt(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.last_input.lock().unwrap() = Instant::now();
    }
}

#[tauri::command]
pub async fn set_preview_photo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Reply<()> {
    let state = (*state).clone();
    let selection = state
        .library
        .updates
        .selection
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    work(move || {
        let catalog = Catalog::open(&state.db)?;
        catalog.detail(id)?;
        let mut running = state.library.updates.running.lock().unwrap();
        if selection != state.library.updates.selection.load(Ordering::SeqCst) { return Ok(()); }
        if state.library.updates.active.swap(id, Ordering::SeqCst) == id { return Ok(()); }
        state.library.updates.interrupt();
        catalog.release_preview_updates(id)?;
        if *running { return Ok(()); }
        *running = true;
        drop(running);
        std::thread::spawn(move || {
            let result = drain(&state, |id| { let _ = app.emit("library-preview-updated", id); });
            if let Err(error) = result {
                *state.library.updates.running.lock().unwrap() = false;
                let _ = app.emit("library-preview-failed", format!("一覧画像を更新できませんでした。設定は保存済みです。次の写真切り替えで再試行します。{error}"));
            }
        });
        Ok(())
    }).await
}
pub(crate) fn drain(state: &AppState, updated: impl Fn(i64)) -> anyhow::Result<()> {
    loop {
        let catalog = Catalog::open(&state.db)?;
        let mut running = state.library.updates.running.lock().unwrap();
        let jobs = catalog.pending_preview_updates()?;
        if jobs.is_empty() {
            *running = false;
            return Ok(());
        }
        drop(running);
        for (id, revision) in jobs {
            while state.library.updates.busy() {
                std::thread::sleep(Duration::from_millis(100));
            }
            if state.library.updates.active.load(Ordering::SeqCst) == id {
                continue;
            }
            let generation = state.library.updates.generation.load(Ordering::SeqCst);
            let cancelled =
                || state.library.updates.generation.load(Ordering::SeqCst) != generation;
            let result = state.library.generate(state, id, &cancelled, true);
            if cancelled() {
                continue;
            }
            result?;
            let current_stamp = catalog.preview_stamp(id)?;
            anyhow::ensure!(
                catalog
                    .library_preview(id)?
                    .is_some_and(|record| record.stamp == current_stamp && record.data.is_some()),
                "原本が利用できないため一覧画像の更新を保留しました。"
            );
            catalog.complete_preview_update(id, revision)?;
            updated(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_foreground_render_keeps_background_work_waiting() {
        let updates = Updates::default();
        let render = updates.foreground();
        *updates.last_input.lock().unwrap() = Instant::now() - Duration::from_secs(2);
        assert!(updates.busy());
        let generation = updates.generation.load(Ordering::SeqCst);
        drop(render);
        assert!(updates.busy());
        assert!(updates.generation.load(Ordering::SeqCst) > generation);
        *updates.last_input.lock().unwrap() = Instant::now() - Duration::from_secs(2);
        assert!(!updates.busy());
    }
}
