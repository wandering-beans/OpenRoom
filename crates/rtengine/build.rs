use std::{path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    println!(
        "cargo:rerun-if-changed={}",
        root.join("native/rtengine").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("scripts/build-rtengine.py").display()
    );
    for name in ["OPENROOM_PYTHON", "OPENROOM_MSYS2_PREFIX"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    assert_eq!(
        std::env::var("HOST").unwrap(),
        std::env::var("TARGET").unwrap(),
        "rtengine requires a native build on the target OS"
    );
    let python = std::env::var_os("OPENROOM_PYTHON")
        .unwrap_or_else(|| if windows { "python" } else { "python3" }.into());
    let output = Command::new(python)
        .arg(root.join("scripts/build-rtengine.py"))
        .output()
        .expect("Python and the rtengine build dependencies are required");
    assert!(
        output.status.success(),
        "rtengine build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Invalid rtengine build result");
    let library = PathBuf::from(result["library"].as_str().unwrap());
    if windows {
        // raw-dylib generates the MSVC import library for the exported C ABI.
        // Cargo test binaries run in deps; desktop binaries run in the profile directory.
        let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let profile = out.ancestors().nth(3).expect("Cargo profile directory");
        for directory in [profile.to_path_buf(), profile.join("deps")] {
            std::fs::create_dir_all(&directory).unwrap();
            for dll in std::iter::once(library.clone()).chain(
                result["runtime"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| PathBuf::from(value.as_str().unwrap())),
            ) {
                let destination = directory.join(dll.file_name().unwrap());
                if std::fs::read(&destination).ok() != Some(std::fs::read(&dll).unwrap()) {
                    std::fs::copy(&dll, &destination).expect("Cannot stage rtengine runtime DLL");
                }
            }
        }
    } else {
        println!(
            "cargo:rustc-link-search=native={}",
            library.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=dylib=openroom_rtengine");
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            library.parent().unwrap().display()
        );
    }
    println!(
        "cargo:rustc-env=OPENROOM_RTENGINE_DATA={}",
        result["data"].as_str().unwrap()
    );
}
