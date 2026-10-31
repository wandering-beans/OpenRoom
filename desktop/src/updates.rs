//! Signed application updates are independent of catalogs and original photos.
use crate::{AppState, Reply};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct UpdateState(Mutex<Option<Update>>);

#[derive(Serialize)]
pub struct AvailableUpdate {
    version: String,
    notes: String,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

fn validate_release(update: &Update) -> Reply<()> {
    let expected = format!(
        "https://github.com/wandering-beans/OpenRoom/releases/download/v{0}/OpenRoom-{0}-macos-arm64.app.tar.gz",
        update.version
    );
    if update.download_url.as_str() != expected {
        return Err("更新ファイルの配布先を確認できません。".into());
    }
    let required = update.raw_json["minimum_macos"]
        .as_str()
        .and_then(system_version)
        .ok_or("更新に必要なmacOSのバージョンを確認できません。")?;
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|error| error.to_string())?;
    let current = system_version(String::from_utf8_lossy(&output.stdout).trim())
        .filter(|_| output.status.success())
        .ok_or("macOSのバージョンを確認できません。")?;
    if current < required {
        return Err(format!(
            "この更新にはmacOS {}.{}.{}以降が必要です。",
            required[0], required[1], required[2]
        ));
    }
    Ok(())
}

fn system_version(value: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut version = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        version[index] = part.parse().ok()?;
    }
    Some(version)
}

#[tauri::command]
pub async fn check_app_update(
    app: tauri::AppHandle,
    state: State<'_, UpdateState>,
) -> Reply<Option<AvailableUpdate>> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err("アプリ内更新はApple Silicon搭載Macに対応しています。".into());
    }
    let mut pending = state.0.try_lock().map_err(|_| "更新処理が進行中です。")?;
    *pending = None;
    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let Some(mut update) = update else {
        return Ok(None);
    };
    validate_release(&update)?;
    update.timeout = Some(Duration::from_secs(300));
    let info = AvailableUpdate {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
    };
    *pending = Some(update);
    Ok(Some(info))
}

fn ensure_idle(app: &tauri::AppHandle) -> Reply<()> {
    if app.state::<AppState>().scanning.load(Ordering::SeqCst)
        || app.state::<crate::batch_export::ExportState>().is_busy()
        || app.state::<crate::mcp::McpState>().has_running_jobs()
    {
        return Err("読み込み・書き出し・AIの処理が終わってから更新してください。".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle, state: State<'_, UpdateState>) -> Reply<()> {
    // Only a packaged app may replace itself; never replace a development target directory.
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    if !executable
        .ancestors()
        .nth(3)
        .is_some_and(|path| path.extension().is_some_and(|ext| ext == "app"))
    {
        return Err("更新はインストールしたOpenRoom.appから実行してください。".into());
    }
    ensure_idle(&app)?;
    let mut pending = state.0.try_lock().map_err(|_| "更新処理が進行中です。")?;
    let update = pending.as_ref().ok_or("先に更新を確認してください。")?;
    let mut downloaded = 0;
    let mut emitted = 0;
    // The plugin verifies both the signature and its signed version before returning bytes.
    let bytes = update
        .download(
            |length, total| {
                downloaded += length as u64;
                if downloaded - emitted >= 1024 * 1024 || Some(downloaded) == total {
                    let _ = app.emit("update-progress", DownloadProgress { downloaded, total });
                    emitted = downloaded;
                }
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())?;
    ensure_idle(&app)?;
    let update = update.clone();
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    *pending = None;
    Ok(())
}

#[cfg(test)]
mod tests;
