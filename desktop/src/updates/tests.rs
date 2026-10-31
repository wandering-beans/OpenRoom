use super::*;
use base64::Engine;
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex, atomic::AtomicBool};

// A disposable test key signs only this tiny synthetic app; it is unrelated to release signing.
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU0OTk2ODU2MDA0MTcxQ0MKUldUTWNVRUFWbWlaNU0yMjY2Y3dvRGZiSnV5RVdaa3hmd1Fpdk51YnZiSlM1TDN4UkM4WGJaNTQK";
const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUTWNVRUFWbWlaNUdTUlNJMUJtNmhuNEhXRWJxTzVVbjFNSS9vdVIxaEFvY3pDcXpjK2ozcnJpY1Myd01RK0pJZUNIQjJ4ZUMyZVE0eHdCNzJxQjdZTmRHNW9sTFNsK0EwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzNDE5MzgzCWZpbGU6dXBkYXRlLnRhci5negl2ZXJzaW9uOjIuMC4wCmlabGdIeCtoaG5BdkdYTHc1VVlLSktwV1A1T2ZrREtFdjNJN1FSWTZaWmhNcE5xbjRLelcwRjVmYWdwTExNdXRtYk9wQUpaN3g0WE4zMm9maW9pc0FRPT0K";
const ARCHIVE: &str = "H4sICHdo5WoC/3VwZGF0ZS50YXIA7c1BCgIhGMVxj+IJ0mJkDtA6hDqBNA64SMXR6PhjEQRtgwj6/zbv473FZ7OPx5QuG5ez2qdYfayLOrizPSn73MSHdDca88juPbXeja/73m8HowchtfiCtlRX+nvxn66+hDn4SbY8uerlHG61FS8AAAAAAAAAAAAAAAAAAL9tBa7W5jEAKAAA";

