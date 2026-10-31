//! Persistent preview state uses catalog attributes without touching network originals.
use crate::{Catalog, now};
use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

pub struct PreviewRecord {
    pub stamp: String,
    pub data: Option<String>,
    pub error: Option<String>,
}

impl Catalog {
    pub fn library_preview_records(&self) -> Result<Vec<PreviewRecord>> {
        Ok(self.conn.prepare("SELECT stamp,data,error FROM library_previews")?
            .query_map([], |row| Ok(PreviewRecord {
                stamp: row.get(0)?, data: row.get(1)?, error: row.get(2)?,
            }))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn preview_update(&self, id: i64) -> Result<Option<(i64, bool)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision,ready FROM develop_preview_updates WHERE item_id=?",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }
    pub fn release_preview_updates(&self, active_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE develop_preview_updates SET ready=(item_id!=?)",
            [active_id],
        )?;
        Ok(())
    }
    pub fn pending_preview_updates(&self) -> Result<Vec<(i64, i64)>> {
        Ok(self.conn.prepare("SELECT item_id,revision FROM develop_preview_updates WHERE ready=1 ORDER BY item_id LIMIT 32")?.query_map([], |row| Ok((row.get(0)?,row.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn complete_preview_update(&self, id: i64, revision: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM develop_preview_updates WHERE item_id=? AND revision=? AND ready=1",
            params![id, revision],
        )?;
        Ok(())
    }
    pub fn refresh_preview_source(&self, id: i64) -> Result<bool> {
        let path = self.source_path(id)?;
        let item = self.detail(id)?.item;
        let root = self
            .roots()?
            .into_iter()
            .find(|root| root.id == item.root_id)
            .ok_or_else(|| anyhow::anyhow!("登録フォルダが見つかりません。"))?;
        let record = crate::file_record(&root, &path, item.extension)?;
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute("UPDATE items SET size=?,modified=?,identity=? WHERE id=? AND (size!=? OR modified!=? OR identity!=?)",params![record.size,record.modified,record.identity,id,record.size,record.modified,record.identity])? != 0;
        if changed {
            tx.execute("UPDATE state SET generation=generation+1", [])?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn preview_stamp(&self, id: i64) -> Result<String> {
        let values = self.conn.query_row(
            "SELECT r.path,i.path,i.size,i.modified,i.identity,COALESCE(d.revision,0) FROM items i JOIN roots r ON r.id=i.root_id LEFT JOIN develop_edits d ON d.item_id=i.id WHERE i.id=?",
            [id],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?)),
        )?;
        Ok(serde_json::to_string(&values)?)
    }

    pub fn library_preview(&self, id: i64) -> Result<Option<PreviewRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT stamp,data,error FROM library_previews WHERE item_id=?",
                [id],
                |r| {
                    Ok(PreviewRecord {
                        stamp: r.get(0)?,
                        data: r.get(1)?,
                        error: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_library_preview(
        &self,
        id: i64,
        stamp: &str,
        data: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool> {
        // Reserve the writer before reading the revision; WAL read upgrades cannot wait.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if self.preview_stamp(id)? != stamp {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO library_previews(item_id,stamp,data,error,updated) VALUES(?,?,?,?,?) ON CONFLICT(item_id) DO UPDATE SET stamp=excluded.stamp,data=excluded.data,error=excluded.error,updated=excluded.updated",
            params![id,stamp,data,error,now()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn preview_items(&self, root_id: Option<i64>, after: i64) -> Result<Vec<i64>> {
        Ok(self.conn.prepare("SELECT id FROM items WHERE missing=0 AND id>? AND (? IS NULL OR root_id=?) ORDER BY id LIMIT 200")?
            .query_map(params![after,root_id,root_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Select startup work from catalog records only, without opening originals or preview files.
    pub fn pending_library_preview_items(
        &self,
        root_id: Option<i64>,
        version: &str,
        render_version: &str,
        raw_render_version: &str,
    ) -> Result<Vec<i64>> {
        let raw_extensions = serde_json::to_string(crate::RAW_EXTENSIONS)?;
        Ok(self.conn.prepare(
            "WITH recorded AS (
                SELECT i.id,i.extension,i.path,i.size,i.modified,i.identity,r.path AS root_path,
                    COALESCE(d.revision,0) AS revision,d.item_id AS edited,
                    p.error,
                    CASE WHEN json_valid(p.stamp) THEN p.stamp END AS stamp,
                    CASE WHEN json_valid(p.data) THEN p.data END AS data
                FROM items i JOIN roots r ON r.id=i.root_id
                LEFT JOIN develop_edits d ON d.item_id=i.id
                LEFT JOIN library_previews p ON p.item_id=i.id
                WHERE i.missing=0 AND (?1 IS NULL OR i.root_id=?1)
            )
            SELECT id FROM recorded WHERE
                error IS NOT NULL
                OR json_extract(stamp,'$[0]') IS NOT root_path
                OR json_extract(stamp,'$[1]') IS NOT path
                OR json_extract(stamp,'$[2]') IS NOT size
                OR json_extract(stamp,'$[3]') IS NOT modified
                OR json_extract(stamp,'$[4]') IS NOT identity
                OR json_extract(stamp,'$[5]') IS NOT revision
                OR json_extract(data,'$.version') IS NOT ?2
                OR COALESCE(json_extract(data,'$.extension'),'') NOT IN ('avif','jpg')
                OR COALESCE(json_extract(data,'$.large_bytes'),0)<=0
                OR COALESCE(json_extract(data,'$.thumbnail_bytes'),0)<=0
                OR CASE WHEN extension IN (SELECT value FROM json_each(?5)) THEN
                    json_extract(data,'$.render_version') IS NOT ?4
                    OR (COALESCE(json_extract(data,'$.raw_developed'),0)=0
                        AND COALESCE(json_extract(data,'$.camera_fallback'),0)=0)
                   WHEN edited IS NOT NULL THEN
                    json_extract(data,'$.render_version') IS NOT ?3
                   ELSE 0 END
            ORDER BY id"
        )?.query_map(params![root_id,version,render_version,raw_render_version,raw_extensions], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn startup_candidates_use_catalog_changes_without_opening_photos() {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        for index in 1..=8 {
            catalog.conn.execute("INSERT INTO items(id,root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,?,?,'photo','photo','jpg',1,1,1,'identity')",
                params![index, root.id, format!("not-on-disk-{index}.jpg")]).unwrap();
            let data = r#"{"version":"v1","extension":"jpg","large_bytes":20,"thumbnail_bytes":10,"render_version":"render1"}"#;
            catalog.save_library_preview(index, &catalog.preview_stamp(index).unwrap(), Some(data), None).unwrap();
        }
        let pending = |catalog: &Catalog| catalog.pending_library_preview_items(None, "v1", "render1", "raw1").unwrap();
        assert!(pending(&catalog).is_empty(), "valid records must not require originals or cache files");
        catalog.conn.execute("DELETE FROM library_previews WHERE item_id=1", []).unwrap();
        catalog.conn.execute("UPDATE library_previews SET error='interrupted' WHERE item_id=2", []).unwrap();
        catalog.conn.execute("UPDATE items SET modified=2 WHERE id=3", []).unwrap();
        catalog.save_develop_recipe(4, r#"{"version":1,"exposure":1}"#, 0).unwrap();
        catalog.conn.execute("UPDATE library_previews SET data='broken JSON' WHERE item_id=5", []).unwrap();
        catalog.conn.execute("UPDATE library_previews SET stamp='broken JSON' WHERE item_id=6", []).unwrap();
        catalog.conn.execute("UPDATE library_previews SET data=json_set(data,'$.large_bytes',0) WHERE item_id=7", []).unwrap();
        catalog.conn.execute("UPDATE items SET rating=5,flag=1 WHERE id=8", []).unwrap();
        assert_eq!(pending(&catalog), vec![1,2,3,4,5,6,7]);
        catalog.conn.execute("UPDATE items SET missing=1 WHERE id=1", []).unwrap();
        assert_eq!(pending(&catalog), vec![2,3,4,5,6,7]);
        assert!(catalog.pending_library_preview_items(Some(root.id + 1), "v1", "render1", "raw1").unwrap().is_empty());
    }

    #[test]
    fn startup_candidates_include_changed_render_versions_and_raw_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        for (id, extension) in [(1, "nef"), (2, "jpg")] {
            catalog.conn.execute("INSERT INTO items(id,root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,?,?,'photo','photo',?,1,1,1,'identity')",params![id,root.id,format!("photo-{id}"),extension]).unwrap();
        }
        catalog.save_develop_recipe(2, r#"{"version":1,"exposure":1}"#, 0).unwrap();
        let pending = |catalog: &Catalog| catalog.pending_library_preview_items(None, "v1", "render1", "raw1").unwrap();
        for (id, version) in [(1, "raw1"), (2, "render1")] {
            let data = serde_json::json!({"version":"v1","extension":"jpg","large_bytes":20,"thumbnail_bytes":10,"render_version":version,"raw_developed":true});
            catalog.save_library_preview(id, &catalog.preview_stamp(id).unwrap(), Some(&data.to_string()), None).unwrap();
        }
        assert!(pending(&catalog).is_empty());
        catalog.conn.execute("UPDATE library_previews SET data=json_set(data,'$.render_version','old')", []).unwrap();
        assert_eq!(pending(&catalog), vec![1,2]);
        catalog.conn.execute("UPDATE library_previews SET data=json_set(data,'$.render_version','raw1','$.raw_developed',0,'$.camera_fallback',1) WHERE item_id=1", []).unwrap();
        assert_eq!(pending(&catalog), vec![2], "supported camera fallbacks remain reusable");
        catalog.conn.execute("UPDATE library_previews SET data=json_set(data,'$.camera_fallback',0) WHERE item_id=1", []).unwrap();
        assert_eq!(pending(&catalog), vec![1,2]);
    }

    #[test]
    fn preview_save_waits_for_writer_and_rechecks_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&path).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        catalog.conn.execute("INSERT INTO items(root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,'a.jpg','a.jpg','a.jpg','jpg',1,1,1,'a')", [root.id]).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let stamp = catalog.preview_stamp(id).unwrap();
        let worker = Catalog::open(&path).unwrap();
        catalog.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        catalog
            .save_develop_recipe(id, r#"{"version":1,"exposure":1}"#, 0)
            .unwrap();
        let (started, start) = std::sync::mpsc::channel();
        let (sent, result) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            started.send(()).unwrap();
            sent.send(worker.save_library_preview(id, &stamp, Some("obsolete"), None))
                .unwrap();
        });
        start.recv().unwrap();
        let early = result.recv_timeout(std::time::Duration::from_millis(100));
        catalog.conn.execute_batch("COMMIT").unwrap();
        thread.join().unwrap();
        assert!(
            matches!(early, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
            "Save should wait for the writer, got {early:?}"
        );
        assert!(!result.recv().unwrap().unwrap());
        assert!(catalog.library_preview(id).unwrap().is_none());
    }

    #[test]
    fn undo_save_waits_for_preview_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let catalog = Catalog::open(&path).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        catalog.conn.execute("INSERT INTO items(root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,'a.jpg','a.jpg','a.jpg','jpg',1,1,1,'a')", [root.id]).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let mut worker = Catalog::open(&path).unwrap();
        catalog.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        catalog
            .conn
            .execute(
                "INSERT INTO library_previews VALUES(?,'stamp','{}',NULL,0)",
                [id],
            )
            .unwrap();
        let (started, start) = std::sync::mpsc::channel();
        let (sent, result) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            started.send(()).unwrap();
            sent.send(worker.save_develop_undo(
                id,
                &[crate::HistoryEdit {
                    recipe: r#"{"version":1,"exposure":1}"#.into(),
                    label: "edit".into(),
                }],
                0,
            ))
            .unwrap();
        });
        start.recv().unwrap();
        let early = result.recv_timeout(std::time::Duration::from_millis(100));
        catalog.conn.execute_batch("COMMIT").unwrap();
        thread.join().unwrap();
        assert!(
            matches!(early, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
            "Undo should wait for the writer, got {early:?}"
        );
        assert_eq!(result.recv().unwrap().unwrap(), 1);
        assert_eq!(catalog.develop_recipe(id).unwrap().1, 1);
        assert_eq!(catalog.develop_history(id, None).unwrap().len(), 1);
    }

    #[test]
    fn parallel_preview_saves_undo_and_scans_do_not_lock_each_other_out() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        std::fs::write(photos.join("a.jpg"), b"untouched").unwrap();
        let path = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for role in 0..4 {
                let path = &path;
                let photos = &photos;
                let start = &start;
                scope.spawn(move || {
                    let mut catalog = Catalog::open(path).unwrap();
                    // Sustained writes can exceed the app's lock timeout on slow CI disks.
                    catalog
                        .conn
                        .busy_timeout(std::time::Duration::from_secs(60))
                        .unwrap();
                    start.wait();
                    for revision in 0..200 {
                        match role {
                            0 | 1 => {
                                let stamp = catalog.preview_stamp(id).unwrap();
                                catalog
                                    .save_library_preview(id, &stamp, Some("{}"), None)
                                    .unwrap();
                            }
                            2 => {
                                catalog
                                    .save_develop_undo(
                                        id,
                                        &[crate::HistoryEdit {
                                            recipe: format!(
                                                r#"{{"version":1,"exposure":{}}}"#,
                                                revision % 5
                                            ),
                                            label: "edit".into(),
                                        }],
                                        revision,
                                    )
                                    .unwrap();
                            }
                            _ => {
                                catalog.add_roots(std::slice::from_ref(photos)).unwrap();
                                assert_eq!(
                                    catalog
                                        .scan(&AtomicBool::new(false), |_| {})
                                        .unwrap()
                                        .unchanged,
                                    1
                                );
                            }
                        }
                    }
                });
            }
        });
        assert_eq!(catalog.develop_recipe(id).unwrap().1, 200);
        assert_eq!(catalog.develop_history(id, None).unwrap().len(), 1);
        assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"untouched");
        assert_eq!(
            catalog
                .conn
                .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn preview_state_survives_offline_sources_and_rejects_obsolete_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("photo.jpg"), b"fixture").unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        catalog.add_root(&root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let stamp = catalog.preview_stamp(id).unwrap();
        assert!(
            catalog
                .save_library_preview(id, &stamp, Some("{}"), None)
                .unwrap()
        );
        std::fs::rename(&root, dir.path().join("offline")).unwrap();
        assert_eq!(catalog.preview_stamp(id).unwrap(), stamp);
        assert_eq!(
            catalog
                .library_preview(id)
                .unwrap()
                .unwrap()
                .data
                .as_deref(),
            Some("{}")
        );
        catalog
            .save_develop_recipe(id, "{\"version\":1,\"exposure\":1}", 0)
            .unwrap();
        assert!(
            !catalog
                .save_library_preview(id, &stamp, Some("obsolete"), None)
                .unwrap()
        );
        assert_ne!(catalog.preview_stamp(id).unwrap(), stamp);
    }

    #[test]
    fn deferred_updates_are_coalesced_persistent_and_revision_checked() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        std::fs::write(photos.join("a.jpg"), b"photo").unwrap();
        let path = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.preview_items(None, 0).unwrap()[0];
        let settings = r#"{"version":1,"exposure":1}"#;
        catalog.save_develop_recipe(id, settings, 0).unwrap();
        catalog.save_develop_recipe(id, settings, 1).unwrap();
        assert!(catalog.pending_preview_updates().unwrap().is_empty());
        catalog.release_preview_updates(id).unwrap();
        assert!(catalog.pending_preview_updates().unwrap().is_empty());
        catalog.release_preview_updates(id + 1).unwrap();
        assert_eq!(catalog.pending_preview_updates().unwrap(), vec![(id, 2)]);
        catalog.complete_preview_update(id, 1).unwrap();
        assert_eq!(catalog.preview_update(id).unwrap(), Some((2, true)));
        catalog.save_develop_recipe(id, settings, 2).unwrap();
        catalog.complete_preview_update(id, 2).unwrap();
        drop(catalog);
        let reopened = Catalog::open(path).unwrap();
        assert_eq!(reopened.preview_update(id).unwrap(), Some((3, false)));
        reopened.release_preview_updates(id + 1).unwrap();
        reopened.complete_preview_update(id, 3).unwrap();
        assert!(reopened.preview_update(id).unwrap().is_none());
    }

    #[test]
    fn preview_jobs_page_through_one_hundred_thousand_catalog_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        let tx = catalog.conn.transaction().unwrap();
        {
            let mut statement = tx.prepare("INSERT INTO items(root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,?,?,?,'jpg',1,1,1,?)").unwrap();
            for id in 0..100_000 {
                let name = format!("photo-{id:06}.jpg");
                statement
                    .execute(params![root.id, name, name, name, id.to_string()])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
        let mut after = 0;
        let mut count = 0;
        loop {
            let ids = catalog.preview_items(Some(root.id), after).unwrap();
            if ids.is_empty() {
                break;
            }
            assert!(ids.len() <= 200 && ids[0] > after);
            count += ids.len();
            after = *ids.last().unwrap();
        }
        assert_eq!(count, 100_000);
    }
}
