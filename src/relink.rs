//! Reconnect catalog entries in place. The search never imports or writes files.
use crate::{Catalog, FileRecord, Root, file_record};
use anyhow::{Context, Result, ensure};
use rusqlite::{TransactionBehavior, params};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Default, Serialize)]
pub struct RelinkReport {
    pub linked: usize,
    pub not_found: usize,
    pub conflicts: usize,
    pub unreadable: usize,
    pub first_id: Option<i64>,
}

struct Original {
    id: i64,
    path: String,
    identity: String,
    size: i64,
    modified: i64,
    missing: bool,
}

impl Original {
    fn matches(&self, file: &FileRecord) -> bool {
        self.size == file.size && self.modified == file.modified
    }
}

impl Catalog {
    pub fn relink_folder(
        &mut self,
        root_id: i64,
        folder: &Path,
        search: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<RelinkReport> {
        self.relink_folder_with_previews(root_id, folder, search, cancelled, |_, _| None)
    }

    pub fn relink_folder_with_previews(
        &mut self,
        root_id: i64,
        folder: &Path,
        search: &Path,
        cancelled: impl Fn() -> bool,
        reuse_preview: impl Fn(&crate::previews::PreviewRecord, &Path) -> Option<String>,
    ) -> Result<RelinkReport> {
        ensure!(
            folder
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
            "Invalid catalog folder"
        );
        let search = search.canonicalize()?;
        ensure!(search.is_dir(), "検索先のフォルダが見つかりません。");
        let search_root = Root {
            id: 0,
            path: search.to_str().context("non-UTF8 search path")?.into(),
        };
        let old_root = self
            .roots()?
            .into_iter()
            .find(|r| r.id == root_id)
            .context("登録フォルダが見つかりません。")?;
        let generation: i64 = self
            .conn
            .query_row("SELECT generation FROM state", [], |r| r.get(0))?;
        let wanted = self.missing_originals(&old_root, folder, &cancelled)?;
        let mut report = RelinkReport::default();
        if wanted.is_empty() {
            return Ok(report);
        }

        let (candidates, claims) =
            find_candidates(&search_root, &old_root, &wanted, &mut report, &cancelled)?;
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        ensure!(
            tx.query_row("SELECT generation FROM state", [], |r| r.get::<_, i64>(0))? == generation,
            "検索中にカタログが変更されました。もう一度検索してください。"
        );
        let mut roots = self.roots()?;
        // Add shallower destination roots first, so nested photos reuse the same root.
        let mut originals: Vec<_> = wanted.into_values().flatten().collect();
        originals.sort_by_key(|original| {
            let depth = candidates
                .get(&original.id)
                .and_then(|paths| paths.first())
                .map(|file| {
                    preferred_root(file, &search, Path::new(&original.path))
                        .components()
                        .count()
                });
            (depth, original.id)
        });
        for original in originals {
            ensure!(!cancelled(), "原本の検索を中止しました。");
            let Some(paths) = candidates.get(&original.id) else {
                report.not_found += 1;
                continue;
            };
            if paths.len() != 1 || claims[&paths[0]] != 1 {
                report.conflicts += 1;
                continue;
            }
            let path = &paths[0];
            let old_path = Path::new(&old_root.path).join(&original.path);
            if old_path != *path && old_path.try_exists()? {
                report.conflicts += 1;
                continue;
            }
            // Recheck the file after walking, including any directory now replaced by a symlink.
            if path.canonicalize().ok().as_ref() != Some(path) {
                report.not_found += 1;
                continue;
            }
            let file = file_record(&search_root, path, String::new())?;
            if !original.matches(&file) {
                report.not_found += 1;
                continue;
            }
            // An unreadable subtree might hide a second metadata-only match.
            if report.unreadable > 0 && original.identity != file.identity {
                report.conflicts += 1;
                continue;
            }
            let preferred = preferred_root(path, &search, Path::new(&original.path));
            let Some(destination) = destination_root(path, preferred, &roots) else {
                report.conflicts += 1;
                continue;
            };
            let destination_id =
                if let Some(root) = roots.iter().find(|r| Path::new(&r.path) == destination) {
                    root.id
                } else {
                    let text = destination.to_str().context("non-UTF8 destination")?;
                    tx.execute("INSERT INTO roots(path) VALUES(?)", [text])?;
                    let id = tx.last_insert_rowid();
                    roots.push(Root {
                        id,
                        path: text.into(),
                    });
                    id
                };
            let relative = path
                .strip_prefix(destination)?
                .to_str()
                .context("non-UTF8 photo path")?;
            let old_stamp = self.preview_stamp(original.id)?;
            let reusable = self
                .library_preview(original.id)?
                .filter(|r| r.stamp == old_stamp && r.error.is_none())
                .and_then(|record| reuse_preview(&record, path));
            let changed = tx.execute(
                "UPDATE OR IGNORE items SET root_id=?,path=?,identity=?,missing=0
                WHERE id=? AND root_id=? AND path=? AND identity=? AND size=? AND modified=?",
                params![
                    destination_id,
                    relative,
                    file.identity,
                    original.id,
                    root_id,
                    original.path,
                    original.identity,
                    original.size,
                    original.modified
                ],
            )?;
            if changed == 0 {
                report.conflicts += 1;
                continue;
            }
            let new_stamp = self.preview_stamp(original.id)?;
            if let Some(data) = reusable {
                tx.execute(
                    "UPDATE library_previews SET stamp=?, data=? WHERE item_id=? AND stamp=?",
                    params![new_stamp, data, original.id, old_stamp],
                )?;
            } else {
                // A different file can reappear at precisely the old path with the same size and time.
                tx.execute(
                    "UPDATE library_previews SET data=json_set(data,'$.version','')
                    WHERE item_id=? AND stamp=? AND json_valid(data)",
                    params![original.id, new_stamp],
                )?;
            }
            report.linked += 1;
            report.first_id.get_or_insert(original.id);
        }
        if report.linked > 0 {
            if !Path::new(&old_root.path).try_exists()? {
                tx.execute("DELETE FROM roots WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM items WHERE root_id=?1)", [root_id])?;
            }
            tx.execute("UPDATE state SET generation=generation+1", [])?;
        }
        ensure!(!cancelled(), "原本の検索を中止しました。");
        tx.commit()?;
        Ok(report)
    }

    fn missing_originals(
        &self,
        root: &Root,
        folder: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<HashMap<String, Vec<Original>>> {
        let mut wanted = HashMap::<String, Vec<Original>>::new();
        let mut statement = self.conn.prepare(
            "SELECT id,path,name,identity,size,modified,missing FROM items WHERE root_id=?",
        )?;
        let rows = statement.query_map([root.id], |r| {
            Ok((
                r.get::<_, String>(2)?,
                Original {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    identity: r.get(3)?,
                    size: r.get(4)?,
                    modified: r.get(5)?,
                    missing: r.get(6)?,
                },
            ))
        })?;
        for row in rows {
            ensure!(!cancelled(), "原本の検索を中止しました。");
            let (name, original) = row?;
            if Path::new(&original.path).starts_with(folder)
                && (original.missing || !Path::new(&root.path).join(&original.path).try_exists()?)
            {
                wanted.entry(name).or_default().push(original);
            }
        }
        Ok(wanted)
    }
}

type CandidatePaths = HashMap<i64, Vec<PathBuf>>;
type ClaimCounts = HashMap<PathBuf, usize>;

fn find_candidates(
    search_root: &Root,
    old_root: &Root,
    wanted: &HashMap<String, Vec<Original>>,
    report: &mut RelinkReport,
    cancelled: &impl Fn() -> bool,
) -> Result<(CandidatePaths, ClaimCounts)> {
    let mut candidates = HashMap::<i64, Vec<PathBuf>>::new();
    let mut claims = HashMap::<PathBuf, usize>::new();
    for entry in WalkDir::new(&search_root.path).follow_links(false) {
        ensure!(!cancelled(), "原本の検索を中止しました。");
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.depth() == 0 => return Err(error.into()),
            Err(_) => {
                report.unreadable += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(originals) = entry.file_name().to_str().and_then(|name| wanted.get(name)) else {
            continue;
        };
        let path = entry.path();
        let Ok(record) = file_record(search_root, path, String::new()) else {
            report.unreadable += 1;
            continue;
        };
        for original in originals.iter().filter(|o| o.matches(&record)) {
            // A catalog-marked missing file can already be back at its old path.
            let previous = Path::new(&old_root.path).join(&original.path);
            if original.missing && previous != path && previous.try_exists()? {
                continue;
            }
            candidates
                .entry(original.id)
                .or_default()
                .push(path.to_owned());
            *claims.entry(path.to_owned()).or_default() += 1;
        }
    }

    Ok((candidates, claims))
}

// Preserve as much of the old relative path as still exists under the chosen folder.
fn preferred_root<'a>(file: &'a Path, search: &Path, old_relative: &Path) -> &'a Path {
    let matching = file
        .strip_prefix(search)
        .unwrap()
        .components()
        .rev()
        .zip(old_relative.components().rev())
        .take_while(|(a, b)| a == b)
        .count();
    file.ancestors().nth(matching.max(1)).unwrap()
}

// Reuse a registered root. Otherwise keep the matching subtree, without registering
// an entire search disk or overlapping a sibling root already in the catalog.
fn destination_root<'a>(file: &'a Path, preferred: &Path, roots: &[Root]) -> Option<&'a Path> {
    let parent = file.parent()?;
    if let Some(existing) = parent
        .ancestors()
        .find(|p| roots.iter().any(|r| Path::new(&r.path) == *p))
    {
        return Some(existing);
    }
    parent
        .ancestors()
        .take_while(|p| p.starts_with(preferred))
        .take_while(|p| !roots.iter().any(|r| Path::new(&r.path).starts_with(p)))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;
    use std::{fs, sync::atomic::AtomicBool};

    #[test]
    fn moved_photos_keep_their_edits_and_unrelated_files_are_not_imported() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let base = temporary.path().canonicalize()?;
        let old = base.join("old");
        fs::create_dir_all(old.join("set/nested"))?;
        for name in [
            "set/a.jpg",
            "set/nested/b.jpg",
            "set/twin.jpg",
            "set/gone.jpg",
            "set/wrong.jpg",
            "set/live.jpg",
            "outside.jpg",
        ] {
            fs::write(old.join(name), name)?;
        }
        let mut catalog = Catalog::open(base.join("catalog.db"))?;
        let root = catalog.add_root(&old)?;
        catalog.scan(&AtomicBool::new(false), |_| {})?;
        let before = catalog.page(&Query::default(), 100, None)?.items;
        let photo = before.iter().find(|i| i.name == "b.jpg").unwrap();
        catalog.set_attributes(&[photo.id], Some(5), Some(1), None)?;
        let recipe = r#"{"version":1,"exposure":1}"#;
        catalog.save_develop_recipe(photo.id, recipe, 0)?;
        let search = base.join("search");
        let new = search.join("renamed/photos");
        fs::create_dir_all(&new)?;
        for name in ["a.jpg", "nested/b.jpg", "twin.jpg"] {
            fs::rename(
                old.join("set").join(name),
                new.join(Path::new(name).file_name().unwrap()),
            )?;
        }
        fs::remove_file(old.join("set/gone.jpg"))?;
        fs::remove_file(old.join("set/wrong.jpg"))?;
        fs::write(new.join("wrong.jpg"), "different")?;
        fs::rename(old.join("outside.jpg"), new.join("outside.jpg"))?;
        fs::hard_link(new.join("twin.jpg"), search.join("twin.jpg"))?;
        fs::write(new.join("extra.jpg"), "unrelated")?;
        let result = catalog.relink_folder(root.id, Path::new("set"), &search, || false)?;
        assert_eq!(
            (result.linked, result.conflicts, result.not_found),
            (2, 1, 2)
        );
        assert_eq!(catalog.source_path(photo.id)?, new.join("b.jpg"));
        assert_eq!(catalog.detail(photo.id)?.item.rating, 5);
        assert_eq!(catalog.develop_recipe(photo.id)?.0.as_deref(), Some(recipe));
        assert_eq!(catalog.develop_history(photo.id, None)?.len(), 2);
        assert_eq!(catalog.count(&Query::default())?, before.len() as i64);
        assert_eq!(
            catalog
                .relink_folder(root.id, Path::new("set"), &search, || false)?
                .linked,
            0
        );
        Ok(())
    }

    #[test]
    fn copied_tree_reconnects_after_cancellation_and_survives_reopening() -> Result<()> {
        use crate::removal::Removal;
        let temp = tempfile::tempdir()?;
        let old = temp.path().join("old");
        let search = temp.path().join("disk");
        let moved = search.join("archive/renamed");
        let relative = Path::new("nested/photo.jpg");
        fs::create_dir_all(old.join("nested"))?;
        fs::create_dir_all(moved.join("nested"))?;
        fs::write(old.join(relative), b"original")?;
        let db = temp.path().join("catalog.db");
        let mut catalog = Catalog::open(&db)?;
        let root = catalog.add_root(&old)?;
        catalog.scan(&AtomicBool::new(false), |_| {})?;
        let id = catalog.page(&Query::default(), 1, None)?.items[0].id;
        let selection = Removal::Photos { ids: vec![id] };
        let old_plan = catalog.plan_trash(&selection)?;
        fs::copy(old.join(relative), moved.join(relative))?;
        fs::OpenOptions::new()
            .write(true)
            .open(moved.join(relative))?
            .set_modified(fs::metadata(old.join(relative))?.modified()?)?;
        fs::remove_dir_all(&old)?;
        assert!(
            catalog
                .relink_folder(root.id, Path::new(""), &search, || true)
                .is_err()
        );
        assert_eq!(catalog.roots()?.len(), 1);
        assert!(catalog.source_path(id).is_err());
        let result = catalog.relink_folder(root.id, Path::new(""), &search, || false)?;
        assert_eq!((result.linked, result.first_id), (1, Some(id)));
        drop(catalog);
        let catalog = Catalog::open(db)?;
        let roots = catalog.roots()?;
        assert_eq!(roots.len(), 1);
        assert_eq!(Path::new(&roots[0].path), moved.canonicalize()?);
        assert_eq!(
            catalog.source_path(id)?,
            moved.join(relative).canonicalize()?
        );
        let stale = catalog.trash_photos(&old_plan, |_| {
            panic!("deleted a relinked photo without confirming its new location")
        })?;
        assert_eq!(stale.removed, 0);
        let confirmed = catalog.plan_trash(&selection)?;
        let report = catalog.trash_photos(&confirmed, |path| {
            assert_eq!(path, moved.join(relative).canonicalize()?);
            Ok(fs::rename(path, temp.path().join("recycled"))?)
        })?;
        assert_eq!((report.removed, report.failed), (1, 0));
        Ok(())
    }

    #[test]
    fn missing_entries_reuse_existing_root_and_never_replace_a_registered_photo() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let old = dir.path().join("old");
        let new = dir.path().join("registered");
        fs::create_dir(&old)?;
        fs::create_dir(&new)?;
        fs::write(old.join("occupied.jpg"), b"original")?;
        fs::write(old.join("recover.jpg"), b"another")?;
        fs::hard_link(old.join("occupied.jpg"), new.join("occupied.jpg"))?;
        let mut catalog = Catalog::open(dir.path().join("catalog.db"))?;
        let old_root = catalog.add_root(&old)?;
        let new_root = catalog.add_root(&new)?;
        catalog.scan(&AtomicBool::new(false), |_| {})?;
        fs::remove_file(old.join("occupied.jpg"))?;
        fs::rename(old.join("recover.jpg"), new.join("recover.jpg"))?;
        catalog.scan_root(old_root.id, &AtomicBool::new(false), |_| {})?;
        assert_eq!(catalog.count(&Query::default())?, 1);
        let report = catalog.relink_folder(old_root.id, Path::new(""), &new, || false)?;
        assert_eq!((report.linked, report.conflicts), (1, 1));
        assert_eq!(
            catalog.detail(report.first_id.unwrap())?.item.root_id,
            new_root.id
        );
        assert_eq!(catalog.count(&Query::default())?, 2);
        Ok(())
    }
}
