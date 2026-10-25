//! Catalog indices for Lightroom-style attribute and metadata filtering.
use crate::{Catalog, Query, filter};
use anyhow::{Result, ensure};
use rusqlite::{params, params_from_iter};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Facet {
    pub value: String,
    pub count: i64,
}
#[derive(Debug, Serialize)]
pub struct MetadataCandidate {
    pub id: i64,
    pub size: i64,
    pub modified: i64,
}
impl Catalog {
    pub fn set_attributes(
        &mut self,
        ids: &[i64],
        rating: Option<u8>,
        flag: Option<i8>,
        label: Option<&str>,
    ) -> Result<()> {
        ensure!(
            !ids.is_empty() && ids.len() <= 10000,
            "Select 1–10000 photos"
        );
        ensure!(
            rating.is_none_or(|v| v <= 5) && flag.is_none_or(|v| (-1..=1).contains(&v)),
            "Invalid attribute"
        );
        ensure!(
            label.is_none_or(|v| ["", "red", "yellow", "green", "blue", "purple"].contains(&v)),
            "Invalid label"
        );
        let tx = self.conn.transaction()?;
        for id in ids {
            ensure!(tx.execute("UPDATE items SET rating=coalesce(?,rating),flag=coalesce(?,flag),label=coalesce(?,label) WHERE id=?", params![rating,flag,label,id])? == 1,"Photo not found");
        }
        tx.execute("UPDATE state SET generation=generation+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn metadata_candidates(
        &self,
        query: &Query,
        limit: usize,
    ) -> Result<Vec<MetadataCandidate>> {
        ensure!((1..=32).contains(&limit), "Invalid metadata batch size");
        // Metadata filters cannot match photos whose metadata has not been read yet.
        // Index the selected source, including photos hidden by the current filters.
        let scope = Query {
            root_id: query.root_id,
            folder: query.folder.clone(),
            ..Default::default()
        };
        let (condition, args) = filter(&scope)?;
        Ok(self.conn.prepare(&format!("SELECT id,size,modified FROM items WHERE {condition} AND NOT EXISTS(SELECT 1 FROM photo_metadata m WHERE m.item_id=items.id AND m.source_size=items.size AND m.source_modified=items.modified) ORDER BY id LIMIT {limit}"))?
            .query_map(params_from_iter(args), |r| Ok(MetadataCandidate{id:r.get(0)?,size:r.get(1)?,modified:r.get(2)?}))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn index_metadata(
        &self,
        item: &MetadataCandidate,
        fields: &BTreeMap<String, String>,
        error: Option<&str>,
    ) -> Result<()> {
        let field = |key: &str| fields.get(key).cloned().unwrap_or_default();
        let date = field("撮影日時")
            .chars()
            .take(10)
            .collect::<String>()
            .replace(':', "-");
        self.conn.execute("INSERT INTO photo_metadata(item_id,source_size,source_modified,date,camera,lens,iso,aperture,focal,fields,error) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(item_id) DO UPDATE SET source_size=excluded.source_size,source_modified=excluded.source_modified,date=excluded.date,camera=excluded.camera,lens=excluded.lens,iso=excluded.iso,aperture=excluded.aperture,focal=excluded.focal,fields=excluded.fields,error=excluded.error",
            params![item.id,item.size,item.modified,date,field("カメラ"),field("レンズ"),field("ISO"),field("絞り"),field("焦点距離"),serde_json::to_string(fields)?,error])?;
        Ok(())
    }
    pub fn metadata_facets(&self, query: &Query, field: &str) -> Result<Vec<Facet>> {
        ensure!(
            [
                "date",
                "camera",
                "lens",
                "iso",
                "aperture",
                "focal",
                "label",
                "extension"
            ]
            .contains(&field),
            "Invalid metadata field"
        );
        let mut query = query.clone();
        query.metadata.remove(field);
        if field == "label" {
            query.labels.clear();
        }
        if field == "extension" {
            query.extensions.clear();
        }
        let (condition, args) = filter(&query)?;
        let expression = if field == "label" || field == "extension" {
            field.to_string()
        } else {
            format!(
                "coalesce((SELECT {field} FROM photo_metadata m WHERE m.item_id=items.id AND m.source_size=items.size AND m.source_modified=items.modified),'')"
            )
        };
        Ok(self.conn.prepare(&format!("SELECT {expression} AS value,count(*) FROM items WHERE {condition} GROUP BY value ORDER BY value"))?
            .query_map(params_from_iter(args),|r|Ok(Facet{value:r.get(0)?,count:r.get(1)?}))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_index_scopes_sources_without_excluding_unindexed_photos() {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path().join("catalog.db")).unwrap();
        let mut roots = Vec::new();
        for root in ["first", "second"] {
            let path = dir.path().join(root);
            for folder in ["selected", "selected/child", "selected-other"] {
                let folder = path.join(folder);
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(folder.join("photo.jpg"), b"fixture").unwrap();
            }
            roots.push(catalog.add_root(path).unwrap().id);
        }
        catalog
            .scan(&std::sync::atomic::AtomicBool::new(false), |_| {})
            .unwrap();
        let query = Query {
            root_id: Some(roots[1]),
            folder: Some("selected".into()),
            rating: Some(5),
            metadata: BTreeMap::from([("camera".into(), vec!["Canon".into()])]),
            ..Default::default()
        };
        assert_eq!(catalog.count(&query).unwrap(), 0);
        let candidates = catalog.metadata_candidates(&query, 32).unwrap();
        assert_eq!(candidates.len(), 2);
        for item in candidates {
            assert_eq!(catalog.detail(item.id).unwrap().item.root_id, roots[1]);
            catalog
                .index_metadata(&item, &BTreeMap::new(), None)
                .unwrap();
        }
        assert!(catalog.metadata_candidates(&query, 1).unwrap().is_empty());
        assert_eq!(
            catalog
                .metadata_candidates(&Query::default(), 32)
                .unwrap()
                .len(),
            4
        );
        assert!(
            catalog
                .metadata_candidates(
                    &Query {
                        folder: Some("../outside".into()),
                        ..query
                    },
                    16
                )
                .is_err()
        );
    }

    #[test]
    fn attribute_and_metadata_filters_survive_reopen_and_preserve_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            std::fs::write(photos.join(name), b"fixture").unwrap();
        }
        let db = dir.path().join("catalog.db");
        let mut catalog = Catalog::open(&db).unwrap();
        catalog.add_root(&photos).unwrap();
        catalog
            .scan(&std::sync::atomic::AtomicBool::new(false), |_| {})
            .unwrap();
        let ids: Vec<_> = catalog
            .page(&Query::default(), 100, None)
            .unwrap()
            .items
            .into_iter()
            .map(|p| p.id)
            .collect();
        catalog
            .set_attributes(&[ids[0]], Some(5), Some(1), Some("red"))
            .unwrap();
        catalog
            .set_attributes(&[ids[1]], Some(3), Some(-1), None)
            .unwrap();
        assert!(
            catalog
                .set_attributes(&[ids[0], 999999], Some(0), None, None)
                .is_err()
        );
        assert_eq!(catalog.detail(ids[0]).unwrap().item.rating, 5);
        for item in catalog.metadata_candidates(&Query::default(), 32).unwrap() {
            catalog
                .index_metadata(
                    &item,
                    &BTreeMap::from([
                        (
                            "カメラ".into(),
                            if item.id == ids[0] {
                                "Canon".into()
                            } else {
                                "Sony".into()
                            },
                        ),
                        ("撮影日時".into(), "2026:10:20 12:00:00".into()),
                        ("レンズ".into(), "18–150 mm".into()),
                    ]),
                    None,
                )
                .unwrap();
        }
        drop(catalog);
        let catalog = Catalog::open(db).unwrap();
        let q = Query {
            rating: Some(3),
            rating_mode: "gte".into(),
            ..Default::default()
        };
        assert_eq!(catalog.count(&q).unwrap(), 2);
        for (mode, expected) in [("eq", 1), ("lte", 2)] {
            assert_eq!(
                catalog
                    .count(&Query {
                        rating_mode: mode.into(),
                        ..q.clone()
                    })
                    .unwrap(),
                expected
            );
        }
        assert_eq!(
            catalog
                .count(&Query {
                    flags: vec![-1, 1],
                    ..Default::default()
                })
                .unwrap(),
            2
        );
        assert_eq!(
            catalog
                .count(&Query {
                    flags: vec![-1],
                    ..Default::default()
                })
                .unwrap(),
            1
        );
        let q = Query {
            metadata: BTreeMap::from([
                ("camera".into(), vec!["Canon".into()]),
                ("date".into(), vec!["2026".into()]),
            ]),
            labels: vec!["red".into()],
            ..Default::default()
        };
        assert_eq!(catalog.count(&q).unwrap(), 1);
        assert_eq!(
            catalog
                .metadata_facets(&Query::default(), "camera")
                .unwrap()
                .len(),
            2
        );
        assert!(
            catalog
                .metadata_candidates(&Query::default(), 32)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            catalog
                .count(&Query {
                    metadata: BTreeMap::from([("date".into(), vec!["".into()])]),
                    ..Default::default()
                })
                .unwrap(),
            0,
            "An unknown date must not match every indexed date"
        );
        assert!(
            catalog
                .metadata_facets(&q, "camera; DROP TABLE items")
                .is_err()
        );
        catalog
            .conn
            .execute("UPDATE items SET modified=modified+1 WHERE id=?", [ids[0]])
            .unwrap();
        assert_eq!(
            catalog.count(&q).unwrap(),
            0,
            "A changed source invalidates its old metadata"
        );
        let pending = catalog.metadata_candidates(&Query::default(), 32).unwrap();
        assert_eq!(pending.len(), 1);
        catalog
            .index_metadata(
                &pending[0],
                &BTreeMap::new(),
                Some("unreadable test source"),
            )
            .unwrap();
        assert_eq!(
            catalog
                .count(&Query {
                    metadata: BTreeMap::from([("date".into(), vec!["".into()])]),
                    ..Default::default()
                })
                .unwrap(),
            1
        );
    }
}
