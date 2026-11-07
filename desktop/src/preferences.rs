//! Application preferences are independent of the selected catalog.
use crate::Reply;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tauri::Manager;

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Ja,
    En,
}

pub(crate) fn directory(app: &tauri::AppHandle) -> tauri::Result<PathBuf> {
    Ok(std::env::var_os("AE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or(app.path().app_data_dir()?))
}
fn read(data: &Path) -> Language {
    std::fs::read(data.join("language.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn save(data: &Path, filename: &str, value: &impl Serialize) -> anyhow::Result<()> {
    std::fs::create_dir_all(data)?;
    let mut file = tempfile::NamedTempFile::new_in(data)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.as_file().sync_all()?;
    file.persist(data.join(filename))?;
    Ok(())
}
// Keep the accepted wording independent of app versions, catalogs and platform line endings.
fn current_terms() -> [String; 2] {
    [include_str!("../../TERMS.md"), include_str!("../../DISCLAIMER.md")]
        .map(|text| text.replace("\r\n", "\n"))
}

pub(crate) fn terms_accepted(data: &Path) -> anyhow::Result<bool> {
    match std::fs::read(data.join("accepted-terms.json")) {
        Ok(bytes) => Ok(serde_json::from_slice::<[String; 2]>(&bytes).ok() == Some(current_terms())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub fn accept_terms(app: tauri::AppHandle) -> Reply<()> {
    let data = directory(&app).map_err(|error| error.to_string())?;
    save(&data, "accepted-terms.json", &current_terms()).map_err(|error| error.to_string())
}

pub fn text<'a>(app: &tauri::AppHandle, japanese: &'a str, english: &'a str) -> &'a str {
    if directory(app)
        .map(|path| read(&path) == Language::En)
        .unwrap_or(false)
    {
        english
    } else {
        japanese
    }
}
#[tauri::command]
pub fn get_language(app: tauri::AppHandle, window: tauri::Window) -> Reply<Language> {
    let language = read(&directory(&app).map_err(|error| error.to_string())?);
    window
        .set_title(if language == Language::En {
            "OpenRoom — Library"
        } else {
            "OpenRoom — ライブラリ"
        })
        .map_err(|error| error.to_string())?;
    Ok(language)
}
#[tauri::command]
pub fn set_language(app: tauri::AppHandle, language: Language) -> Reply<()> {
    save(
        &directory(&app).map_err(|error| error.to_string())?,
        "language.json",
        &language,
    )
    .map_err(|error| error.to_string())?;
    if let Some(menu) = app.menu() {
        for item in menu.items().map_err(|error| error.to_string())? {
            if let Some(submenu) = item.as_submenu() {
                for (id, ja, en) in [
                    ("preferences", "環境設定…", "Settings…"),
                    ("quit-guarded", "OpenRoomを終了", "Quit OpenRoom"),
                ] {
                    if let Some(item) = submenu.get(id).and_then(|item| item.as_menuitem().cloned())
                    {
                        item.set_text(if language == Language::En { en } else { ja })
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
        }
    }
    Ok(())
}
fn read_auto_update(data: &Path) -> anyhow::Result<bool> {
    match std::fs::read(data.join("auto-update.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub fn get_auto_update(app: tauri::AppHandle) -> Reply<bool> {
    read_auto_update(&directory(&app).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_auto_update(app: tauri::AppHandle, enabled: bool) -> Reply<()> {
    save(&directory(&app).map_err(|error| error.to_string())?, "auto-update.json", &enabled)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_requires_the_current_wording_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        assert!(!terms_accepted(data).unwrap());
        save(data, "accepted-terms.json", &current_terms()).unwrap();
        assert!(terms_accepted(data).unwrap());
        save(data, "accepted-terms.json", &["older terms", "older disclaimer"]).unwrap();
        assert!(!terms_accepted(data).unwrap());
        std::fs::write(data.join("accepted-terms.json"), b"{").unwrap();
        assert!(!terms_accepted(data).unwrap());
    }
    #[test]
    fn automatic_updates_default_on_persist_and_fail_closed_on_invalid_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("catalog.db"), b"unchanged").unwrap();
        assert!(read_auto_update(dir.path()).unwrap());
        for enabled in [false, true, false] {
            save(dir.path(), "auto-update.json", &enabled).unwrap();
            assert_eq!(read_auto_update(dir.path()).unwrap(), enabled);
        }
        assert_eq!(std::fs::read(dir.path().join("catalog.db")).unwrap(), b"unchanged");
        std::fs::write(dir.path().join("auto-update.json"), b"invalid").unwrap();
        assert!(read_auto_update(dir.path()).is_err());
    }

    #[test]
    fn language_persists_without_touching_the_catalog_and_rejects_unknown_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("catalog.db"), b"unchanged").unwrap();
        assert_eq!(read(dir.path()), Language::Ja);
        save(dir.path(), "language.json", &Language::En).unwrap();
        assert_eq!(read(dir.path()), Language::En);
        assert!(serde_json::from_str::<Language>("\"other\"").is_err());
        assert_eq!(
            std::fs::read(dir.path().join("catalog.db")).unwrap(),
            b"unchanged"
        );
        save(dir.path(), "language.json", &Language::Ja).unwrap();
        assert_eq!(read(dir.path()), Language::Ja);
    }
}
