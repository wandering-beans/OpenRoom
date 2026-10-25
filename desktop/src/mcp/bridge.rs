//! Correlated UI requests. Expired requests cannot later perform a catalog write.
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::Emitter;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
pub struct UiRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
struct Pending {
    request: UiRequest,
    deadline: Instant,
    claimed: bool,
    executing: bool,
    stop: CancellationToken,
    reply: oneshot::Sender<Result<Value, String>>,
}
#[derive(Default)]
pub struct Bridge {
    pending: Mutex<HashMap<String, Pending>>,
    next: AtomicU64,
    pub serial: tokio::sync::Mutex<()>,
}
impl Bridge {
    pub async fn request(
        self: &Arc<Self>,
        app: Option<&tauri::AppHandle>,
        name: String,
        arguments: Value,
        stop: CancellationToken,
    ) -> Result<Value> {
        let _serial = self.serial.lock().await;
        ensure!(!stop.is_cancelled(), "MCP server is stopped");
        let id = format!("ui-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        let request = UiRequest {
            id: id.clone(),
            name,
            arguments,
        };
        let (reply, mut receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                request: request.clone(),
                deadline: Instant::now() + Duration::from_secs(30),
                claimed: false,
                executing: false,
                stop,
                reply,
            },
        );
        let _cleanup = PendingGuard {
            bridge: self.clone(),
            id: id.clone(),
        };
        if let Some(app) = app
            && let Err(error) = app.emit("mcp-request", &request)
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(error.into());
        }
        let result = tokio::select! {
            result=&mut receiver=>result,
            _=tokio::time::sleep(Duration::from_secs(30))=>{
                {
                    let mut pending=self.pending.lock().unwrap();
                    let executing=pending.get(&id).is_some_and(|request|request.executing);
                    if !executing {pending.remove(&id);return Err(anyhow::anyhow!("UI request expired; no MCP change was executed"));}
                }
                tokio::time::timeout(Duration::from_secs(120), receiver).await
                    .context("UI response lost after execution started; inspect the catalog before retrying")?
            }
        };
        self.pending.lock().unwrap().remove(&id);
        result
            .context("UI disconnected; check current state before retrying")?
            .map_err(anyhow::Error::msg)
    }
    pub fn claim(&self, id: &str) -> Result<UiRequest> {
        let mut pending = self.pending.lock().unwrap();
        let request = pending
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("UI request expired"))?;
        ensure!(
            !request.claimed && Instant::now() < request.deadline,
            "UI request expired or already claimed"
        );
        request.claimed = true;
        Ok(request.request.clone())
    }
    pub fn begin(&self, id: &str) -> Result<UiRequest> {
        let mut pending = self.pending.lock().unwrap();
        let request = pending
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("UI request expired"))?;
        ensure!(
            request.claimed
                && !request.executing
                && !request.stop.is_cancelled()
                && Instant::now() < request.deadline,
            "UI request expired or already executed"
        );
        request.executing = true;
        Ok(request.request.clone())
    }
    pub fn complete(&self, id: &str, value: Value, error: Option<String>) -> Result<()> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("UI request expired"))?;
        let _ = pending.reply.send(match error {
            Some(error) => Err(error),
            None => Ok(value),
        });
        Ok(())
    }
    #[cfg(test)]
    pub fn queued(&self) -> Vec<UiRequest> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .filter(|request| !request.claimed)
            .map(|request| request.request.clone())
            .collect()
    }
    pub fn stop(&self) {
        let mut requests = self.pending.lock().unwrap();
        // In-flight writes must finish and report their actual result.
        let queued: Vec<_> = requests
            .iter()
            .filter(|(_, request)| !request.executing)
            .map(|(id, _)| id.clone())
            .collect();
        for id in queued {
            if let Some(request) = requests.remove(&id) {
                let _ = request.reply.send(Err("MCP server stopped".into()));
            }
        }
    }
}
use anyhow::Context;

struct PendingGuard {
    bridge: Arc<Bridge>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.bridge.pending.lock().unwrap().remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_is_checked_before_claim_and_again_before_execution() {
        let bridge = Bridge::default();
        let (reply, _receiver) = oneshot::channel();
        bridge.pending.lock().unwrap().insert(
            "expired".into(),
            Pending {
                request: UiRequest {
                    id: "expired".into(),
                    name: "set_view".into(),
                    arguments: Value::Null,
                },
                deadline: Instant::now(),
                claimed: false,
                executing: false,
                stop: CancellationToken::new(),
                reply,
            },
        );
        assert!(bridge.claim("expired").is_err());
        bridge
            .pending
            .lock()
            .unwrap()
            .get_mut("expired")
            .unwrap()
            .claimed = true;
        assert!(bridge.begin("expired").is_err());
    }
}
