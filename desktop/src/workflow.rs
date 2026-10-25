use crate::{AppState, Catalog, Reply, work};
use advanced_explorer_develop::Settings;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::State;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    List {
        item_id: Option<i64>,
    },
    Store {
        item_id: Option<i64>,
        name: String,
        settings: Settings,
        fields: Vec<String>,
    },
    Rename {
        id: i64,
        name: String,
    },
    Delete {
        id: i64,
    },
    RenameHistory {
        item_id: i64,
        id: i64,
        name: String,
    },
    Apply {
        ids: Vec<i64>,
        settings: Settings,
        fields: Vec<String>,
    },
}
pub fn patch(settings: &Settings, fields: &[String]) -> Result<Value> {
    settings.validate()?;
    ensure!(!fields.is_empty(), "補正項目を選択してください");
    let all = serde_json::to_value(settings)?;
    let mut patch = serde_json::Map::new();
    for key in fields {
        ensure!(
            key != "version" && all.get(key).is_some(),
            "Unknown setting: {key}"
        );
        patch.insert(key.clone(), all[key].clone());
    }
    Ok(Value::Object(patch))
}
#[tauri::command]
pub async fn develop_workflow(state: State<'_, AppState>, request: Request) -> Reply<Value> {
    let db = state.db.clone();
    work(move || {
        let mut catalog = Catalog::open(db)?;
        match request {
            Request::List { item_id } => Ok(serde_json::to_value(catalog.saved_recipes(item_id)?)?),
            Request::Store {
                item_id,
                name,
                settings,
                fields,
            } => {
                settings.validate()?;
                let recipe = if item_id.is_some() {
                    settings.catalog_recipe()?
                } else {
                    patch(&settings, &fields)?.to_string()
                };
                Ok(json!(catalog.store_recipe(item_id, &name, &recipe)?))
            }
            Request::Rename { id, name } => {
                catalog.rename_saved_recipe(id, &name)?;
                Ok(Value::Null)
            }
            Request::Delete { id } => {
                catalog.delete_saved_recipe(id)?;
                Ok(Value::Null)
            }
            Request::RenameHistory { item_id, id, name } => {
                catalog.rename_history(item_id, id, &name)?;
                Ok(Value::Null)
            }
            Request::Apply {
                ids,
                settings,
                fields,
            } => {
                let patch = patch(&settings, &fields)?;
                catalog.apply_develop_batch(&ids, &patch, "設定を同期")?;
                Ok(json!(ids.len()))
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn preset_file(
    app: tauri::AppHandle,
    window: tauri::Window,
    write: bool,
    value: Value,
) -> Reply<Option<Value>> {
    use tauri_plugin_dialog::DialogExt;
    work(move || {
        let dialog = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter(crate::preferences::text(&app, "OpenRoom プリセット", "OpenRoom Preset"), &["json"]);
        let selected = if write {
            dialog.set_file_name("preset.json").blocking_save_file()
        } else {
            dialog.blocking_pick_file()
        };
        let Some(path) = selected else {
            return Ok(None);
        };
        let path = path
            .into_path()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if write {
            let settings: Settings = serde_json::from_value(value.clone())?;
            settings.validate()?;
            let parent = path.parent().context("Preset has no parent directory")?;
            let temporary = tempfile::tempdir_in(parent)?;
            let prepared = temporary.path().join("preset.json");
            std::fs::write(&prepared, serde_json::to_vec_pretty(&value)?)?;
            crate::batch_export::publish(&prepared, &path)?;
            Ok(None)
        } else {
            ensure!(
                std::fs::metadata(&path)?.len() <= 2_097_152,
                "Preset exceeds 2 MiB"
            );
            let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
            let settings: Settings = serde_json::from_value(value.clone())?;
            settings.validate()?;
            Ok(Some(value))
        }
    })
    .await
}
