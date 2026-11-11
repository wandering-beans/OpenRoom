//! Catalog-only removal and explicitly confirmed recycling of originals.
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
    Parent { path: String, root_ids: Vec<i64> },
}

#[derive(Deserialize, Serialize)]
pub struct TrashPlan {
    pub ids: Vec<i64>,
    pub generation: i64,
}

#[derive(Default, Serialize)]
pub struct TrashReport {
    pub removed: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

pub(super) fn remove_root(tx: &Transaction<'_>, id: i64) -> Result<usize> {
    let count = tx.execute("DELETE FROM items WHERE root_id=?", [id])?;
    ensure!(
        tx.execute("DELETE FROM roots WHERE id=?", [id])? == 1,
        "Root not found"
    );
    Ok(count)
}

impl Catalog {
    // Only act on the registered children shown when the menu opened.
    fn check_parent(&self, path: &str, ids: &[i64]) -> Result<()> {
        let parent = Path::new(path);
        let children: std::collections::HashSet<_> = self
            .roots()?
            .into_iter()
            .filter(|root| Path::new(&root.path).parent() == Some(parent))
            .map(|root| root.id)
            .collect();
        ensure!(
            !ids.is_empty() && ids.iter().all(|id| children.contains(id)),
            "Folder group changed"
        );
        Ok(())
    }

    pub fn plan_trash(&self, selection: &Removal) -> Result<TrashPlan> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)?;
        let generation = tx.query_row("SELECT generation FROM state", [], |r| r.get(0))?;
        let mut ids = match selection {
            Removal::Photos { ids } => ids.clone(),
            Removal::Parent { path, root_ids } => {
                self.check_parent(path, root_ids)?;
                let mut query = tx.prepare("SELECT id FROM items WHERE root_id=?")?;
                let mut ids = Vec::new();
                for root_id in root_ids {
                    for id in query.query_map([root_id], |row| row.get(0))? {
                        ids.push(id?);
                    }
                }
                ids
            }
            Removal::Folder { root_id, path } => {
                ensure!(
                    Path::new(path)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_))),
                    "Invalid folder path"
                );
                let prefix = format!("{path}{}", std::path::MAIN_SEPARATOR);
                let mut query = tx.prepare(
                    "SELECT id FROM items WHERE root_id=? AND (?='' OR substr(path,1,length(?))=?) ORDER BY id",
                )?;
                query
                    .query_map(params![root_id, path, prefix, prefix], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        ids.sort_unstable();
        ids.dedup();
        tx.commit()?;
        Ok(TrashPlan { ids, generation })
    }

    fn original_for_trash(&self, id: i64) -> Result<std::path::PathBuf> {
        use anyhow::Context;
        let (root, relative, extension, size, modified, identity, missing) = self.conn.query_row(
            "SELECT r.id,r.path,i.path,i.extension,i.size,i.modified,i.identity,i.missing
             FROM items i JOIN roots r ON r.id=i.root_id WHERE i.id=?",
            [id],
            |r| {
                Ok((
                    crate::Root {
                        id: r.get(0)?,
                        path: r.get(1)?,
                    },
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, bool>(7)?,
                ))
            },
        )?;
        let relative = Path::new(&relative);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
                && crate::supported(&extension)
                && relative
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(&extension)),
            "Invalid catalog photo path"
        );
        let path = Path::new(&root.path).join(relative);
        (|| -> Result<()> {
            ensure!(
                !missing
                    && std::fs::symlink_metadata(&path)?.is_file()
                    && path.canonicalize()? == path,
                "Original unavailable or path redirected"
            );
            let current = crate::file_record(&root, &path, extension)?;
            ensure!(
                (size, modified, identity) == (current.size, current.modified, current.identity),
                "The original has changed since import; rescan before deleting"
            );
            Ok(())
        })()
        .with_context(|| relative.display().to_string())?;
        Ok(path)
    }

    pub fn trash_photos(
        &self,
        plan: &TrashPlan,
        mut recycle: impl FnMut(&Path) -> Result<()>,
    ) -> Result<TrashReport> {
        use anyhow::Context;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_trash_cleanup(id INTEGER PRIMARY KEY CHECK(id=1))",
        )?;
        let mut report = TrashReport::default();
        for &id in &plan.ids {
            // Commit each successful move before starting another irreversible catalog change.
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            let generation: i64 = tx.query_row("SELECT generation FROM state", [], |r| r.get(0))?;
            if generation != plan.generation + report.removed as i64 {
                report
                    .errors
                    .push("カタログが変更されました。対象を確認してやり直してください。".into());
                break;
            }
            let path = match self.original_for_trash(id) {
                Ok(path) => path,
                Err(error) => {
                    if report.errors.len() < 3 {
                        report.errors.push(format!("#{id}: {error:#}"));
                    }
                    continue;
                }
            };
            ensure!(
                tx.execute("DELETE FROM items WHERE id=?", [id])? == 1,
                "Photo not found"
            );
            tx.execute("UPDATE state SET generation=generation+1", [])?;
            // Commit the cleanup request with the row deletion so an interrupted sweep is retried.
            tx.execute("INSERT OR IGNORE INTO pending_trash_cleanup VALUES(1)", [])?;
            let moved = recycle(&path).and_then(|()| {
                ensure!(!path.try_exists()?, "The original is still on disk");
                Ok(())
            });
            if let Err(error) = moved {
                if report.errors.len() < 3 {
                    report.errors.push(format!("{}: {error:#}", path.display()));
                }
                continue;
            }
            tx.commit()
                .context("原本をゴミ箱に移動しましたが、カタログの更新に失敗しました。")?;
            report.removed += 1;
        }
        report.failed = plan.ids.len() - report.removed;
        Ok(report)
    }

    pub fn finish_trash_cleanup(&self, cleanup: impl FnOnce(&Self) -> Result<()>) -> Result<()> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='pending_trash_cleanup')",
            [],
            |r| r.get(0),
        )?;
        if exists {
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            if tx.execute("DELETE FROM pending_trash_cleanup WHERE id=1", [])? != 0 {
                cleanup(self)?;
            }
            tx.commit()?;
        }
        Ok(())
    }
}