#[test]
fn macos_versions_compare_numerically_and_reject_invalid_metadata() {
    assert_eq!(system_version("27"), Some([27, 0, 0]));
    assert_eq!(system_version("27.1.2"), Some([27, 1, 2]));
    assert!(system_version("27.10") > system_version("27.9"));
    for value in ["", "27.x", "27.1.2.3", "27.", "-1"] {
        assert_eq!(system_version(value), None, "{value}");
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn signed_updates_reject_tampering_and_replace_only_the_temporary_app() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("OpenRoom.app/Contents/MacOS/OpenRoom");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"original app").unwrap();
    let catalog = temp.path().join("catalog.openroom");
    std::fs::write(&catalog, b"preserved catalog").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let manifest = Arc::new(StdMutex::new(json!({
        "version":"2.0.0", "minimum_macos":"0.0",
        "platforms":{"darwin-aarch64":{"url":format!("{base}/artifact"),"signature":SIGNATURE}}
    })));
    let corrupt = Arc::new(AtomicBool::new(false));
    let manifest_route = manifest.clone();
    let corrupt_route = corrupt.clone();
    let routes = axum::Router::new()
        .route(
            "/manifest",
            axum::routing::get(move || {
                let body = manifest_route.lock().unwrap().clone();
                async move { axum::Json(body) }
            }),
        )
        .route(
            "/artifact",
            axum::routing::get(move || {
                let mut bytes = base64::engine::general_purpose::STANDARD
                    .decode(ARCHIVE)
                    .unwrap();
                if corrupt_route.load(Ordering::SeqCst) {
                    bytes[0] ^= 1;
                }
                async move { bytes }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, routes).await.unwrap();
    });
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.package_info_mut().version = "1.0.0".parse().unwrap();
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let mut updater_config = config["plugins"]["updater"].clone();
    assert_eq!(updater_config["requireSignedVersion"], true);
    updater_config["pubkey"] = json!(PUBLIC_KEY);
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), updater_config);
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let updater = app
        .updater_builder()
        .executable_path(&executable)
        .endpoints(vec![format!("{base}/manifest").parse().unwrap()])
        .unwrap()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    for version in ["0.9.0", "1.0.0"] {
        manifest.lock().unwrap()["version"] = json!(version);
        assert!(
            updater.check().await.unwrap().is_none(),
            "never downgrade or reinstall {version}"
        );
    }
    manifest.lock().unwrap()["version"] = json!("2.0.1");
    let mismatched = updater.check().await.unwrap().unwrap();
    assert!(
        mismatched.download(|_, _| {}, || {}).await.is_err(),
        "signed version must match manifest"
    );
    manifest.lock().unwrap()["version"] = json!("2.0.0");
    let update = updater.check().await.unwrap().unwrap();
    assert!(
        validate_release(&update).is_err(),
        "production accepts only the fixed GitHub asset location"
    );
    let mut release = update.clone();
    release.download_url = "https://github.com/wandering-beans/OpenRoom/releases/download/v2.0.0/OpenRoom-2.0.0-macos-arm64.app.tar.gz".parse().unwrap();
    assert!(validate_release(&release).is_ok());
    release.raw_json["minimum_macos"] = json!("9999.0");
    assert!(
        validate_release(&release).is_err(),
        "unsupported macOS must not be offered"
    );
    release.raw_json["minimum_macos"] = json!("invalid");
    assert!(
        validate_release(&release).is_err(),
        "missing OS requirements must not be guessed"
    );
    corrupt.store(true, Ordering::SeqCst);
    assert!(
        update.download(|_, _| {}, || {}).await.is_err(),
        "modified archive must fail verification"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), b"original app");
    corrupt.store(false, Ordering::SeqCst);
    let mut progress = 0;
    let bytes = update
        .download(|length, _| progress += length, || {})
        .await
        .unwrap();
    assert_eq!(progress, bytes.len());
    update.install(bytes).unwrap();
    assert_eq!(
        std::fs::read(&executable).unwrap(),
        b"verified update fixture"
    );
    assert_eq!(std::fs::read(&catalog).unwrap(), b"preserved catalog");
    *manifest.lock().unwrap() = json!({"version":"invalid"});
    assert!(
        updater.check().await.is_err(),
        "invalid manifest is an error, not up to date"
    );
    server.abort();
    let _ = server.await;
    assert!(
        updater.check().await.is_err(),
        "offline is an error, not up to date"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires OPENROOM_UPDATE_PACKAGE pointing to a locally packaged release"]
async fn packaged_release_verifies_and_installs_with_the_production_public_key() {
    let package = std::path::PathBuf::from(
        std::env::var_os("OPENROOM_UPDATE_PACKAGE").expect("package directory"),
    );
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package.join("latest.json")).unwrap()).unwrap();
    let version = manifest["version"].as_str().unwrap().to_owned();
    let archive = Arc::new(
        std::fs::read(package.join(format!("OpenRoom-{version}-macos-arm64.app.tar.gz"))).unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    manifest["platforms"]["darwin-aarch64"]["url"] = json!(format!("{base}/artifact"));
    let routes = axum::Router::new()
        .route(
            "/manifest",
            axum::routing::get(move || {
                let body = manifest.clone();
                async move { axum::Json(body) }
            }),
        )
        .route(
            "/artifact",
            axum::routing::get(move || {
                let bytes = archive.as_ref().clone();
                async move { bytes }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, routes).await.unwrap();
    });
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.package_info_mut().version = "0.0.0".parse().unwrap();
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), config["plugins"]["updater"].clone());
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("OpenRoom.app");
    let executable = bundle.join("Contents/MacOS/OpenRoom");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"old application").unwrap();
    let catalog = temp.path().join("catalog.openroom");
    std::fs::write(&catalog, b"keep this catalog").unwrap();
    let updater = app
        .updater_builder()
        .executable_path(&executable)
        .endpoints(vec![format!("{base}/manifest").parse().unwrap()])
        .unwrap()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let update = updater.check().await.unwrap().unwrap();
    assert_eq!(update.version, version);
    let bytes = update.download(|_, _| {}, || {}).await.unwrap();
    update.install(bytes).unwrap();
    assert!(
        std::process::Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&bundle)
            .status()
            .unwrap()
            .success()
    );
    let installed_version = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(installed_version.stdout).unwrap().trim(),
        version
    );
    assert_eq!(std::fs::read(catalog).unwrap(), b"keep this catalog");
    server.abort();
    let _ = server.await;
}
