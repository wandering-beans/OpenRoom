//! Catalog selection is applied on restart so in-flight work cannot cross catalogs.
use crate::{AppState, Catalog, CloseGuard, Reply, work};
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

pub struct CatalogSelection {
    pub data: PathBuf,
    pub temporary: Option<tempfile::TempDir>,
}

pub fn startup_catalog(data: &Path) -> Result<Option<PathBuf>> {
    let preference = data.join("selected-catalog.json");
    if !preference.exists() {
        let default = data.join("catalog.db");
        return Ok(default.is_file().then_some(default));
    }
    let path: PathBuf = serde_json::from_slice(&std::fs::read(preference)?)?;
    ensure!(
        path.is_file(),
        "選択したカタログが見つかりません：{}",
        path.display()
    );
    Ok(Some(path))
}

#[tauri::command]
pub fn catalog_needs_creation(selection: State<'_, CatalogSelection>) -> bool {
    selection.temporary.is_some()
}

fn remember_catalog(data: &Path, path: &Path) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(data)?;
    file.write_all(&serde_json::to_vec(path)?)?;
    file.as_file().sync_all()?;
    file.persist(data.join("selected-catalog.json"))?;
    Ok(())
}

fn prepare_catalog(path: &Path, create: bool) -> Result<PathBuf> {
    if create {
        ensure!(
            path.file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.trim().is_empty() && name != ".openroom"),
            "カタログ名を入力してください。"
        );
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("openroom")),
            "拡張子は.openroomにしてください。"
        );
        ensure!(
            !path.exists(),
            "保存先はすでに存在します。別の名前を指定してください。"
        );
        let directory = tempfile::tempdir()?;
        Catalog::open(directory.path().join("empty.db"))?.backup(path)?;
    } else {
        ensure!(path.is_file(), "カタログが見つかりません。");
        Catalog::open(path)?
            .check()
            .context("カタログの整合性を確認できません")?;
    }
    Ok(std::fs::canonicalize(path)?)
}

#[tauri::command]
pub async fn choose_catalog(
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    selection: State<'_, CatalogSelection>,
    create: bool,
) -> Reply<Option<String>> {
    if state.scanning.load(Ordering::SeqCst) {
        return Err("読み込みが終わってから切り替えてください。".into());
    }
    let data = selection.data.clone();
    work(move || {
        let dialog = app.dialog().file().set_parent(&window).add_filter(
            crate::preferences::text(&app, "OpenRoomカタログ", "OpenRoom Catalog"),
            if create {
                &["openroom"][..]
            } else {
                &["openroom", "db"][..]
            },
        );
        let chosen = if create {
            dialog
                .set_title(crate::preferences::text(
                    &app,
                    "カタログ名と保存先を指定",
                    "Choose a Catalog Name and Location",
                ))
                .set_file_name("")
                .blocking_save_file()
        } else {
            dialog.blocking_pick_file()
        };
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        let path = prepare_catalog(
            &chosen
                .into_path()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?,
            create,
        )?;
        remember_catalog(&data, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub async fn restart_catalog(
    app: tauri::AppHandle,
    guard: State<'_, CloseGuard>,
    selection: State<'_, CatalogSelection>,
) -> Reply<()> {
    crate::mcp::stop_for_exit(&app).await?;
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(bundle) = executable
            .ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
        {
            // Relaunch the bundle through LaunchServices; a bare child executable can lose its window.
            let status = std::process::Command::new("/usr/bin/open")
                .arg("-n")
                .arg(bundle)
                .arg("--env")
                .arg(format!("AE_DATA_DIR={}", selection.data.display()))
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("アプリを再起動できませんでした。".into());
            }
            guard.0.store(false, Ordering::SeqCst);
            app.exit(0);
            return Ok(());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = selection;
    guard.0.store(false, Ordering::SeqCst);
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_survives_restart_and_rejects_foreign_or_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(startup_catalog(dir.path()).unwrap(), None);
        assert!(!dir.path().join("catalog.db").exists());
        Catalog::open(dir.path().join("catalog.db")).unwrap();
        assert_eq!(
            startup_catalog(dir.path()).unwrap(),
            Some(dir.path().join("catalog.db"))
        );
        let target = dir.path().join("作品.openroom");
        let canonical = prepare_catalog(&target, true).unwrap();
        remember_catalog(dir.path(), &canonical).unwrap();
        assert_eq!(
            startup_catalog(dir.path()).unwrap(),
            Some(canonical.clone())
        );
        assert!(prepare_catalog(&target, true).is_err());
        assert_eq!(prepare_catalog(&target, false).unwrap(), canonical);
        let foreign = dir.path().join("wrong.db");
        std::fs::write(&foreign, b"not a catalog").unwrap();
        assert!(prepare_catalog(&foreign, false).is_err());
        assert_eq!(startup_catalog(dir.path()).unwrap(), Some(canonical));
        std::fs::remove_file(target).unwrap();
        assert!(startup_catalog(dir.path()).is_err());
        assert!(prepare_catalog(&dir.path().join(" .openroom"), true).is_err());
        assert!(prepare_catalog(&dir.path().join(".openroom"), true).is_err());
    }
}
