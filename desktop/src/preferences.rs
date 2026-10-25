//! Application language is independent of the selected catalog.
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

fn directory(app: &tauri::AppHandle) -> tauri::Result<PathBuf> {
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
fn save(data: &Path, language: Language) -> anyhow::Result<()> {
    std::fs::create_dir_all(data)?;
    let mut file = tempfile::NamedTempFile::new_in(data)?;
    file.write_all(&serde_json::to_vec(&language)?)?;
    file.as_file().sync_all()?;
    file.persist(data.join("language.json"))?;
    Ok(())
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
        language,
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_persists_without_touching_the_catalog_and_rejects_unknown_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("catalog.db"), b"unchanged").unwrap();
        assert_eq!(read(dir.path()), Language::Ja);
        save(dir.path(), Language::En).unwrap();
        assert_eq!(read(dir.path()), Language::En);
        assert!(serde_json::from_str::<Language>("\"other\"").is_err());
        assert_eq!(
            std::fs::read(dir.path().join("catalog.db")).unwrap(),
            b"unchanged"
        );
        save(dir.path(), Language::Ja).unwrap();
        assert_eq!(read(dir.path()), Language::Ja);
    }
}
