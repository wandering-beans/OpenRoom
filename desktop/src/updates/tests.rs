use super::*;
use base64::Engine;
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex, atomic::AtomicBool};

// A disposable test key signs only this tiny synthetic app; it is unrelated to release signing.
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJGREI5MjQ5RTMyMjYyNzMKUldSellpTGpTWkxidi9CdEY0ZUxGUEp5dTBvUFFoNTA3WlRPQVZzb1liL0ZTTEJETXo1MGUzbUoK";
const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSellpTGpTWkxidjNsWi9COHMzTDBpWjhJNngrYTRJNTQ3ZnlEdFhGbVpnZnVoQmUrYUhWWGZCdmJMSHAwb3Y0V1k3a28wbUJoYUhLVnpnQXp6OVRZa1MvcG9YV1hIR1EwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzOTMwMDI1CWZpbGU6T3BlblJvb20tMi4wLjAtbWFjb3MtYXJtNjQuYXBwLnRhci5negl2ZXJzaW9uOjIuMC4wCjR1NXBrcDYxN2wvOE92QXBTMk1HVGlzdFVDRkNSMVVRbDVaclNoTGhQbVdwbURMM2VQMno4aGxPdlNSZVdwaGNNSmlRVlBsVUc0VFRld2g3WGRnR0F3PT0K";
const WINDOWS_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSellpTGpTWkxidjVVbUdCOThXeWVjZnB0RUoyT05Kd1o1bEltM1dzeEF1QUtaZ3E1djVBbVIwM1M2SXJ4NlNtV3g3VllqZ3JMSFkrNzNiVTJaMHUrd2MwVHlXZU9DdWc4PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzOTMwMDI2CWZpbGU6T3BlblJvb20tMi4wLjAtd2luZG93cy14NjQtc2V0dXAuZXhlCXZlcnNpb246Mi4wLjAKSWt1cjM0ZVFwMVQ2YW5pWXFBaktWdHRXdXNMYmZabFBjUmFadnpzMXlYTzZQeGhqZEkxc2xPU2hxMVpXNDVuMkdkcWlMejI3M0c4cmtFbHRRdVB2Q0E9PQo=";
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
        .target("darwin-aarch64")
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
    release.raw_json["platforms"]["darwin-aarch64"]["url"] = json!(release.download_url.as_str());
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
    validate_verified_package(&update, "darwin-aarch64", &bytes).unwrap();
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
#[ignore = "requires OPENROOM_UPDATE_PACKAGE pointing to a locally packaged release"]
async fn packaged_release_verifies_and_installs_with_the_production_public_key() {
    let package = std::path::PathBuf::from(
        std::env::var_os("OPENROOM_UPDATE_PACKAGE").expect("package directory"),
    );
    let target = update_target().unwrap();
    let feed = if cfg!(target_os = "linux") { "linux-x86_64.json" } else { "macos-aarch64.json" };
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package.join(feed)).unwrap()).unwrap();
    let version = manifest["version"].as_str().unwrap().to_owned();
    let archive = Arc::new(
        std::fs::read(package.join(asset_name(&version, target).unwrap())).unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    manifest["platforms"][target]["url"] = json!(format!("{base}/artifact"));
    #[cfg(target_os = "linux")]
    let expected_bytes = Arc::clone(&archive);
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
    #[cfg(target_os = "linux")]
    let config = {
        let mut config = config;
        let linux: serde_json::Value = serde_json::from_str(include_str!("../../tauri.linux.conf.json")).unwrap();
        config["plugins"]["updater"]["pubkey"] = linux["plugins"]["updater"]["pubkey"].clone();
        config
    };
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
    #[cfg(target_os = "macos")]
    let bundle = temp.path().join("OpenRoom.app");
    #[cfg(target_os = "linux")]
    let executable = temp.path().join("OpenRoom.AppImage");
    #[cfg(target_os = "macos")]
    let executable = bundle.join("Contents/MacOS/OpenRoom");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"old application").unwrap();
    let catalog = temp.path().join("catalog.openroom");
    std::fs::write(&catalog, b"keep this catalog").unwrap();
    let updater = app
        .updater_builder()
        .target(target)
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
    validate_verified_package(&update, target, &bytes).unwrap();
    update.install(bytes).unwrap();
    #[cfg(target_os = "linux")]
    assert_eq!(std::fs::read(&executable).unwrap().as_slice(), expected_bytes.as_slice());
    #[cfg(target_os = "macos")]
    {
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
    }
    assert_eq!(std::fs::read(catalog).unwrap(), b"keep this catalog");
    server.abort();
    let _ = server.await;
}

