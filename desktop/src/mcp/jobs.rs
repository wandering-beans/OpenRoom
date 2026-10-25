use anyhow::{Result, bail, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::Emitter;

#[derive(Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub progress: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    #[serde(skip)]
    pub cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct Jobs {
    entries: Mutex<BTreeMap<String, Job>>,
    next: AtomicU64,
}
impl Jobs {
    pub fn list(&self) -> Vec<Job> {
        self.entries.lock().unwrap().values().cloned().collect()
    }
    pub fn get(&self, id: &str) -> Result<Job> {
        self.entries
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Job not found"))
    }
    pub fn cancel(&self, id: &str) -> Result<Job> {
        let job = self.get(id)?;
        job.cancel.store(true, Ordering::SeqCst);
        self.get(id)
    }
    pub fn cancel_all(&self) {
        for job in self.entries.lock().unwrap().values() {
            job.cancel.store(true, Ordering::SeqCst);
        }
    }
    pub fn running(&self) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|job| job.status == "running")
    }
    pub fn progress(&self, id: &str, value: Value, app: Option<&tauri::AppHandle>) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(job) = entries.get_mut(id) {
            job.progress = value;
            if let Some(app) = app {
                let _ = app.emit("mcp-job", &*job);
            }
        }
    }
    pub fn start(
        self: &Arc<Self>,
        kind: &str,
        app: Option<tauri::AppHandle>,
        action: impl FnOnce(Arc<AtomicBool>, String) -> Result<Value> + Send + 'static,
    ) -> Result<Job> {
        let mut entries = self.entries.lock().unwrap();
        ensure!(
            !entries
                .values()
                .any(|job| job.kind == kind && job.status == "running"),
            "This job is already running"
        );
        if entries.len() >= 100 {
            let oldest = entries
                .iter()
                .find(|(_, job)| job.status != "running")
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                entries.remove(&id);
            } else {
                bail!("Too many active jobs");
            }
        }
        let id = format!("job-{:016}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        let job = Job {
            id: id.clone(),
            kind: kind.into(),
            status: "running".into(),
            progress: Value::Null,
            result: None,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        entries.insert(id.clone(), job.clone());
        drop(entries);
        let jobs = self.clone();
        let cancel = job.cancel.clone();
        tokio::spawn(async move {
            let worker_cancel = cancel.clone();
            let worker_id = id.clone();
            let result =
                tokio::task::spawn_blocking(move || action(worker_cancel, worker_id)).await;
            let result = result
                .map_err(anyhow::Error::from)
                .and_then(|result| result);
            let mut entries = jobs.entries.lock().unwrap();
            let job = entries.get_mut(&id).expect("Running jobs are retained");
            job.status = if cancel.load(Ordering::SeqCst) {
                "cancelled"
            } else if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
            .into();
            match result {
                Ok(value) => job.result = Some(value),
                Err(error) => job.error = Some(format!("{error:#}")),
            }
            if let Some(app) = app {
                let _ = app.emit("mcp-job", &*job);
            }
        });
        Ok(job)
    }
}