impl Catalog {
    pub fn remove_catalog_items(
        &self,
        selection: &Removal,
        cleanup: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let count = match selection {
            Removal::Parent { path, root_ids } => {
                self.check_parent(path, root_ids)?;
                let mut count = 0;
                for &id in root_ids {
                    count += remove_root(&tx, id)?;
                }
                count
            }
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
                if path.is_empty() {
                    remove_root(&tx, *root_id)?
                } else {
                    let prefix = format!("{path}{}", std::path::MAIN_SEPARATOR);
                    tx.execute(
                        "DELETE FROM items WHERE root_id=? AND substr(path,1,length(?))=?",
                        params![root_id, prefix, prefix],
                    )?
                }
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
    fn disk_removal_targets_only_registered_unchanged_files_in_the_selected_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        for folder in ["trip", "trip/nested", "trip-other"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("photo.jpg"), b"original").unwrap();
        }
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root_id = catalog.add_root(&root).unwrap().id;
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        std::fs::write(root.join("trip/not-imported.jpg"), b"unregistered").unwrap();
        std::fs::write(root.join("trip/photo.jpg"), b"replacement original").unwrap();
        let plan = catalog
            .plan_trash(&Removal::Folder {
                root_id,
                path: "trip".into(),
            })
            .unwrap();
        assert_eq!(plan.ids.len(), 2);
        let skipped = catalog.trash_photos(&plan, |_| Ok(())).unwrap();
        assert_eq!((skipped.removed, skipped.failed), (0, 2));
        let report = catalog
            .trash_photos(&plan, |path| {
                assert_eq!(path, root.join("trip/nested/photo.jpg").canonicalize()?);
                Ok(std::fs::rename(path, dir.path().join("recycled"))?)
            })
            .unwrap();
        assert_eq!((report.removed, report.failed), (1, 1));
        assert!(root.join("trip/nested").is_dir());
        assert!(
            root.join("trip-other/photo.jpg").is_file()
                && root.join("trip/not-imported.jpg").is_file()
        );
        assert_eq!(
            std::fs::read(root.join("trip/photo.jpg")).unwrap(),
            b"replacement original"
        );
    }

    #[test]
    fn parent_menu_only_targets_the_selected_registered_children() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("photos");
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let mut ids = Vec::new();
        for folder in [
            parent.join("A"),
            parent.join("B"),
            dir.path().join("other/photos/A"),
        ] {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("sample.jpg"), b"original").unwrap();
            ids.push(catalog.add_root(&folder).unwrap().id);
        }
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let selection = |root_ids| Removal::Parent {
            path: parent.canonicalize().unwrap().to_str().unwrap().into(),
            root_ids,
        };
        let wrong = selection(ids.clone());
        assert!(catalog.plan_trash(&wrong).is_err());
        assert!(catalog.remove_catalog_items(&wrong, |_| Ok(())).is_err());
        let group = selection(ids[..2].to_vec());
        let plan = catalog.plan_trash(&group).unwrap();
        assert_eq!(plan.ids.len(), 2);
        assert_eq!(catalog.remove_catalog_items(&group, |_| Ok(())).unwrap(), 2);
        assert_eq!(catalog.roots().unwrap()[0].id, ids[2]);
        assert_eq!(catalog.count(&Query::default()).unwrap(), 1);
        assert!(parent.join("A/sample.jpg").is_file() && parent.join("B/sample.jpg").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn disk_removal_rejects_redirected_originals_even_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir(&root).unwrap();
        let original = root.join("photo.jpg");
        std::fs::write(&original, b"photo").unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root_id = catalog.add_root(&root).unwrap().id;
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let target = root.join("other.jpg");
        std::fs::rename(&original, &target).unwrap();
        std::os::unix::fs::symlink(&target, &original).unwrap();
        let plan = catalog
            .plan_trash(&Removal::Folder {
                root_id,
                path: String::new(),
            })
            .unwrap();
        let report = catalog
            .trash_photos(&plan, |_| panic!("followed symlink"))
            .unwrap();
        assert_eq!((report.removed, report.failed), (0, 1));
        assert!(target.exists() && original.is_symlink());
    }

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
