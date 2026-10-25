pub mod library;
pub mod previews;
mod workflow;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{
    Connection, Transaction, TransactionBehavior, params, params_from_iter, types::Value,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;
pub use workflow::SavedRecipe;

pub fn normalize(s: &str) -> String {
    s.nfkc().flat_map(char::to_lowercase).collect()
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "arw", "sr2", "srf", "raf", "nef", "nrw", "rw2", "orf", "pef",
    "rwl", "srw", "3fr", "fff", "iiq", "heic", "heif", "hif", "jpg", "jpeg", "png", "tif", "tiff",
    "avif", "psd", "psb",
];
pub const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "arw", "sr2", "srf", "raf", "nef", "nrw", "rw2", "orf", "pef",
    "rwl", "srw", "3fr", "fff", "iiq",
];
pub fn supported(s: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&s)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    pub id: i64,
    pub path: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Folder {
    pub root_id: i64,
    pub path: String,
    pub count: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub id: i64,
    pub root_id: i64,
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size: i64,
    pub modified: i64,
    pub added: i64,
    pub missing: bool,
    pub rating: i64,
    pub flag: i8,
    pub label: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Detail {
    pub item: Item,
    pub tags: Vec<String>,
    pub memo: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Query {
    pub ids: Vec<i64>,
    pub text: String,
    pub extensions: Vec<String>,
    pub rating: Option<u8>,
    pub rating_mode: String,
    pub flags: Vec<i8>,
    pub labels: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, Vec<String>>,
    pub root_id: Option<i64>,
    pub folder: Option<String>,
    pub tags: Vec<String>,
    pub added_from: Option<i64>,
    pub added_to: Option<i64>,
    pub include_missing: bool,
    pub sort: Sort,
    pub descending: bool,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Name,
    Added,
    Modified,
    Size,
}
impl Sort {
    fn column(self) -> &'static str {
        match self {
            Self::Name => "name_norm",
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Size => "size",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    generation: i64,
    query: Query,
    key: serde_json::Value,
    id: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<Cursor>,
    pub generation: i64,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub visited: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub missing: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub tags: Vec<String>,
    pub rating: u8,
    pub memo: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationPatch {
    pub tags: Option<Vec<String>>,
    pub rating: Option<u8>,
    pub memo: Option<String>,
}

pub struct Catalog {
    conn: Connection,
}
#[derive(Debug, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub label: String,
    pub created: i64,
}
pub struct HistoryEdit {
    pub recipe: String,
    pub label: String,
}
const CATALOG_APPLICATION_ID: i64 = 0x4f524f4d; // OROM

impl Catalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Identify the format before any journal-mode change or migration.
        let application_id: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            application_id == 0 || application_id == CATALOG_APPLICATION_ID,
            "This is not an OpenRoom catalog"
        );
        ensure!(
            (0..=9).contains(&version),
            "unsupported database schema {version}"
        );
        if version == 0 {
            let tables: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            ensure!(tables == 0, "This is not an OpenRoom catalog");
        } else if application_id == 0 {
            // Older OpenRoom catalogs have no application_id.
            ensure!(version <= 3, "Unrecognized catalog format");
            conn.prepare("SELECT i.id,i.root_id,i.path,i.identity,i.rating,i.tags,i.memo,r.path,s.generation,t.tag FROM items i,roots r,state s,item_tags t LIMIT 0")?;
        }
        if version > 0 && version < 8 {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(".before-previews.openroom");
            let backup = path.with_file_name(name);
            if !backup.exists() {
                let temporary = tempfile::NamedTempFile::new_in(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?;
                conn.execute(
                    "VACUUM INTO ?",
                    [temporary
                        .path()
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("non-UTF8 catalog path"))?],
                )?;
                temporary.as_file().sync_all()?;
                temporary.persist_noclobber(backup)?;
            }
        }
        if version > 0 && version < 9 {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(".before-history-limit.openroom");
            let backup = path.with_file_name(name);
            if !backup.exists() {
                let temporary = tempfile::NamedTempFile::new_in(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?;
                conn.execute(
                    "VACUUM INTO ?",
                    [temporary
                        .path()
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("non-UTF8 catalog path"))?],
                )?;
                temporary.as_file().sync_all()?;
                temporary.persist_noclobber(backup)?;
            }
        }
        conn.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        if version == 0 {
            conn.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL);
INSERT INTO state VALUES(1,0);
CREATE TABLE roots(id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE);
CREATE TABLE items(id INTEGER PRIMARY KEY AUTOINCREMENT,root_id INTEGER NOT NULL REFERENCES roots(id),path TEXT NOT NULL,name TEXT NOT NULL,name_norm TEXT NOT NULL,extension TEXT NOT NULL,size INTEGER NOT NULL,modified INTEGER NOT NULL,added INTEGER NOT NULL,identity TEXT NOT NULL,missing INTEGER NOT NULL DEFAULT 0,rating INTEGER NOT NULL DEFAULT 0 CHECK(rating BETWEEN 0 AND 5),tags TEXT NOT NULL DEFAULT '[]',tags_norm TEXT NOT NULL DEFAULT '',memo TEXT NOT NULL DEFAULT '',memo_norm TEXT NOT NULL DEFAULT '',UNIQUE(root_id,path));
CREATE TABLE item_tags(item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,tag TEXT NOT NULL,PRIMARY KEY(item_id,tag));
CREATE INDEX tags_lookup ON item_tags(tag,item_id);
CREATE INDEX item_identity ON items(identity);
CREATE INDEX item_name ON items(missing,name_norm,id);
CREATE INDEX item_added ON items(missing,added,id);
CREATE INDEX item_modified ON items(missing,modified,id);
CREATE INDEX item_size ON items(missing,size,id);
CREATE INDEX item_root ON items(root_id,missing);
CREATE INDEX item_extension ON items(extension,missing);
CREATE VIRTUAL TABLE search USING fts5(name_norm,tags_norm,memo_norm,content='items',content_rowid='id',tokenize='trigram');
CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN INSERT INTO search(rowid,name_norm,tags_norm,memo_norm) VALUES(new.id,new.name_norm,new.tags_norm,new.memo_norm); END;
CREATE TRIGGER items_ad AFTER DELETE ON items BEGIN INSERT INTO search(search,rowid,name_norm,tags_norm,memo_norm) VALUES('delete',old.id,old.name_norm,old.tags_norm,old.memo_norm); END;
CREATE TRIGGER items_au AFTER UPDATE OF name_norm,tags_norm,memo_norm ON items BEGIN INSERT INTO search(search,rowid,name_norm,tags_norm,memo_norm) VALUES('delete',old.id,old.name_norm,old.tags_norm,old.memo_norm); INSERT INTO search(rowid,name_norm,tags_norm,memo_norm) VALUES(new.id,new.name_norm,new.tags_norm,new.memo_norm); END;
PRAGMA user_version=1; COMMIT;")?;
        }
        if version < 2 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
CREATE INDEX IF NOT EXISTS item_extension_name ON items(extension,missing,name_norm,id);
PRAGMA user_version=2; COMMIT;",
            )?;
        }
        if version < 3 {
            conn.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS develop_edits(item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,recipe TEXT NOT NULL,revision INTEGER NOT NULL,updated INTEGER NOT NULL);
PRAGMA user_version=3; COMMIT;")?;
        }
        if version < 4 {
            conn.execute_batch(&format!(
                "BEGIN IMMEDIATE; PRAGMA application_id={CATALOG_APPLICATION_ID}; PRAGMA user_version=4; COMMIT;"
            ))?;
        }
        if version < 5 {
            conn.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS develop_history(id INTEGER PRIMARY KEY AUTOINCREMENT,item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,recipe TEXT NOT NULL,label TEXT NOT NULL,created INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS history_item ON develop_history(item_id,id);
INSERT INTO develop_history(item_id,recipe,label,created)
SELECT item_id,recipe,'移行時の設定',updated FROM develop_edits d WHERE NOT EXISTS(SELECT 1 FROM develop_history h WHERE h.item_id=d.item_id);
PRAGMA user_version=5; COMMIT;")?;
        }
        if version < 6 {
            conn.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE saved_recipes(id INTEGER PRIMARY KEY AUTOINCREMENT,item_id INTEGER REFERENCES items(id) ON DELETE CASCADE,name TEXT NOT NULL,recipe TEXT NOT NULL,created INTEGER NOT NULL);
CREATE INDEX saved_recipe_item ON saved_recipes(item_id);
PRAGMA user_version=6; COMMIT;")?;
        }
        if version < 7 {
            conn.execute_batch("BEGIN IMMEDIATE;
ALTER TABLE items ADD COLUMN flag INTEGER NOT NULL DEFAULT 0 CHECK(flag BETWEEN -1 AND 1);
ALTER TABLE items ADD COLUMN label TEXT NOT NULL DEFAULT '';
CREATE INDEX items_flag_rating ON items(flag,rating);
CREATE INDEX items_label ON items(label);
CREATE TABLE photo_metadata(item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,source_size INTEGER NOT NULL,source_modified INTEGER NOT NULL,date TEXT NOT NULL,camera TEXT NOT NULL,lens TEXT NOT NULL,iso TEXT NOT NULL,aperture TEXT NOT NULL,focal TEXT NOT NULL,fields TEXT NOT NULL,error TEXT);
CREATE INDEX metadata_date ON photo_metadata(date,item_id);
CREATE INDEX metadata_camera ON photo_metadata(camera,item_id);
CREATE INDEX metadata_lens ON photo_metadata(lens,item_id);
PRAGMA user_version=7; COMMIT;")?;
        }
        if version < 8 {
            conn.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS library_previews(item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,stamp TEXT NOT NULL,data TEXT,error TEXT,updated INTEGER NOT NULL);
PRAGMA user_version=8; COMMIT;")?;
        }
        if version < 9 {
            conn.execute_batch("BEGIN IMMEDIATE;
DELETE FROM develop_history WHERE id IN (SELECT id FROM (SELECT id,row_number() OVER (PARTITION BY item_id ORDER BY id DESC) AS position FROM develop_history) WHERE position>11);
CREATE TABLE IF NOT EXISTS develop_preview_updates(item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,revision INTEGER NOT NULL,ready INTEGER NOT NULL DEFAULT 0);
PRAGMA user_version=9; COMMIT;")?;
        }
        Ok(Self { conn })
    }
    /// A consistent, compact, standalone snapshot, including committed WAL changes.
    /// Includes preview records; desktop backup also copies their companion images.
    pub fn backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();
        ensure!(
            !destination.exists(),
            "保存先はすでに存在します。別の名前を指定してください。"
        );
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".openroom-")
            .tempfile_in(parent)?;
        self.conn.execute(
            "VACUUM INTO ?",
            [temporary
                .path()
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 catalog path"))?],
        )?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(destination)?;
        Ok(())
    }
    pub fn roots(&self) -> Result<Vec<Root>> {
        Ok(self
            .conn
            .prepare("SELECT id,path FROM roots ORDER BY id")?
            .query_map([], |r| {
                Ok(Root {
                    id: r.get(0)?,
                    path: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn develop_recipe(&self, id: i64) -> Result<(Option<String>, i64)> {
        Ok(self.conn.query_row(
            "SELECT d.recipe,coalesce(d.revision,0) FROM items i LEFT JOIN develop_edits d ON d.item_id=i.id WHERE i.id=?",
            [id],
            |row| {let value=row.get_ref(0)?;Ok((if value==rusqlite::types::ValueRef::Null {None}else{Some(workflow::decode_recipe(value)?)},row.get(1)?))},
        )?)
    }
    pub fn save_develop_recipe(
        &mut self,
        id: i64,
        recipe: &str,
        expected_revision: i64,
    ) -> Result<i64> {
        self.save_develop_steps(
            id,
            &[HistoryEdit {
                recipe: recipe.into(),
                label: "現像設定".into(),
            }],
            expected_revision,
        )
    }
    pub fn develop_history(&self, id: i64, before: Option<i64>) -> Result<Vec<HistoryEntry>> {
        Ok(self.conn.prepare("SELECT id,label,created FROM develop_history WHERE item_id=? AND id<? ORDER BY id DESC LIMIT 11")?
            .query_map(params![id,before.unwrap_or(i64::MAX)], |row| Ok(HistoryEntry { id: row.get(0)?, label: row.get(1)?, created: row.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn history_recipe(&self, id: i64, history_id: i64) -> Result<String> {
        Ok(self.conn.query_row(
            "SELECT recipe FROM develop_history WHERE item_id=? AND id=?",
            params![id, history_id],
            |row| workflow::decode_recipe(row.get_ref(0)?),
        )?)
    }
    pub fn save_develop_steps(
        &mut self,
        id: i64,
        steps: &[HistoryEdit],
        expected_revision: i64,
    ) -> Result<i64> {
        self.write_develop_steps(id, steps, expected_revision, false)
    }
    /// Replace the bounded undo chain and current state in the same revision commit.
    pub fn save_develop_undo(
        &mut self,
        id: i64,
        states: &[HistoryEdit],
        expected_revision: i64,
    ) -> Result<i64> {
        ensure!(
            !states.is_empty() && states.len() <= 11,
            "Invalid undo history"
        );
        self.write_develop_steps(id, states, expected_revision, true)
    }
    fn write_develop_steps(
        &mut self,
        id: i64,
        steps: &[HistoryEdit],
        expected_revision: i64,
        replace: bool,
    ) -> Result<i64> {
        ensure!(
            !steps.is_empty() && steps.len() <= 100,
            "Invalid history batch"
        );
        for step in steps {
            ensure!(
                step.recipe.len() <= 2_097_152 && step.label.len() <= 256,
                "History entry is too large"
            );
            let value: serde_json::Value = serde_json::from_str(&step.recipe)?;
            ensure!(
                value.is_object() && value["version"] == 1,
                "Invalid development recipe"
            );
        }
        let recipe = workflow::encode_recipe(&steps.last().unwrap().recipe)?;
        ensure!(
            (0..i64::MAX).contains(&expected_revision),
            "Invalid recipe revision"
        );
        ensure!(
            self.conn
                .query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?)", [id], |r| r
                    .get::<_, bool>(0))?,
            "Photo is not in this catalog"
        );
        let transaction = self.conn.savepoint()?;
        // Ordinary callers retain the pre-edit state; UI callers supply their complete undo chain.
        if !replace {
            transaction.execute("INSERT INTO develop_history(item_id,recipe,label,created) SELECT ?,coalesce((SELECT recipe FROM develop_edits WHERE item_id=?),'{\"version\":1}'),'編集開始時の設定',? WHERE NOT EXISTS(SELECT 1 FROM develop_history WHERE item_id=?)",params![id,id,now(),id])?;
        }
        if expected_revision == 0 {
            ensure!(
                transaction.execute(
                    "INSERT OR IGNORE INTO develop_edits VALUES(?,?,1,?)",
                    params![id, recipe, now()]
                )? == 1,
                "現像設定が別のウィンドウで更新されました。再読み込みしてください。"
            );
        } else {
            ensure!(transaction.execute("UPDATE develop_edits SET recipe=?,revision=revision+1,updated=? WHERE item_id=? AND revision=?", params![recipe, now(), id, expected_revision])? == 1,
                "現像設定が別のウィンドウで更新されました。再読み込みしてください。");
        }
        // Read history after the revision write has acquired the writer lock.
        let existing = if replace {
            transaction
                .prepare("SELECT id,recipe FROM develop_history WHERE item_id=? ORDER BY id")?
                .query_map([id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        workflow::decode_recipe(row.get_ref(1)?)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let mut retained = Vec::new();
        for step in steps {
            if let Some((history_id, _)) = existing.iter().find(|(history_id, recipe)| {
                *history_id > retained.last().copied().unwrap_or(0) && *recipe == step.recipe
            }) {
                retained.push(*history_id);
                continue;
            }
            transaction.execute(
                "INSERT INTO develop_history(item_id,recipe,label,created) VALUES(?,?,?,?)",
                params![
                    id,
                    workflow::encode_recipe(&step.recipe)?,
                    step.label,
                    now()
                ],
            )?;
            retained.push(transaction.last_insert_rowid());
        }
        for (history_id, _) in existing {
            if !retained.contains(&history_id) {
                transaction.execute(
                    "DELETE FROM develop_history WHERE item_id=? AND id=?",
                    params![id, history_id],
                )?;
            }
        }
        transaction.execute("DELETE FROM develop_history WHERE item_id=? AND id NOT IN (SELECT id FROM develop_history WHERE item_id=? ORDER BY id DESC LIMIT 11)", params![id,id])?;
        transaction.execute("INSERT INTO develop_preview_updates(item_id,revision,ready) VALUES(?,?,0) ON CONFLICT(item_id) DO UPDATE SET revision=excluded.revision,ready=0", params![id,expected_revision+1])?;
        transaction.commit()?;
        Ok(expected_revision + 1)
    }
    pub fn add_roots(&self, paths: &[PathBuf]) -> Result<Vec<Root>> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let roots = paths
            .iter()
            .map(|path| self.add_root(path))
            .collect::<Result<_>>()?;
        transaction.commit()?;
        Ok(roots)
    }
    pub fn add_root(&self, path: impl AsRef<Path>) -> Result<Root> {
        let path = std::fs::canonicalize(path)?;
        ensure!(path.is_dir(), "root must be a directory");
        let text = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 root path"))?
            .to_owned();
        for root in self.roots()? {
            let existing = Path::new(&root.path);
            if path == existing {
                return Ok(root);
            }
            ensure!(
                !path.starts_with(existing) && !existing.starts_with(&path),
                "overlapping roots are not allowed"
            );
        }
        self.conn
            .execute("INSERT INTO roots(path) VALUES(?)", [&text])?;
        Ok(Root {
            id: self.conn.last_insert_rowid(),
            path: text,
        })
    }
    pub fn remove_root(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM items WHERE root_id=?", [id])?;
        ensure!(
            tx.execute("DELETE FROM roots WHERE id=?", [id])? == 1,
            "root not found"
        );
        tx.execute("UPDATE state SET generation=generation+1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn scan(
        &mut self,
        cancel: &AtomicBool,
        progress: impl FnMut(&ScanReport),
    ) -> Result<ScanReport> {
        self.scan_roots(self.roots()?, cancel, progress)
    }
    pub fn scan_root(
        &mut self,
        root_id: i64,
        cancel: &AtomicBool,
        progress: impl FnMut(&ScanReport),
    ) -> Result<ScanReport> {
        let root = self
            .roots()?
            .into_iter()
            .find(|root| root.id == root_id)
            .ok_or_else(|| anyhow::anyhow!("root not found"))?;
        self.scan_roots(vec![root], cancel, progress)
    }
    fn scan_roots(
        &mut self,
        roots: Vec<Root>,
        cancel: &AtomicBool,
        mut progress: impl FnMut(&ScanReport),
    ) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        for root in roots {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            let errors_before = report.errors.len();
            let mut files = Vec::new();
            for entry in WalkDir::new(&root.path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let n = e.file_name().to_string_lossy();
                    !n.starts_with('.')
                        && !(e.file_type().is_dir()
                            && ["app", "bundle", "framework", "photoslibrary"]
                                .iter()
                                .any(|ext| n.to_lowercase().ends_with(&format!(".{ext}"))))
                })
            {
                if cancel.load(Ordering::Relaxed) {
                    report.cancelled = true;
                    break;
                }
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        report.errors.push(e.to_string());
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                report.visited += 1;
                let p = entry.path();
                let extension = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !supported(&extension) {
                    continue;
                }
                match file_record(&root, p, extension) {
                    Ok(f) => files.push(f),
                    Err(e) => report.errors.push(format!("{}: {e}", p.display())),
                }
                if report.visited.is_multiple_of(1000) {
                    progress(&report)
                }
            }
            if report.cancelled {
                break;
            }
            let complete = report.errors.len() == errors_before;
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS seen(id INTEGER PRIMARY KEY); DELETE FROM seen;",
            )?;
            let mut changed = false;
            let committed_counts = (report.indexed, report.unchanged);
            for (position, f) in files.into_iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    report.cancelled = true;
                    break;
                }
                let old=tx.prepare_cached("SELECT id,size,modified,identity,missing FROM items WHERE root_id=? AND path=?")?.query_row(params![root.id,f.path],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,bool>(4)?)));
                let id = match old {
                    Ok((id, size, modified, identity, missing)) => {
                        if size == f.size
                            && modified == f.modified
                            && identity == f.identity
                            && !missing
                        {
                            report.unchanged += 1;
                        } else {
                            tx.prepare_cached("UPDATE items SET size=?,modified=?,identity=?,missing=0 WHERE id=?")?.execute(params![f.size,f.modified,f.identity,id])?;
                            report.indexed += 1;
                            changed = true;
                        }
                        id
                    }
                    Err(rusqlite::Error::QueryReturnedNoRows) => {
                        let candidates:Vec<(i64,i64,String,String)>=tx.prepare_cached("SELECT i.id,i.root_id,i.path,r.path FROM items i JOIN roots r ON r.id=i.root_id WHERE identity=?")?.query_map([&f.identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
                        let movable = if candidates.len() == 1 {
                            let c = &candidates[0];
                            match Path::new(&c.3).join(&c.2).try_exists() {
                                Ok(false) => Some(c.0),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let id = if let Some(id) = movable {
                            tx.prepare_cached("UPDATE items SET root_id=?,path=?,name=?,name_norm=?,extension=?,size=?,modified=?,missing=0 WHERE id=?")?.execute(params![root.id,f.path,f.name,normalize(&f.name),f.extension,f.size,f.modified,id])?;
                            id
                        } else {
                            tx.prepare_cached("INSERT INTO items(root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,?,?,?,?,?,?,?,?)")?.execute(params![root.id,f.path,f.name,normalize(&f.name),f.extension,f.size,f.modified,now(),f.identity])?;
                            tx.last_insert_rowid()
                        };
                        report.indexed += 1;
                        changed = true;
                        id
                    }
                    Err(e) => return Err(e.into()),
                };
                tx.prepare_cached("INSERT OR IGNORE INTO seen VALUES(?)")?
                    .execute([id])?;
                if (position + 1).is_multiple_of(1000) {
                    progress(&report);
                }
            }
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
            }
            if report.cancelled {
                (report.indexed, report.unchanged) = committed_counts;
                tx.rollback()?;
                break;
            }
            if complete {
                let n=tx.execute("UPDATE items SET missing=1 WHERE root_id=? AND missing=0 AND id NOT IN(SELECT id FROM seen)",[root.id])?;
                report.missing += n;
                changed |= n > 0;
            }
            if changed {
                tx.execute("UPDATE state SET generation=generation+1", [])?;
            }
            tx.commit()?;
            progress(&report);
        }
        Ok(report)
    }
    pub fn annotate(&mut self, ids: &[i64], annotation: &Annotation) -> Result<()> {
        self.patch_annotations(
            ids,
            &AnnotationPatch {
                tags: Some(annotation.tags.clone()),
                rating: Some(annotation.rating),
                memo: Some(annotation.memo.clone()),
            },
        )
    }
    /// Omitted fields retain their per-photo values, in the same atomic transaction.
    pub fn patch_annotations(&mut self, ids: &[i64], patch: &AnnotationPatch) -> Result<()> {
        ensure!(ids.len() <= 500, "annotation exceeds size limit");
        ensure!(
            patch.rating.is_none_or(|rating| rating <= 5),
            "rating must be 0..5"
        );
        if let Some(tags) = &patch.tags {
            ensure!(
                tags.len() <= 100
                    && tags
                        .iter()
                        .all(|tag| tag.len() <= 256 && !tag.contains('\0')),
                "Invalid tags"
            );
        }
        if let Some(memo) = &patch.memo {
            ensure!(
                memo.len() <= 1024 * 1024 && !memo.contains('\0'),
                "Invalid memo"
            );
        }
        let tags = patch.tags.as_ref().map(|tags| {
            let mut seen = HashSet::new();
            tags.iter()
                .map(|tag| tag.trim())
                .filter(|tag| !tag.is_empty())
                .filter(|tag| seen.insert(normalize(tag)))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
        let tags_json = tags.as_ref().map(serde_json::to_string).transpose()?;
        let norm_tags = tags
            .as_ref()
            .map(|tags| tags.iter().map(|tag| normalize(tag)).collect::<Vec<_>>());
        let search_tags = norm_tags.as_ref().map(|tags| tags.join("\n"));
        let search_memo = patch.memo.as_ref().map(|memo| normalize(memo));
        let tx = self.conn.transaction()?;
        for id in ids {
            ensure!(tx.execute(
                "UPDATE items SET tags=coalesce(?,tags),tags_norm=coalesce(?,tags_norm),rating=coalesce(?,rating),memo=coalesce(?,memo),memo_norm=coalesce(?,memo_norm) WHERE id=?",
                params![tags_json, search_tags, patch.rating, patch.memo, search_memo, id]
            )? == 1, "item not found: {id}");
            if let Some(tags) = &norm_tags {
                tx.execute("DELETE FROM item_tags WHERE item_id=?", [id])?;
                for tag in tags {
                    tx.execute("INSERT INTO item_tags VALUES(?,?)", params![id, tag])?;
                }
            }
        }
        if !ids.is_empty() {
            tx.execute("UPDATE state SET generation=generation+1", [])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Resolve only the location columns; large notes/tags must never delay image requests.
    pub fn source_path(&self, id: i64) -> Result<PathBuf> {
        let (root, relative, missing, extension): (String, String, bool, String) = self.conn.query_row(
            "SELECT r.path,i.path,i.missing,i.extension FROM items i JOIN roots r ON r.id=i.root_id WHERE i.id=?",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        ensure!(
            supported(&extension),
            "写真ライブラリの対応形式ではありません。"
        );
        ensure!(!missing, "ファイルが見つかりません。再走査してください。");
        let base = std::fs::canonicalize(&root).with_context(|| {
            format!("写真の保存先にアクセスできません。ドライブ・共有フォルダの接続を確認してください：{root}")
        })?;
        let path = std::fs::canonicalize(base.join(&relative)).with_context(|| {
            format!(
                "元の写真にアクセスできません。保存場所とアクセス権を確認してください：{relative}"
            )
        })?;
        ensure!(
            path.starts_with(base),
            "登録フォルダの外にはアクセスできません。"
        );
        Ok(path)
    }
    pub fn detail(&self, id: i64) -> Result<Detail> {
        Ok(self.conn.query_row(
            &format!("SELECT {COLUMNS},tags,memo FROM items WHERE id=?"),
            [id],
            |r| {
                Ok(Detail {
                    item: read_item(r)?,
                    tags: serde_json::from_str(&r.get::<_, String>(12)?).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            12,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    memo: r.get(13)?,
                })
            },
        )?)
    }
    pub fn count(&self, q: &Query) -> Result<i64> {
        let (sql, args) = filter(q)?;
        Ok(self.conn.query_row(
            &format!("SELECT count(*) FROM items WHERE {sql}"),
            params_from_iter(args),
            |r| r.get(0),
        )?)
    }
    /// Build folder counts from the catalog without touching mounted drives.
    pub fn folders(&self) -> Result<Vec<Folder>> {
        let (sql, args) = filter(&Query::default())?;
        let mut statement = self
            .conn
            .prepare(&format!("SELECT root_id,path FROM items WHERE {sql}"))?;
        let rows = statement.query_map(params_from_iter(args), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut counts = BTreeMap::new();
        for row in rows {
            let (root_id, path) = row?;
            let mut directory = Path::new(&path).parent();
            while let Some(parent) = directory {
                *counts
                    .entry((root_id, parent.to_string_lossy().into_owned()))
                    .or_insert(0) += 1;
                directory = parent.parent();
            }
        }
        Ok(counts
            .into_iter()
            .map(|((root_id, path), count)| Folder {
                root_id,
                path,
                count,
            })
            .collect())
    }
    pub fn page(&self, q: &Query, limit: usize, cursor: Option<&Cursor>) -> Result<Page> {
        ensure!((1..=500).contains(&limit), "page size must be 1..500");
        let tx = self.conn.unchecked_transaction()?;
        let generation: i64 =
            tx.query_row("SELECT generation FROM state WHERE id=1", [], |r| r.get(0))?;
        let (mut sql, mut args) = filter(q)?;
        let col = q.sort.column();
        let direction = if q.descending { "DESC" } else { "ASC" };
        if let Some(c) = cursor {
            ensure!(
                c.generation == generation && c.query == *q,
                "stale or incompatible cursor; restart pagination"
            );
            let key = match q.sort {
                Sort::Name => Value::Text(
                    c.key
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("invalid cursor key"))?
                        .to_owned(),
                ),
                _ => Value::Integer(
                    c.key
                        .as_i64()
                        .ok_or_else(|| anyhow::anyhow!("invalid cursor key"))?,
                ),
            };
            sql.push_str(&format!(
                " AND ({col},id) {} (?,?)",
                if q.descending { "<" } else { ">" }
            ));
            args.push(key);
            args.push(Value::Integer(c.id));
        }
        args.push(Value::Integer((limit + 1) as i64));
        let mut items:Vec<Item>=tx.prepare(&format!("SELECT {COLUMNS} FROM items WHERE {sql} ORDER BY {col} {direction},id {direction} LIMIT ?"))?.query_map(params_from_iter(args),read_item)?.collect::<rusqlite::Result<_>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next = if more {
            let i = items.last().unwrap();
            Some(Cursor {
                generation,
                query: q.clone(),
                key: match q.sort {
                    Sort::Name => normalize(&i.name).into(),
                    Sort::Added => i.added.into(),
                    Sort::Modified => i.modified.into(),
                    Sort::Size => i.size.into(),
                },
                id: i.id,
            })
        } else {
            None
        };
        tx.commit()?;
        Ok(Page {
            items,
            next,
            generation,
        })
    }
    pub fn check(&self) -> Result<()> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        ensure!(result == "ok", "database integrity: {result}");
        self.conn.execute(
            "INSERT INTO search(search,rank) VALUES('integrity-check',1)",
            [],
        )?;
        Ok(())
    }
}
const COLUMNS: &str =
    "id,root_id,path,name,extension,size,modified,added,missing,rating,flag,label";
fn read_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        root_id: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        extension: r.get(4)?,
        size: r.get(5)?,
        modified: r.get(6)?,
        added: r.get(7)?,
        missing: r.get(8)?,
        rating: r.get(9)?,
        flag: r.get(10)?,
        label: r.get(11)?,
    })
}
fn filter(q: &Query) -> Result<(String, Vec<Value>)> {
    ensure!(
        !q.text.contains('\0') && q.tags.iter().all(|t| !t.contains('\0')),
        "NUL characters are not supported"
    );
    ensure!(q.rating.is_none_or(|r| r <= 5), "rating must be 0..5");
    let mut conditions = vec![if q.include_missing {
        "1=1".to_owned()
    } else {
        "missing=0".to_owned()
    }];
    // Keep legacy asset records and annotations, but exclude them from photo queries.
    conditions.push(format!(
        "extension IN ({})",
        vec!["?"; SUPPORTED_EXTENSIONS.len()].join(",")
    ));
    let mut args: Vec<Value> = SUPPORTED_EXTENSIONS
        .iter()
        .map(|ext| Value::Text((*ext).into()))
        .collect();
    ensure!(
        q.ids.len() <= 10000 && q.ids.iter().all(|id| *id > 0),
        "Invalid photo IDs"
    );
    if !q.ids.is_empty() {
        conditions.push(format!("id IN ({})", vec!["?"; q.ids.len()].join(",")));
        args.extend(q.ids.iter().copied().map(Value::Integer));
    }
    let terms: Vec<String> = normalize(&q.text)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let long: Vec<String> = terms
        .iter()
        .filter(|t| t.chars().count() >= 3 && !t.contains('\0'))
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if !long.is_empty() {
        conditions.push("id IN (SELECT rowid FROM search WHERE search MATCH ?)".into());
        args.push(Value::Text(long.join(" AND ")));
    }
    for term in terms {
        conditions
            .push("(instr(name_norm,?)>0 OR instr(tags_norm,?)>0 OR instr(memo_norm,?)>0)".into());
        args.extend([
            Value::Text(term.clone()),
            Value::Text(term.clone()),
            Value::Text(term),
        ]);
    }
    if !q.extensions.is_empty() {
        ensure!(q.extensions.len() <= 32, "too many extensions");
        conditions.push(format!(
            "extension IN ({})",
            vec!["?"; q.extensions.len()].join(",")
        ));
        args.extend(
            q.extensions
                .iter()
                .map(|s| Value::Text(s.trim_start_matches('.').to_lowercase())),
        );
    }
    ensure!(
        ["", "eq", "gte", "lte"].contains(&q.rating_mode.as_str()),
        "Invalid rating mode"
    );
    if let Some(rating) = q.rating {
        let op = match q.rating_mode.as_str() {
            "gte" => ">=",
            "lte" => "<=",
            _ => "=",
        };
        conditions.push(format!("rating{op}?"));
        args.push(Value::Integer(rating.into()));
    }
    ensure!(
        q.flags.len() <= 3 && q.flags.iter().all(|v| (-1..=1).contains(v)),
        "Invalid flags"
    );
    if !q.flags.is_empty() {
        conditions.push(format!("flag IN ({})", vec!["?"; q.flags.len()].join(",")));
        args.extend(q.flags.iter().map(|v| Value::Integer((*v).into())));
    }
    ensure!(q.labels.len() <= 6, "Too many labels");
    if !q.labels.is_empty() {
        conditions.push(format!(
            "label IN ({})",
            vec!["?"; q.labels.len()].join(",")
        ));
        args.extend(q.labels.iter().cloned().map(Value::Text));
    }
    for (field, values) in &q.metadata {
        ensure!(
            ["date", "camera", "lens", "iso", "aperture", "focal"].contains(&field.as_str())
                && values.len() <= 64,
            "Invalid metadata filter"
        );
        if values.is_empty() {
            continue;
        }
        let mut clauses = Vec::new();
        for value in values {
            if field == "date" && !value.is_empty() {
                clauses.push("substr(date,1,length(?))=?".to_string());
                args.push(Value::Text(value.clone()));
            } else {
                clauses.push(format!("{field}=?"));
            }
            args.push(Value::Text(value.clone()));
        }
        conditions.push(format!("id IN (SELECT m.item_id FROM photo_metadata m WHERE m.source_size=items.size AND m.source_modified=items.modified AND ({}))",clauses.join(" OR ")));
    }
    if let Some(root_id) = q.root_id {
        conditions.push("root_id=?".into());
        args.push(Value::Integer(root_id));
    }
    if let Some(folder) = &q.folder {
        ensure!(q.root_id.is_some(), "folder filtering requires a root");
        ensure!(
            !folder.contains('\0')
                && Path::new(folder)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "folder must be a relative directory without parent components"
        );
        if !folder.is_empty() {
            let prefix = format!("{folder}{}", std::path::MAIN_SEPARATOR);
            conditions.push("substr(path,1,length(?))=?".into());
            args.extend([Value::Text(prefix.clone()), Value::Text(prefix)]);
        }
    }
    for (op, value) in [(">=", q.added_from), ("<=", q.added_to)] {
        if let Some(v) = value {
            conditions.push(format!("added{op}?"));
            args.push(Value::Integer(v));
        }
    }
    for tag in &q.tags {
        conditions.push("id IN (SELECT item_id FROM item_tags WHERE tag=?)".into());
        args.push(Value::Text(normalize(tag.trim())));
    }
    ensure!(args.len() < 900, "too many query parameters");
    Ok((conditions.join(" AND "), args))
}
struct FileRecord {
    path: String,
    name: String,
    extension: String,
    size: i64,
    modified: i64,
    identity: String,
}
fn file_record(root: &Root, p: &Path, extension: String) -> Result<FileRecord> {
    let m = std::fs::metadata(p)?;
    let path = p
        .strip_prefix(&root.path)?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 filename"))?
        .to_owned();
    let name = p.file_name().and_then(|s| s.to_str()).unwrap().to_owned();
    let modified = m
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .min(i64::MAX as u128) as i64;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}",
            m.dev(),
            m.ino(),
            m.created()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    };
    #[cfg(not(unix))]
    let identity = format!("path:{}", p.display());
    if m.len() > i64::MAX as u64 {
        bail!("file too large")
    }
    Ok(FileRecord {
        path,
        name,
        extension,
        size: m.len() as i64,
        modified,
        identity,
    })
}
pub fn absolute_path(root: &Root, item: &Item) -> PathBuf {
    Path::new(&root.path).join(&item.path)
}

#[cfg(test)]
mod development_tests {
    use super::*;
    #[test]
    fn unavailable_photo_location_explains_how_to_reconnect() {
        let directory = tempfile::tempdir().unwrap();
        let photos = directory.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        let photo = photos.join("sample.jpg");
        std::fs::write(&photo, b"photo").unwrap();
        let mut catalog = Catalog::open(directory.path().join("catalog.db")).unwrap();
        let root = catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        std::fs::remove_file(&photo).unwrap();
        assert!(
            catalog
                .source_path(id)
                .unwrap_err()
                .to_string()
                .contains("元の写真にアクセスできません")
        );
        std::fs::remove_dir(&photos).unwrap();
        let message = catalog.source_path(id).unwrap_err().to_string();
        assert!(message.contains("ドライブ・共有フォルダの接続を確認"));
        assert!(message.contains(&root.path));
        std::fs::create_dir(&photos).unwrap();
        std::fs::write(&photo, b"photo").unwrap();
        assert_eq!(
            catalog.source_path(id).unwrap(),
            std::fs::canonicalize(photo).unwrap()
        );
    }
    #[test]
    fn foreign_catalogs_are_rejected_without_modification() {
        let directory = tempfile::tempdir().unwrap();
        for (name, sql) in [
            (
                "lightroom.lrcat",
                "CREATE TABLE Adobe_images(id_local INTEGER PRIMARY KEY);",
            ),
            (
                "other.db",
                "PRAGMA application_id=42; CREATE TABLE photos(id INTEGER);",
            ),
            (
                "unrelated.db",
                "PRAGMA user_version=3; CREATE TABLE photos(id INTEGER);",
            ),
        ] {
            let path = directory.path().join(name);
            Connection::open(&path).unwrap().execute_batch(sql).unwrap();
            let before = std::fs::read(&path).unwrap();
            assert!(Catalog::open(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert!(!directory.path().join(format!("{name}-wal")).exists());
        }
    }

    #[test]
    fn catalog_copy_contains_committed_edits_without_originals_or_wal() {
        let directory = tempfile::tempdir().unwrap();
        let photos = directory.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        let original = photos.join("a.jpg");
        std::fs::write(&original, b"unchanged photo").unwrap();
        let database = directory.path().join("catalog.db");
        let mut catalog = Catalog::open(&database).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        let recipe = r#"{"version":1,"exposure":1.25}"#;
        catalog.save_develop_recipe(id, recipe, 0).unwrap();
        catalog
            .annotate(
                &[id],
                &Annotation {
                    rating: 4,
                    tags: vec!["作品".into()],
                    memo: "メモ".into(),
                },
            )
            .unwrap();
        assert!(directory.path().join("catalog.db-wal").exists());
        let backup = directory.path().join("copy.openroom");
        catalog.backup(&backup).unwrap();
        assert!(!directory.path().join("copy.openroom-wal").exists());
        assert!(!directory.path().join("copy.openroom-shm").exists());
        let saved_bytes = std::fs::read(&backup).unwrap();
        assert!(catalog.backup(&backup).is_err());
        assert!(catalog.backup(&original).is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), saved_bytes);
        assert_eq!(std::fs::read(&original).unwrap(), b"unchanged photo");

        // Copy just the catalog file; no sibling WAL or cache is needed for restoration.
        let restored_directory = tempfile::tempdir().unwrap();
        let restored_path = restored_directory.path().join("restored.openroom");
        std::fs::copy(&backup, &restored_path).unwrap();
        let restored = Catalog::open(&restored_path).unwrap();
        assert_eq!(
            restored.develop_recipe(id).unwrap(),
            (Some(recipe.into()), 1)
        );
        assert_eq!(restored.detail(id).unwrap().tags, vec!["作品"]);
        assert_eq!(restored.detail(id).unwrap().item.rating, 4);
        assert_eq!(restored.detail(id).unwrap().memo, "メモ");
        restored.check().unwrap();
        assert!(restored.develop_recipe(id + 100).is_err());
        drop(restored);
        assert_eq!(
            std::fs::read_dir(restored_directory.path())
                .unwrap()
                .count(),
            1
        );

        drop(catalog);
        let legacy = Connection::open(&database).unwrap();
        legacy
            .execute_batch(
                "DROP TABLE develop_history; DROP TABLE saved_recipes; DROP TABLE photo_metadata; DROP INDEX items_flag_rating; DROP INDEX items_label; ALTER TABLE items DROP COLUMN flag; ALTER TABLE items DROP COLUMN label; PRAGMA application_id=0; PRAGMA user_version=3;",
            )
            .unwrap();
        drop(legacy);
        let migrated = Catalog::open(&database).unwrap();
        let migrated_history = migrated.develop_history(id, None).unwrap();
        assert_eq!(migrated_history.len(), 1);
        assert_eq!(migrated_history[0].label, "移行時の設定");
        assert_eq!(
            migrated.history_recipe(id, migrated_history[0].id).unwrap(),
            recipe
        );
        assert_eq!(
            migrated.develop_recipe(id).unwrap(),
            (Some(recipe.into()), 1)
        );
        assert_eq!(
            migrated
                .conn
                .query_row("PRAGMA application_id", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            CATALOG_APPLICATION_ID
        );
        assert_eq!(
            migrated
                .conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            9
        );
    }

    #[test]
    fn history_migration_backs_up_and_keeps_ten_undo_states() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        std::fs::write(photos.join("a.jpg"), b"original").unwrap();
        let path = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        catalog
            .save_develop_recipe(id, r#"{"version":1,"exposure":2}"#, 0)
            .unwrap();
        catalog
            .conn
            .execute("DELETE FROM develop_history", [])
            .unwrap();
        for index in 0..=20 {
            catalog.conn.execute("INSERT INTO develop_history(item_id,recipe,label,created) VALUES(?,?,'test',0)", params![id, format!(r#"{{"version":1,"exposure":{}}}"#, index as f32 / 10.)]).unwrap();
        }
        catalog
            .conn
            .execute_batch("DROP TABLE develop_preview_updates; PRAGMA user_version=8;")
            .unwrap();
        drop(catalog);
        let mut catalog = Catalog::open(&path).unwrap();
        let backup =
            Connection::open(dir.path().join("catalog.db.before-history-limit.openroom")).unwrap();
        assert_eq!(
            backup
                .query_row("SELECT count(*) FROM develop_history", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            21
        );
        let history = catalog.develop_history(id, None).unwrap();
        assert_eq!(history.len(), 11);
        let chain: Vec<_> = history
            .iter()
            .rev()
            .map(|row| HistoryEdit {
                recipe: catalog.history_recipe(id, row.id).unwrap(),
                label: row.label.clone(),
            })
            .collect();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&chain[0].recipe).unwrap()["exposure"],
            1.
        );
        // Undo removes the current state instead of appending a new "undo" operation.
        catalog.save_develop_undo(id, &chain[..10], 1).unwrap();
        assert_eq!(catalog.develop_history(id, None).unwrap().len(), 10);
        assert_eq!(
            catalog.develop_history(id, None).unwrap()[0].id,
            history[1].id
        );
        assert!(catalog.save_develop_undo(id, &chain[..1], 1).is_err());
        assert_eq!(catalog.develop_history(id, None).unwrap().len(), 10);
        drop(catalog);
        let reopened = Catalog::open(&path).unwrap();
        assert_eq!(
            reopened.develop_recipe(id).unwrap().0.unwrap(),
            chain[9].recipe
        );
        assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"original");
    }

    #[test]
    fn history_is_atomic_persistent_bounded_and_included_in_backup() {
        let directory = tempfile::tempdir().unwrap();
        let photos = directory.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        std::fs::write(photos.join("a.jpg"), b"original").unwrap();
        let database = directory.path().join("catalog.db");
        let mut catalog = Catalog::open(&database).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        let steps: Vec<_> = (1..=3)
            .map(|value| HistoryEdit {
                recipe: format!(r#"{{"version":1,"exposure":{value}}}"#),
                label: "露光量".into(),
            })
            .collect();
        assert_eq!(catalog.save_develop_steps(id, &steps, 0).unwrap(), 1);
        assert_eq!(catalog.develop_history(id, None).unwrap().len(), 4);
        assert!(catalog.save_develop_steps(id, &steps, 0).is_err());
        assert_eq!(
            catalog.develop_history(id, None).unwrap().len(),
            4,
            "failed writes must not append history"
        );
        let initial = catalog
            .develop_history(id, None)
            .unwrap()
            .last()
            .unwrap()
            .id;
        let recipe = catalog.history_recipe(id, initial).unwrap();
        assert_eq!(recipe, r#"{"version":1}"#);
        assert!(catalog.history_recipe(id + 1, initial).is_err());
        catalog.save_develop_recipe(id, &recipe, 1).unwrap();
        for revision in 2..207 {
            catalog
                .save_develop_recipe(id, &steps[0].recipe, revision)
                .unwrap();
        }
        let mut ids = Vec::new();
        loop {
            let page = catalog.develop_history(id, ids.last().copied()).unwrap();
            if page.is_empty() {
                break;
            }
            ids.extend(page.into_iter().map(|row| row.id));
        }
        assert_eq!(ids.len(), 11);
        assert!(ids.windows(2).all(|pair| pair[0] > pair[1]));
        let copy = directory.path().join("copy.openroom");
        catalog.backup(&copy).unwrap();
        drop(catalog);
        let reopened = Catalog::open(copy).unwrap();
        assert!(reopened.history_recipe(id, initial).is_err());
        assert_eq!(reopened.develop_history(id, None).unwrap().len(), 11);
        reopened.check().unwrap();
    }

    #[test]
    fn recipes_survive_reopen_and_reject_stale_writers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("photos");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.jpg"), b"unchanged original").unwrap();
        let db = directory.path().join("catalog.db");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&root).unwrap();
        catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
        let id = catalog.page(&Query::default(), 1, None).unwrap().items[0].id;
        assert_eq!(catalog.develop_recipe(id).unwrap(), (None, 0));
        let recipe = r#"{"version":1,"exposure":1.5}"#;
        assert_eq!(catalog.save_develop_recipe(id, recipe, 0).unwrap(), 1);
        assert!(catalog.save_develop_recipe(id, recipe, 0).is_err());
        assert_eq!(catalog.save_develop_recipe(id, recipe, 1).unwrap(), 2);
        assert!(catalog.save_develop_recipe(id, recipe, 1).is_err());
        drop(catalog);
        let catalog = Catalog::open(&db).unwrap();
        assert_eq!(
            catalog.develop_recipe(id).unwrap(),
            (Some(recipe.into()), 2)
        );
        assert_eq!(
            std::fs::read(root.join("a.jpg")).unwrap(),
            b"unchanged original"
        );
    }
}
