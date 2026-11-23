//! Small perceptual palettes derived from the rendered library thumbnail.
use crate::Catalog;
use anyhow::Result;
use rusqlite::{Connection, Transaction, TransactionBehavior, functions::FunctionFlags, params};

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let has_palette: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('library_previews') WHERE name='palette')",
        [],
        |r| r.get(0),
    )?;
    if !has_palette {
        tx.execute_batch("ALTER TABLE library_previews ADD COLUMN palette BLOB;")?;
    }
    tx.execute_batch("CREATE TABLE IF NOT EXISTS color_index_state(id INTEGER PRIMARY KEY, generation INTEGER NOT NULL);
        INSERT OR IGNORE INTO color_index_state VALUES(1,0);
        CREATE INDEX IF NOT EXISTS previews_without_colors ON library_previews(item_id) WHERE palette IS NULL AND data IS NOT NULL AND error IS NULL;
        CREATE TRIGGER IF NOT EXISTS color_index_changed AFTER UPDATE OF palette ON library_previews WHEN OLD.palette IS NOT NEW.palette BEGIN
            UPDATE color_index_state SET generation=generation+1 WHERE id=1;
        END;
        CREATE TRIGGER IF NOT EXISTS color_preview_changed AFTER UPDATE OF stamp,data ON library_previews WHEN OLD.stamp!=NEW.stamp OR NEW.data IS NULL BEGIN
            UPDATE library_previews SET palette=NULL WHERE item_id=NEW.item_id;
        END;
        CREATE TRIGGER IF NOT EXISTS color_source_changed AFTER UPDATE OF identity,size,modified ON items WHEN OLD.identity!=NEW.identity OR OLD.size!=NEW.size OR OLD.modified!=NEW.modified BEGIN
            UPDATE library_previews SET palette=NULL WHERE item_id=NEW.id;
        END;
        CREATE TRIGGER IF NOT EXISTS color_edit_added AFTER INSERT ON develop_edits BEGIN
            UPDATE library_previews SET palette=NULL WHERE item_id=NEW.item_id;
        END;
        CREATE TRIGGER IF NOT EXISTS color_edit_changed AFTER UPDATE OF revision ON develop_edits WHEN OLD.revision!=NEW.revision BEGIN
            UPDATE library_previews SET palette=NULL WHERE item_id=NEW.item_id;
        END;
        CREATE TRIGGER IF NOT EXISTS color_edit_removed AFTER DELETE ON develop_edits BEGIN
            UPDATE library_previews SET palette=NULL WHERE item_id=OLD.item_id;
        END;
        PRAGMA user_version=10;")?;
    tx.commit()?;
    Ok(())
}

fn oklab(rgb: [u8; 3]) -> [u8; 3] {
    let [r, g, b] = rgb.map(|v| {
        let x = f64::from(v) / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    });
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        (0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s) * 255.,
        (1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s) * 255. + 128.,
        (0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s) * 255. + 128.,
    ]
    .map(|v| v.round() as u8)
}

fn distance(a: &[u8], b: &[u8]) -> i32 {
    (0..3)
        .map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2))
        .sum()
}

/// At most five colors, four bytes per entry (Oklab and relative area).
pub fn extract(samples: impl Iterator<Item = [u8; 3]>) -> Vec<u8> {
    let mut bins = [[0u32; 4]; 512];
    let mut total = 0;
    for rgb in samples {
        let [r, g, b] = rgb.map(|v| usize::from(v >> 5));
        let bin = &mut bins[(r * 8 + g) * 8 + b];
        bin[0] += 1;
        for i in 0..3 {
            bin[i + 1] += u32::from(rgb[i]);
        }
        total += 1;
    }
    bins.sort_by_key(|bin| std::cmp::Reverse(bin[0]));
    let mut groups: Vec<([u8; 3], u32)> = Vec::new();
    for [n, r, g, b] in bins.into_iter().take_while(|bin| bin[0] > 0) {
        let color = oklab([r, g, b].map(|v| ((v + n / 2) / n) as u8));
        if let Some((_, weight)) = groups
            .iter_mut()
            .filter(|(c, _)| distance(c, &color) <= 22 * 22)
            .min_by_key(|(c, _)| distance(c, &color))
        {
            *weight += n;
        } else {
            groups.push((color, n));
        }
    }
    groups.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let mut palette = Vec::new();
    for (color, n) in groups.into_iter().take(5) {
        if !palette.is_empty() && n * 20 < total {
            break;
        }
        palette.extend_from_slice(&color);
        palette.push(((n * 255 + total / 2) / total) as u8);
    }
    palette
}