#[test]
fn installed_update_guard_rejects_development_folders_and_other_platforms() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("advanced-explorer-desktop.exe");
    std::fs::write(&executable, b"fixture").unwrap();
    assert!(!installed_application(&executable, "windows-x86_64"));
    std::fs::write(temp.path().join("uninstall.exe"), b"fixture").unwrap();
    std::fs::write(
        temp.path().join("openroom-install-target"),
        "darwin-aarch64",
    )
    .unwrap();
    assert!(!installed_application(&executable, "windows-x86_64"));
    std::fs::write(
        temp.path().join("openroom-install-target"),
        "local.advanced-explorer.desktop/windows-x86_64/nsis\n",
    )
    .unwrap();
    assert!(installed_application(&executable, "windows-x86_64"));
    assert!(!installed_application(&executable, "darwin-aarch64"));
    let mac = temp.path().join("OpenRoom.app/Contents/MacOS/OpenRoom");
    assert!(installed_application(&mac, "darwin-aarch64"));
    assert!(!installed_application(&mac, "windows-x86_64"));
}

#[tokio::test]
async fn update_selection_never_crosses_os_and_verifies_signature_before_payload_policy() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let manifest = Arc::new(StdMutex::new(
        json!({"version":"2.0.0", "minimum_macos":"0.0",
        "platforms": {
            "darwin-aarch64":{"url":format!("{base}/mac"), "signature":SIGNATURE},
            "windows-x86_64":{"url":format!("{base}/win"), "signature":WINDOWS_SIGNATURE}
        }}),
    ));
    let corrupt = Arc::new(AtomicBool::new(false));
    let manifest_route = manifest.clone();
    let corrupt_route = corrupt.clone();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(ARCHIVE)
        .unwrap();
    let windows_corrupt = corrupt.clone();
    let windows_artifact = axum::routing::get(move || {
        let mut bytes = vec![0; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..70].copy_from_slice(b"PE\0\0\x4c\x01");
        if windows_corrupt.load(Ordering::SeqCst) {
            bytes[0] ^= 1;
        }
        async move { bytes }
    });
    let artifact = axum::routing::get(move || {
        let mut bytes = bytes.clone();
        if corrupt_route.load(Ordering::SeqCst) {
            bytes[0] ^= 1;
        }
        async move { bytes }
    });
    let routes = axum::Router::new()
        .route(
            "/manifest",
            axum::routing::get(move || {
                let body = manifest_route.lock().unwrap().clone();
                async move { axum::Json(body) }
            }),
        )
        .route("/mac", artifact)
        .route("/win", windows_artifact);
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
    for (target, route, other) in [
        ("windows-x86_64", "win", "darwin-aarch64"),
        ("darwin-aarch64", "mac", "windows-x86_64"),
    ] {
        let updater = app
            .updater_builder()
            .target(target)
            .endpoints(vec![format!("{base}/manifest").parse().unwrap()])
            .unwrap()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let update = updater.check().await.unwrap().unwrap();
        assert_eq!(update.download_url.as_str(), format!("{base}/{route}"));
        let verified = update.download(|_, _| {}, || {}).await.unwrap();
        // Real signatures bind both the platform filename and version.
        assert!(validate_verified_package(&update, target, &verified).is_ok());
        assert!(validate_verified_package(&update, other, &verified).is_err());
        let mut release = update.clone();
        let expected = format!(
            "https://github.com/wandering-beans/OpenRoom/releases/download/v2.0.0/{}",
            asset_name("2.0.0", target).unwrap()
        );
        release.download_url = expected.parse().unwrap();
        release.raw_json["platforms"][target]["url"] = json!(expected);
        assert!(validate_target_release(&release, target).is_ok());
        assert!(validate_target_release(&release, other).is_err());
        release.download_url = format!(
            "https://github.com/wandering-beans/OpenRoom/releases/download/v2.0.0/{}",
            asset_name("2.0.0", other).unwrap()
        )
        .parse()
        .unwrap();
        release.raw_json["platforms"][target]["url"] = json!(release.download_url.as_str());
        assert!(validate_target_release(&release, target).is_err());
        let missing = manifest.lock().unwrap()["platforms"]
            .as_object_mut()
            .unwrap()
            .remove(target)
            .unwrap();
        assert!(
            updater.check().await.is_err(),
            "missing target must never select the other OS"
        );
        manifest.lock().unwrap()["platforms"][target] = missing;
        corrupt.store(true, Ordering::SeqCst);
        assert!(update.download(|_, _| {}, || {}).await.is_err());
        corrupt.store(false, Ordering::SeqCst);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&update.signature)
            .unwrap();
        let relabeled = String::from_utf8(decoded).unwrap().replace(
            &asset_name("2.0.0", target).unwrap(),
            &asset_name("2.0.0", other).unwrap(),
        );
        manifest.lock().unwrap()["platforms"][target]["signature"] =
            json!(base64::engine::general_purpose::STANDARD.encode(relabeled));
        assert!(
            updater
                .check()
                .await
                .unwrap()
                .unwrap()
                .download(|_, _| {}, || {})
                .await
                .is_err(),
            "the trusted filename cannot change without resigning the payload"
        );
        manifest.lock().unwrap()["platforms"][target]["signature"] = json!(update.signature);
        manifest.lock().unwrap()["version"] = json!("2.0.1");
        assert!(
            updater
                .check()
                .await
                .unwrap()
                .unwrap()
                .download(|_, _| {}, || {})
                .await
                .is_err()
        );
        for version in ["0.9.0", "1.0.0"] {
            manifest.lock().unwrap()["version"] = json!(version);
            assert!(updater.check().await.unwrap().is_none());
        }
        manifest.lock().unwrap()["version"] = json!("2.0.0");
        // Unit-test the post-verification format policy with synthetic headers, never execute them.
        let filename = asset_name("2.0.0", target).unwrap();
        release.signature = base64::engine::general_purpose::STANDARD
            .encode(format!("trusted comment: file:{filename}\tversion:2.0.0\n"));
        let mut pe = vec![0; 128];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        pe[64..70].copy_from_slice(b"PE\0\0\x4c\x01");
        let gzip = [0x1f, 0x8b, 0, 0];
        let (valid, wrong): (&[u8], &[u8]) = if target == "windows-x86_64" {
            (&pe, &gzip)
        } else {
            (&gzip, &pe)
        };
        assert!(validate_verified_package(&release, target, valid).is_ok());
        assert!(validate_verified_package(&release, target, wrong).is_err());
        assert!(validate_verified_package(&release, other, valid).is_err());
    }
    server.abort();
}

#[test]
fn startup_update_check_runs_once_per_process_only_when_enabled_and_installed() {
    for (enabled, installed, expected) in [(true, true, true), (false, true, false), (true, false, false)] {
        let state = UpdateState::default();
        assert_eq!(state.claim_startup_check(enabled, installed), expected);
        assert!(!state.claim_startup_check(true, true));
    }
}

#[test]
fn linux_updates_require_a_64_bit_x86_type_two_appimage() {
    let mut bytes = [0; 64];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[8..11].copy_from_slice(b"AI\x02");
    bytes[18] = 62;
    assert!(linux_appimage_header(&bytes));
    assert!(!linux_appimage_header(&bytes[..32]));
    bytes[18] = 183;
    assert!(!linux_appimage_header(&bytes));
    bytes[18] = 62;
    bytes[10] = 0;
    assert!(!linux_appimage_header(&bytes));
}
