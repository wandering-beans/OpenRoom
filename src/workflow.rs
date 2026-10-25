//! Catalog-backed presets, named snapshots, and all-or-nothing batch edits.
use crate::{Catalog, HistoryEdit, now};
use anyhow::{Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedRecipe {
    pub id: i64,
    pub name: String,
    pub recipe: String,
}
impl Catalog {
    pub fn saved_recipes(&self, item_id: Option<i64>) -> Result<Vec<SavedRecipe>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id,name,recipe FROM saved_recipes WHERE item_id IS ? ORDER BY name,id",
            )?
            .query_map([item_id], |r| {
                Ok(SavedRecipe {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    recipe: decode_recipe(r.get_ref(2)?)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn store_recipe(&self, item_id: Option<i64>, name: &str, recipe: &str) -> Result<i64> {
        ensure!(
            !name.trim().is_empty() && name.len() <= 256 && recipe.len() <= 2_097_152,
            "Invalid saved recipe"
        );
        let value: serde_json::Value = serde_json::from_str(recipe)?;
        ensure!(value.is_object(), "Invalid recipe");
        self.conn.execute(
            "INSERT INTO saved_recipes(item_id,name,recipe,created) VALUES(?,?,?,?)",
            params![item_id, name.trim(), encode_recipe(recipe)?, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_saved_recipe(&self, id: i64, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty() && name.len() <= 256, "Invalid name");
        ensure!(
            self.conn.execute(
                "UPDATE saved_recipes SET name=? WHERE id=?",
                params![name.trim(), id]
            )? == 1,
            "Saved recipe not found"
        );
        Ok(())
    }
    pub fn delete_saved_recipe(&self, id: i64) -> Result<()> {
        ensure!(
            self.conn
                .execute("DELETE FROM saved_recipes WHERE id=?", [id])?
                == 1,
            "Saved recipe not found"
        );
        Ok(())
    }
    pub fn rename_history(&self, item_id: i64, id: i64, label: &str) -> Result<()> {
        ensure!(
            !label.trim().is_empty() && label.len() <= 256,
            "Invalid history name"
        );
        ensure!(
            self.conn.execute(
                "UPDATE develop_history SET label=? WHERE id=? AND item_id=?",
                params![label.trim(), id, item_id]
            )? == 1,
            "History not found"
        );
        Ok(())
    }
    /// Commit validated recipes together; stale revisions roll back the entire batch.
    pub fn save_develop_batch(&mut self, edits: &[(i64, HistoryEdit, i64)]) -> Result<()> {
        ensure!(
            !edits.is_empty() && edits.len() <= 500,
            "Invalid edit batch"
        );
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let mut seen = std::collections::HashSet::new();
            for (id, edit, revision) in edits {
                ensure!(seen.insert(*id), "Duplicate photo");
                self.save_develop_steps(*id, std::slice::from_ref(edit), *revision)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
    pub fn apply_develop_batch(
        &mut self,
        ids: &[i64],
        patch: &serde_json::Value,
        label: &str,
    ) -> Result<()> {
        ensure!(
            !ids.is_empty() && ids.len() <= 10000,
            "Select 1–10000 photos"
        );
        ensure!(
            patch.is_object() && patch.to_string().len() <= 2_097_152,
            "Invalid patch"
        );
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let mut seen = std::collections::HashSet::new();
            for id in ids {
                ensure!(seen.insert(*id), "Duplicate photo");
                let (recipe, revision) = self.develop_recipe(*id)?;
                let mut value: serde_json::Value =
                    serde_json::from_str(recipe.as_deref().unwrap_or("{\"version\":1}"))?;
                for (key, entry) in patch.as_object().unwrap() {
                    value[key] = entry.clone();
                }
                self.save_develop_steps(
                    *id,
                    &[HistoryEdit {
                        recipe: value.to_string(),
                        label: label.into(),
                    }],
                    revision,
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
}

pub(crate) fn encode_recipe(recipe: &str) -> Result<rusqlite::types::Value> {
    if recipe.len() < 4096 {
        return Ok(rusqlite::types::Value::Text(recipe.into()));
    }
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(recipe.as_bytes())?;
    Ok(rusqlite::types::Value::Blob(encoder.finish()?))
}
pub(crate) fn decode_recipe(value: rusqlite::types::ValueRef<'_>) -> rusqlite::Result<String> {
    use std::io::Read;
    let result = (|| -> Result<String> {
        match value {
            rusqlite::types::ValueRef::Text(bytes) => Ok(std::str::from_utf8(bytes)?.into()),
            rusqlite::types::ValueRef::Blob(bytes) => {
                let mut output = Vec::new();
                flate2::read::ZlibDecoder::new(bytes)
                    .take(2_097_153)
                    .read_to_end(&mut output)?;
                ensure!(output.len() <= 2_097_152, "Recipe exceeds 2 MiB");
                Ok(String::from_utf8(output)?)
            }
            _ => anyhow::bail!("Invalid stored recipe"),
        }
    })();
    result.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            value.data_type(),
            Box::new(std::io::Error::other(error.to_string())),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn large_recipes_are_compressed_and_bounded_on_read() {
        let recipe =
            serde_json::json!({"version":1,"masks":[{"points":vec![[0.25,0.5];4096]}]}).to_string();
        let encoded = encode_recipe(&recipe).unwrap();
        let rusqlite::types::Value::Blob(bytes) = encoded else {
            panic!("Large recipes should be compressed")
        };
        assert!(bytes.len() < recipe.len() / 10);
        assert_eq!(
            decode_recipe(rusqlite::types::ValueRef::Blob(&bytes)).unwrap(),
            recipe
        );
        assert!(decode_recipe(rusqlite::types::ValueRef::Blob(b"broken stream")).is_err());
        let huge = encode_recipe(&"x".repeat(2_097_153)).unwrap();
        let rusqlite::types::Value::Blob(bytes) = huge else {
            unreachable!()
        };
        assert!(decode_recipe(rusqlite::types::ValueRef::Blob(&bytes)).is_err());
    }
    #[test]
    fn batch_is_atomic_and_named_recipes_survive_backup() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        for name in ["a.jpg", "b.jpg"] {
            std::fs::write(photos.join(name), b"untouched").unwrap();
        }
        let db = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let ids: Vec<_> = catalog
            .page(&Query::default(), 10, None)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
        catalog
            .save_develop_recipe(ids[0], r#"{"version":1,"contrast":20}"#, 0)
            .unwrap();
        let patch = serde_json::json!({"exposure":1.25});
        assert!(
            catalog
                .apply_develop_batch(&[ids[0], i64::MAX], &patch, "同期")
                .is_err()
        );
        assert_eq!(catalog.develop_recipe(ids[0]).unwrap().1, 1);
        assert_eq!(catalog.develop_history(ids[0], None).unwrap().len(), 2);
        catalog.apply_develop_batch(&ids, &patch, "同期").unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&catalog.develop_recipe(ids[0]).unwrap().0.unwrap()).unwrap();
        assert_eq!(saved["contrast"], 20);
        assert_eq!(saved["exposure"], 1.25);
        catalog
            .store_recipe(None, "部分プリセット", &patch.to_string())
            .unwrap();
        catalog
            .store_recipe(Some(ids[0]), "完成", &saved.to_string())
            .unwrap();
        let backup = dir.path().join("backup.db");
        catalog.backup(&backup).unwrap();
        let restored = Catalog::open(backup).unwrap();
        assert_eq!(restored.saved_recipes(None).unwrap().len(), 1);
        assert_eq!(
            restored.saved_recipes(Some(ids[0])).unwrap()[0].name,
            "完成"
        );
        assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"untouched");
    }
}