pub(crate) fn score_sql(rgb: [u8; 3]) -> String {
    let [l, a, b] = oklab(rgb);
    let target = u32::from_be_bytes([0, l, a, b]);
    format!(
        "(SELECT photo_color_score(palette,{target}) FROM library_previews WHERE item_id=items.id)"
    )
}

pub(crate) fn register(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "photo_color_score",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let target = ctx.get::<u32>(1)?.to_be_bytes();
            let result = ctx.get_raw(0).as_blob().ok().and_then(|palette| {
                palette
                    .chunks_exact(4)
                    .take(5)
                    .filter_map(|entry| {
                        let d = distance(entry, &target[1..]);
                        (d <= 40 * 40).then_some(d * 256 + 255 - i32::from(entry[3]))
                    })
                    .min()
            });
            Ok(result)
        },
    )?;
    Ok(())
}

impl Catalog {
    pub fn unindexed_colors(&self, after: i64) -> Result<Vec<(i64, String, String)>> {
        Ok(self.conn.prepare("SELECT item_id,stamp,data FROM library_previews WHERE item_id>? AND palette IS NULL AND data IS NOT NULL AND error IS NULL ORDER BY item_id LIMIT 32")?.query_map([after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn backfill_colors(
        &self,
        id: i64,
        stamp: &str,
        data: &str,
        palette: &[u8],
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if self.preview_stamp(id)? != stamp {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE library_previews SET palette=? WHERE item_id=? AND stamp=? AND data=? AND palette IS NULL",params![palette,id,stamp,data])?;
        tx.commit()?;
        Ok(changed != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;
    #[test]
    fn palettes_rank_major_colors_and_ignore_accents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let catalog = Catalog::open(&path).unwrap();
        let root = catalog.add_root(dir.path()).unwrap();
        let red = [200, 45, 40];
        let blue = [30, 65, 230];
        for (i, (base, accent, n)) in [
            (red, blue, 1),
            (red, blue, 40),
            ([213, 48, 43], red, 0),
            (blue, red, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let id = i as i64 + 1;
            catalog.conn.execute("INSERT INTO items(id,root_id,path,name,name_norm,extension,size,modified,added,identity) VALUES(?,?,?,'sample','sample','jpg',1,1,1,'id')",params![id,root.id,id.to_string()]).unwrap();
            let palette =
                extract(std::iter::repeat_n(base, 100 - n).chain(std::iter::repeat_n(accent, n)));
            assert!(palette.len() <= 20);
            catalog
                .save_library_preview_with_colors(
                    id,
                    &catalog.preview_stamp(id).unwrap(),
                    Some("{}"),
                    None,
                    Some(&palette),
                )
                .unwrap();
        }
        drop(catalog);
        let mut catalog = Catalog::open(&path).unwrap();
        let q = Query {
            photo_color: Some(red),
            descending: true,
            ..Default::default()
        };
        assert_eq!(catalog.count(&q).unwrap(), 3);
        let first = catalog.page(&q, 1, None).unwrap();
        assert_eq!(first.items[0].id, 1);
        assert_eq!(
            catalog.page(&q, 1, first.next.as_ref()).unwrap().items[0].id,
            2
        );
        assert_eq!(
            catalog
                .count(&Query {
                    photo_color: Some(blue),
                    ..Default::default()
                })
                .unwrap(),
            2
        );
        assert_eq!(
            catalog
                .count(&Query {
                    photo_color: Some([255, 255, 255]),
                    ..Default::default()
                })
                .unwrap(),
            0
        );
        assert_eq!(
            catalog
                .count(&Query {
                    root_id: Some(root.id + 1),
                    ..q.clone()
                })
                .unwrap(),
            0
        );
        let stamp = catalog.preview_stamp(1).unwrap();
        catalog.save_develop_recipe(1, r#"{"version":1,"exposure":1}"#, 0).unwrap();
        assert_eq!(catalog.count(&q).unwrap(), 2);
        assert!(catalog.page(&q, 1, first.next.as_ref()).is_err());
        assert!(
            !catalog
                .backfill_colors(1, &stamp, "{}", &extract([red].into_iter()))
                .unwrap()
        );
    }
}
