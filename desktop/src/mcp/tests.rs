use super::*;
use advanced_explorer::{Annotation, AnnotationPatch, Catalog, HistoryEdit, Query};
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture() -> (tempfile::TempDir, Operations, Vec<i64>) {
    let directory = tempfile::tempdir().unwrap();
    let photos = directory.path().join("photos");
    let cache = directory.path().join("cache");
    std::fs::create_dir(&photos).unwrap();
    std::fs::create_dir(&cache).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbImage::from_pixel(32, 24, image::Rgb([50, 100, 150]))
            .save(photos.join(name))
            .unwrap();
    }
    let db = directory.path().join("catalog.db");
    let mut catalog = Catalog::open(&db).unwrap();
    catalog.add_root(&photos).unwrap();
    catalog.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let ids = catalog
        .page(&Query::default(), 200, None)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    let state = AppState {
        library: Arc::new(crate::library_previews::Store::new(&db).unwrap()),
        db,
        cache: cache.clone(),
        renderer: crate::media::Renderer::new(&cache).unwrap(),
        scanning: Arc::new(AtomicBool::new(false)),
        cancel: Arc::new(AtomicBool::new(false)),
        thumbs: Arc::new(tokio::sync::Semaphore::new(1)),
        previews: Arc::new(tokio::sync::Semaphore::new(1)),
        preview_version: Arc::new(AtomicUsize::new(0)),
        metadata_jobs: Arc::new(tokio::sync::Semaphore::new(1)),
        metadata_version: Arc::new(AtomicUsize::new(0)),
    };
    let operations = Operations {
        state: Arc::new(state),
        jobs: Arc::new(Jobs::default()),
        app: None,
        stop: CancellationToken::new(),
        reads: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    (directory, operations, ids)
}
#[test]
fn authentication_rejects_foreign_origins_hosts_and_old_tokens() {
    let auth = Authentication {
        token: Arc::new("test-secret-abcdefghijklmnopqrstuvwxyz".into()),
        port: 39127,
        stop: CancellationToken::new(),
    };
    let mut headers = HeaderMap::new();
    headers.insert("host", "127.0.0.1:39127".parse().unwrap());
    assert_eq!(authorized(&headers, &auth), Err(StatusCode::UNAUTHORIZED));
    headers.insert(
        "authorization",
        format!("Bearer {}", auth.token).parse().unwrap(),
    );
    assert!(authorized(&headers, &auth).is_ok());
    headers.insert("origin", "https://untrusted.example".parse().unwrap());
    assert_eq!(authorized(&headers, &auth), Err(StatusCode::FORBIDDEN));
    headers.insert("origin", "http://127.0.0.1:39127".parse().unwrap());
    assert!(authorized(&headers, &auth).is_ok());
    headers.insert("host", "attacker.example:39127".parse().unwrap());
    assert_eq!(authorized(&headers, &auth), Err(StatusCode::FORBIDDEN));
    headers.insert("host", "127.0.0.1:39127".parse().unwrap());
    let rotated = Authentication {
        token: Arc::new("replacement-secret-abcdefghijklmnopqrstuvwxyz".into()),
        ..auth.clone()
    };
    assert_eq!(
        authorized(&headers, &rotated),
        Err(StatusCode::UNAUTHORIZED)
    );
    auth.stop.cancel();
    assert_eq!(
        authorized(&headers, &auth),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );
}
#[test]
fn partial_annotations_and_id_filter_preserve_other_values_and_rollback() {
    let (_directory, operations, ids) = fixture();
    let mut catalog = Catalog::open(&operations.state.db).unwrap();
    catalog
        .annotate(
            &ids,
            &Annotation {
                rating: 4,
                tags: vec!["旅行".into()],
                memo: "keep".into(),
            },
        )
        .unwrap();
    catalog
        .patch_annotations(
            &[ids[0]],
            &AnnotationPatch {
                memo: Some("changed".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let detail = catalog.detail(ids[0]).unwrap();
    assert_eq!(detail.item.rating, 4);
    assert_eq!(detail.tags, vec!["旅行"]);
    assert_eq!(detail.memo, "changed");
    assert!(
        catalog
            .patch_annotations(
                &[ids[0], 999999],
                &AnnotationPatch {
                    memo: Some("wrong".into()),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(catalog.detail(ids[0]).unwrap().memo, "changed");
    let query = Query {
        ids: vec![ids[1]],
        ..Default::default()
    };
    assert_eq!(catalog.count(&query).unwrap(), 1);
    assert_eq!(catalog.page(&query, 200, None).unwrap().items[0].id, ids[1]);
    catalog
        .patch_annotations(
            &[ids[0]],
            &AnnotationPatch {
                tags: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(catalog.detail(ids[0]).unwrap().tags.is_empty());
}
#[tokio::test]
async fn edits_reject_invalid_values_and_stale_revisions_and_restore_history() {
    let (_directory, operations, ids) = fixture();
    let id = ids[0];
    let edit = |revision, patch| json!({"id":id,"expected_revision":revision,"patch":patch});
    assert!(
        operations
            .execute(
                "update_develop_settings".into(),
                edit(0, json!({"exposure":7}))
            )
            .await
            .is_err()
    );
    assert!(
        operations
            .execute(
                "update_develop_settings".into(),
                edit(0, json!({"expousure":1}))
            )
            .await
            .is_err()
    );
    let result = operations
        .execute(
            "update_develop_settings".into(),
            edit(0, json!({"exposure":1.25})),
        )
        .await
        .unwrap();
    assert_eq!(result["revision"], 1);
    assert!(
        operations
            .execute(
                "update_develop_settings".into(),
                edit(0, json!({"contrast":20}))
            )
            .await
            .is_err()
    );
    let history = Catalog::open(&operations.state.db)
        .unwrap()
        .develop_history(id, None)
        .unwrap();
    let initial = history.last().unwrap().id;
    operations
        .execute(
            "restore_develop_history".into(),
            json!({"id":id,"history_id":initial,"expected_revision":1}),
        )
        .await
        .unwrap();
    let result = operations
        .execute("get_develop_settings".into(), json!({"id":id}))
        .await
        .unwrap();
    assert_eq!(result["settings"]["exposure"], 0.0);
    assert_eq!(result["revision"], 2);
    let mut catalog = Catalog::open(&operations.state.db).unwrap();
    let edits = vec![
        (
            ids[0],
            HistoryEdit {
                recipe: "{\"version\":1,\"contrast\":20}".into(),
                label: "batch".into(),
            },
            2,
        ),
        (
            ids[1],
            HistoryEdit {
                recipe: "{\"version\":1,\"contrast\":20}".into(),
                label: "batch".into(),
            },
            99,
        ),
    ];
    assert!(catalog.save_develop_batch(&edits).is_err());
    assert_eq!(catalog.develop_recipe(ids[0]).unwrap().1, 2);
}
async fn finished(jobs: &Jobs, id: &str) -> jobs::Job {
    for _ in 0..200 {
        let job = jobs.get(id).unwrap();
        if job.status != "running" {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("Job did not finish");
}
#[tokio::test]
async fn export_freezes_settings_preserves_originals_and_refuses_existing_output() {
    let (directory, operations, ids) = fixture();
    let source = directory.path().join("photos/a.png");
    let original = std::fs::read(&source).unwrap();
    let destination = directory.path().join("exports");
    std::fs::create_dir(&destination).unwrap();
    let input = json!({"ids":ids,"destination":destination,"format":"jpeg","options":{"quality":85,"long_edge":16}});
    let started = operations
        .execute("start_export".into(), input.clone())
        .await
        .unwrap();
    let job = finished(&operations.jobs, started["id"].as_str().unwrap()).await;
    assert_eq!(job.status, "completed");
    let result = job.result.unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 2);
    assert!(result["errors"].as_array().unwrap().is_empty());
    let output = destination.join(format!("a-{}-edited.jpg", ids[0]));
    let before = std::fs::read(&output).unwrap();
    assert_eq!(image::open(&output).unwrap().width(), 16);
    let again = operations
        .execute("start_export".into(), input)
        .await
        .unwrap();
    let again = finished(&operations.jobs, again["id"].as_str().unwrap()).await;
    assert_eq!(again.result.unwrap()["errors"].as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert_eq!(std::fs::read(output).unwrap(), before);
}
#[tokio::test]
async fn scan_prepares_library_images_and_comparison_loads_both_cold_photos() {
    let (_directory, operations, ids) = fixture();
    operations.state.library.set_avif_supported(false);
    let (first, second) = tokio::join!(
        crate::preview_inner(&operations.state, ids[0], true, Some(1600)),
        crate::preview_inner(&operations.state, ids[1], true, Some(1600)),
    );
    assert_ne!(first.unwrap().path, second.unwrap().path);
    let catalog = Catalog::open(&operations.state.db).unwrap();
    for id in &ids {
        operations.state.library.invalidate(&catalog, *id, 1600).unwrap();
        operations.state.library.invalidate(&catalog, *id, 384).unwrap();
    }
    let job = operations.execute("start_scan".into(), json!({})).await.unwrap();
    let done = finished(&operations.jobs, job["id"].as_str().unwrap()).await;
    assert_eq!(done.status, "completed");
    assert_eq!(done.result.unwrap()["previews"]["generated"], 2);
    for id in ids {
        std::fs::remove_file(catalog.source_path(id).unwrap()).unwrap();
        for bound in [384, 1600] {
            assert!(operations.state.library.lookup(&catalog, id, bound).unwrap().is_some());
        }
    }
}
#[tokio::test]
async fn cancellation_and_duplicate_jobs_are_reported() {
    let jobs = Arc::new(Jobs::default());
    let first = jobs
        .start("test", None, |cancel, _| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(json!({"stopped":true}))
        })
        .unwrap();
    assert!(jobs.start("test", None, |_, _| Ok(Value::Null)).is_err());
    jobs.cancel(&first.id).unwrap();
    let done = finished(&jobs, &first.id).await;
    assert_eq!(done.status, "cancelled");
    assert_eq!(done.result.unwrap()["stopped"], true);
}
#[tokio::test]
async fn disconnected_and_stopped_ui_requests_cannot_execute_later() {
    let bridge = Arc::new(Bridge::default());
    let caller = bridge.clone();
    let stop = CancellationToken::new();
    let request_stop = stop.clone();
    let task = tokio::spawn(async move {
        caller
            .request(
                None,
                "select_photos".into(),
                json!({"ids":[1]}),
                request_stop,
            )
            .await
    });
    tokio::task::yield_now().await;
    let id = bridge.queued()[0].id.clone();
    bridge.claim(&id).unwrap();
    stop.cancel();
    assert!(bridge.begin(&id).is_err());
    bridge.stop();
    assert!(task.await.unwrap().is_err());
    assert!(bridge.claim(&id).is_err());
    let caller = bridge.clone();
    let task = tokio::spawn(async move {
        caller
            .request(None, "set_view".into(), json!({}), CancellationToken::new())
            .await
    });
    tokio::task::yield_now().await;
    let id = bridge.queued()[0].id.clone();
    task.abort();
    let _ = task.await;
    assert!(bridge.claim(&id).is_err());
}
#[tokio::test]
async fn http_roundtrip_lists_tools_searches_previews_edits_and_exports() {
    let (directory, operations, ids) = fixture();
    let bridge = Arc::new(Bridge::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handler = Handler {
        operations: operations.clone(),
        bridge: bridge.clone(),
    };
    let authentication = "test-only-http-credential";
    let router = router(handler, authentication.into(), port);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    // Simulate the UI acknowledgement only here; browser tests exercise the real JS gate.
    let ui_operations = operations.clone();
    let ui_bridge = bridge.clone();
    let ui = tokio::spawn(async move {
        loop {
            for request in ui_bridge.queued() {
                ui_bridge.claim(&request.id).unwrap();
                ui_bridge.begin(&request.id).unwrap();
                let result = ui_operations.execute(request.name, request.arguments).await;
                match result {
                    Ok(value) => ui_bridge.complete(&request.id, value, None).unwrap(),
                    Err(error) => ui_bridge
                        .complete(&request.id, Value::Null, Some(error.to_string()))
                        .unwrap(),
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/mcp");
    assert_eq!(client.post(&url).send().await.unwrap().status(), 401);
    let rpc = |method: &str, params: Value| {
        client
            .post(&url)
            .bearer_auth(authentication)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
    };
    let initialize:Value=rpc("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"integration-test","version":"1"}})).send().await.unwrap().json().await.unwrap();
    assert!(initialize.get("result").is_some(), "{initialize}");
    let listed: Value = rpc("tools/list", json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 26);
    let search: Value = rpc(
        "tools/call",
        json!({"name":"search_photos","arguments":{"query":{},"limit":1}}),
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        search["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{search}"
    );
    let preview:Value=rpc("tools/call",json!({"name":"preview_photo","arguments":{"id":ids[0],"bound":384,"patch":{"exposure":0.5}}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        preview["result"]["content"][1]["type"], "image",
        "{preview}"
    );
    let edited:Value=rpc("tools/call",json!({"name":"update_develop_settings","arguments":{"id":ids[0],"expected_revision":0,"patch":{"exposure":0.5}}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        edited["result"]["structuredContent"]["revision"], 1,
        "{edited}"
    );
    let exported:Value=rpc("tools/call",json!({"name":"start_export","arguments":{"ids":[ids[0]],"destination":directory.path(),"format":"png"}})).send().await.unwrap().json().await.unwrap();
    let job_id = exported["result"]["structuredContent"]["id"]
        .as_str()
        .unwrap();
    let job = finished(&operations.jobs, job_id).await;
    assert_eq!(job.status, "completed");
    assert_eq!(job.result.unwrap()["files"].as_array().unwrap().len(), 1);
    operations.stop.cancel();
    assert_eq!(
        rpc("tools/list", json!({})).send().await.unwrap().status(),
        503
    );
    server.abort();
    ui.abort();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let renewed = "replacement-test-only-http-credential";
    let mut operations = operations;
    operations.stop = CancellationToken::new();
    let router = super::router(Handler { operations, bridge }, renewed.into(), port);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://127.0.0.1:{port}/mcp");
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(authentication)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let listed: Value = client
        .post(&url)
        .bearer_auth(renewed)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 26);
    server.abort();
}
#[test]
fn settings_persist_without_credentials() {
    let directory = tempfile::tempdir().unwrap();
    assert!(!read_config(directory.path()).unwrap().enabled);
    save_config(
        directory.path(),
        &Config {
            enabled: true,
            port: 39128,
        },
    )
    .unwrap();
    let loaded = read_config(directory.path()).unwrap();
    assert!(loaded.enabled);
    assert_eq!(loaded.port, 39128);
    let value: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("mcp-settings.json")).unwrap())
            .unwrap();
    assert_eq!(value.as_object().unwrap().len(), 2);
}
#[test]
#[ignore = "requires access to the OS credential store"]
fn native_credential_persists_and_rotates() {
    let directory = tempfile::tempdir().unwrap();
    let original = token(directory.path(), false).unwrap();
    assert_eq!(token(directory.path(), false).unwrap(), original);
    let rotated = token(directory.path(), true).unwrap();
    assert_ne!(original, rotated);
    assert_eq!(token(directory.path(), false).unwrap(), rotated);
    credential(directory.path())
        .unwrap()
        .delete_credential()
        .unwrap();
}
