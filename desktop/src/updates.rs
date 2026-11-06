//! Signed application updates are independent of catalogs and original photos.
use crate::{AppState, Reply};
use base64::Engine;
use serde::Serialize;
use std::path::Path;
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

fn update_target() -> Reply<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("darwin-aarch64"),
        ("windows", "x86_64") => Ok("windows-x86_64"),
        _ => Err("このOS・CPUのアプリ内更新には対応していません。".into()),
    }
}

fn asset_name(version: &str, target: &str) -> Reply<String> {
    match target {
        "darwin-aarch64" => Ok(format!("OpenRoom-{version}-macos-arm64.app.tar.gz")),
        "windows-x86_64" => Ok(format!("OpenRoom-{version}-windows-x64-setup.exe")),
        _ => Err("更新対象のOS・CPUを確認できません。".into()),
    }
}

fn validate_target_release(update: &Update, target: &str) -> Reply<()> {
    let asset = asset_name(&update.version, target)?;
    let expected = format!(
        "https://github.com/wandering-beans/OpenRoom/releases/download/v{}/{asset}",
        update.version
    );
    let platform = &update.raw_json["platforms"][target];
    if update.target != target
        || update.download_url.as_str() != expected
        || platform["url"].as_str() != Some(expected.as_str())
        || platform["signature"].as_str() != Some(update.signature.as_str())
    {
        return Err("更新ファイルのOS・CPUまたは配布先を確認できません。".into());
    }
    Ok(())
}

// Call only after Update::download has verified both minisign signatures and the version.
fn validate_verified_package(update: &Update, target: &str, bytes: &[u8]) -> Reply<()> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&update.signature)
        .map_err(|error| error.to_string())?;
    let signature = std::str::from_utf8(&signature).map_err(|error| error.to_string())?;
    let filenames: Vec<_> = signature
        .lines()
        .filter_map(|line| line.strip_prefix("trusted comment: "))
        .flat_map(|line| line.split('\t'))
        .filter_map(|field| field.strip_prefix("file:"))
        .collect();
    if filenames != [asset_name(&update.version, target)?.as_str()] {
        return Err("署名された更新ファイルのOS・CPUが一致しません。".into());
    }
    let valid = match target {
        "darwin-aarch64" => bytes.starts_with(&[0x1f, 0x8b]),
        "windows-x86_64" => {
            // NSIS uses an x86 PE launcher even when its installed application is x64.
            bytes.starts_with(b"MZ")
                && bytes.get(0x3c..0x40).is_some_and(|offset| {
                    let offset = u32::from_le_bytes(offset.try_into().unwrap()) as usize;
                    bytes.get(offset..offset + 6).is_some_and(|header| {
                        &header[..4] == b"PE\0\0"
                            && matches!(u16::from_le_bytes([header[4], header[5]]), 0x14c | 0x8664)
                    })
                })
        }
        _ => false,
    };
    if !valid {
        return Err("更新ファイルの形式がこのOSに対応していません。".into());
    }
    Ok(())
}

fn installed_application(executable: &Path, target: &str) -> bool {
    match target {
        "darwin-aarch64" => executable
            .ancestors()
            .nth(3)
            .is_some_and(|path| path.extension().is_some_and(|ext| ext == "app")),
        "windows-x86_64" => executable.parent().is_some_and(|directory| {
            executable
                .file_name()
                .is_some_and(|name| name == "advanced-explorer-desktop.exe")
                && directory.join("uninstall.exe").is_file()
                && std::fs::read_to_string(directory.join("openroom-install-target")).is_ok_and(
                    |marker| marker.trim() == "local.advanced-explorer.desktop/windows-x86_64/nsis",
                )
        }),
        _ => false,
    }
}

fn validate_release(update: &Update) -> Reply<()> {
    validate_target_release(update, update_target()?)?;
    #[cfg(target_os = "macos")]
    validate_macos_version(update)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_macos_version(update: &Update) -> Reply<()> {
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

#[cfg(any(target_os = "macos", test))]
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
    let target = update_target()?;
    let mut pending = state.0.try_lock().map_err(|_| "更新処理が進行中です。")?;
    *pending = None;
    let update = app
        .updater_builder()
        .target(target)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| match error {
            tauri_plugin_updater::Error::TargetNotFound(_)
            | tauri_plugin_updater::Error::TargetsNotFound(_) => {
                "このOS・CPU向けの更新はまだ公開されていません。".to_owned()
            }
            error => error.to_string(),
        })?;
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
    let target = update_target()?;
    if !installed_application(&executable, target) {
        return Err("更新はインストール済みのOpenRoomから実行してください。".into());
    }
    ensure_idle(&app)?;
    let mut pending = state.0.try_lock().map_err(|_| "更新処理が進行中です。")?;
    let update = pending.as_ref().ok_or("先に更新を確認してください。")?;
    validate_release(update)?;
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
    validate_verified_package(update, target, &bytes)?;
    ensure_idle(&app)?;
    #[cfg(target_os = "windows")]
    crate::mcp::stop_for_exit(&app).await?;
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
