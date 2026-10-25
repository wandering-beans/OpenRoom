use advanced_explorer::{Annotation, Catalog, Cursor, Query};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Parser)]
#[command(
    version,
    about = "Local asset index and search core; never modifies source files"
)]
struct Cli {
    #[arg(long, default_value = "catalog.db")]
    db: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    AddRoot {
        path: PathBuf,
    },
    Roots,
    RemoveRoot {
        id: i64,
    },
    Scan,
    Search {
        #[arg(default_value = "{}")]
        query: String,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    Count {
        #[arg(default_value = "{}")]
        query: String,
    },
    Detail {
        id: i64,
    },
    Annotate {
        #[arg(long,required=true,num_args=1..)]
        ids: Vec<i64>,
        #[arg(long)]
        tags: Vec<String>,
        #[arg(long, default_value_t = 0)]
        rating: u8,
        #[arg(long, default_value = "")]
        memo: String,
    },
    Check,
    Backup {
        path: PathBuf,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut c = Catalog::open(cli.db)?;
    let value = match cli.command {
        Command::AddRoot { path } => serde_json::to_value(c.add_root(path)?)?,
        Command::Roots => serde_json::to_value(c.roots()?)?,
        Command::RemoveRoot { id } => {
            c.remove_root(id)?;
            serde_json::json!({"removed":id})
        }
        Command::Scan => {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed))?;
            serde_json::to_value(c.scan(&cancel, |p| {
                eprintln!(
                    "visited={} indexed={} errors={}",
                    p.visited,
                    p.indexed,
                    p.errors.len()
                )
            })?)?
        }
        Command::Search {
            query,
            limit,
            cursor,
        } => {
            let q: Query = serde_json::from_str(&query)?;
            let cursor: Option<Cursor> = cursor.map(|s| serde_json::from_str(&s)).transpose()?;
            serde_json::to_value(c.page(&q, limit, cursor.as_ref())?)?
        }
        Command::Count { query } => {
            serde_json::json!({"count":c.count(&serde_json::from_str(&query)?)?})
        }
        Command::Detail { id } => serde_json::to_value(c.detail(id)?)?,
        Command::Annotate {
            ids,
            tags,
            rating,
            memo,
        } => {
            c.annotate(&ids, &Annotation { tags, rating, memo })?;
            serde_json::json!({"updated":ids.len()})
        }
        Command::Check => {
            c.check()?;
            serde_json::json!({"integrity":"ok"})
        }
        Command::Backup { path } => {
            c.backup(path)?;
            serde_json::json!({"backup":"ok","includes_originals":false})
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
