use serde_json::{Value, json};
use std::process::Command;
fn run(db: &std::path::Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_advanced-explorer"))
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn cli_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("日本.png"), b"source bytes").unwrap();
    std::fs::write(assets.join("photo.tiff"), b"photo bytes").unwrap();
    let db = temp.path().join("catalog.db");
    let root = run(&db, &["add-root", assets.to_str().unwrap()]);
    assert_eq!(run(&db, &["scan"])["indexed"], 2);
    let first = run(&db, &["search", "{}", "--limit", "1"]);
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    let cursor = first["next"].to_string();
    let second = run(&db, &["search", "{}", "--limit", "1", "--cursor", &cursor]);
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
    let id = first["items"][0]["id"].to_string();
    run(
        &db,
        &[
            "annotate", "--ids", &id, "--tags", "参考", "--rating", "4", "--memo", "keep",
        ],
    );
    assert_eq!(run(&db, &["detail", &id])["memo"], "keep");
    assert_eq!(
        run(&db, &["count", &json!({"tags":["参考"]}).to_string()])["count"],
        1
    );
    assert_eq!(run(&db, &["check"])["integrity"], "ok");
    let backup = temp.path().join("backup.db");
    run(&db, &["backup", backup.to_str().unwrap()]);
    assert_eq!(run(&backup, &["count"])["count"], 2);
    let failure = Command::new(env!("CARGO_BIN_EXE_advanced-explorer"))
        .arg("--db")
        .arg(&db)
        .args(["search", "{}", "--cursor", &cursor])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    run(&db, &["remove-root", &root["id"].to_string()]);
    assert_eq!(run(&db, &["count"])["count"], 0);
    assert_eq!(
        std::fs::read(assets.join("日本.png")).unwrap(),
        b"source bytes"
    );
}
