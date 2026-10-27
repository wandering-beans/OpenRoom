//! Remove catalog-owned data without opening or changing originals.
use crate::Catalog;
use anyhow::{Result, ensure};
use rusqlite::{Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Removal {
    Photos { ids: Vec<i64> },
    Folder { root_id: i64, path: String },
}

impl Catalog {
    pub fn remove_catalog_items(
        &self,
        selection: &Removal,
        cleanup: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let count = match selection {
            Removal::Photos { ids } => {
                ensure!(
                    !ids.is_empty() && ids.len() <= 10000,
                    "Select 1–10000 photos"
                );
                let ids = ids
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>();
                for id in &ids {
                    ensure!(
                        tx.execute("DELETE FROM items WHERE id=?", [id])? == 1,
                        "Photo not found"
                    );
                }
                ids.len()
            }
            Removal::Folder { root_id, path } => {
                ensure!(
                    self.roots()?.iter().any(|root| root.id == *root_id),
                    "Root not found"
                );
                ensure!(
                    path.is_empty()
                        || Path::new(path)
                            .components()
                            .all(|part| matches!(part, Component::Normal(_))),
                    "Invalid folder path"
                );
                let count = if path.is_empty() {
                    let count = tx.execute("DELETE FROM items WHERE root_id=?", [root_id])?;
                    tx.execute("DELETE FROM roots WHERE id=?", [root_id])?;
                    count
                } else {
                    let prefix = format!("{path}{}", std::path::MAIN_SEPARATOR);
                    tx.execute(
                        "DELETE FROM items WHERE root_id=? AND substr(path,1,length(?))=?",
                        params![root_id, prefix, prefix],
                    )?
                };
                count
            }
        };
        // A failed cache cleanup must not discard edits or history.
        cleanup(self)?;
        tx.execute("UPDATE state SET generation=generation+1", [])?;
        tx.commit()?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Annotation, Query, library::MetadataCandidate};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn removal_cascades_only_selected_data_and_preserves_originals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        for folder in ["旅行_100%", "旅行_100%/子", "旅行_100%別"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("photo.jpg"), b"original").unwrap();
        }
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root_id = catalog.add_root(&root).unwrap().id;
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let items = catalog.page(&Query::default(), 100, None).unwrap().items;
        catalog.store_recipe(None, "global preset", "{}").unwrap();
        for item in &items {
            catalog
                .save_develop_recipe(item.id, "{\"version\":1,\"exposure\":1}", 0)
                .unwrap();
            catalog
                .annotate(
                    &[item.id],
                    &Annotation {
                        tags: vec!["test".into()],
                        rating: 4,
                        memo: "note".into(),
                    },
                )
                .unwrap();
            catalog
                .store_recipe(Some(item.id), "snapshot", "{}")
                .unwrap();
            catalog
                .index_metadata(
                    &MetadataCandidate {
                        id: item.id,
                        size: item.size,
                        modified: item.modified,
                    },
                    &Default::default(),
                    None,
                )
                .unwrap();
            catalog
                .save_library_preview(
                    item.id,
                    &catalog.preview_stamp(item.id).unwrap(),
                    Some("{}"),
                    None,
                )
                .unwrap();
        }
        let selection = Removal::Folder {
            root_id,
            path: "旅行_100%".into(),
        };
        assert!(
            catalog
                .remove_catalog_items(&selection, |_| anyhow::bail!("disk failure"))
                .is_err()
        );
        assert_eq!(catalog.count(&Query::default()).unwrap(), 3);
        assert!(
            catalog
                .remove_catalog_items(
                    &Removal::Folder {
                        root_id,
                        path: "../photos".into()
                    },
                    |_| Ok(())
                )
                .is_err()
        );
        assert_eq!(
            catalog
                .remove_catalog_items(&selection, |_| Ok(()))
                .unwrap(),
            2
        );
        let survivor = catalog
            .page(&Query::default(), 100, None)
            .unwrap()
            .items
            .remove(0);
        assert!(survivor.path.starts_with("旅行_100%別"));
        for table in [
            "items",
            "item_tags",
            "develop_edits",
            "develop_history",
            "photo_metadata",
            "library_previews",
            "develop_preview_updates",
        ] {
            let count: i64 = catalog
                .conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                count,
                if table == "develop_history" { 2 } else { 1 },
                "{table}"
            );
        }
        assert_eq!(catalog.saved_recipes(Some(survivor.id)).unwrap().len(), 1);
        assert_eq!(catalog.saved_recipes(None).unwrap().len(), 1);
        assert!(
            catalog
                .remove_catalog_items(
                    &Removal::Photos {
                        ids: vec![survivor.id, i64::MAX]
                    },
                    |_| Ok(())
                )
                .is_err()
        );
        assert!(
            catalog.detail(survivor.id).is_ok(),
            "invalid batches roll back"
        );
        assert_eq!(
            catalog
                .remove_catalog_items(
                    &Removal::Photos {
                        ids: vec![survivor.id, survivor.id]
                    },
                    |_| Ok(())
                )
                .unwrap(),
            1
        );
        assert!(catalog.saved_recipes(Some(survivor.id)).unwrap().is_empty());
        assert_eq!(catalog.roots().unwrap().len(), 1);
        catalog
            .remove_catalog_items(
                &Removal::Folder {
                    root_id,
                    path: String::new(),
                },
                |_| Ok(()),
            )
            .unwrap();
        assert!(catalog.roots().unwrap().is_empty());
        for item in items {
            assert_eq!(std::fs::read(root.join(item.path)).unwrap(), b"original");
        }
    }
}
