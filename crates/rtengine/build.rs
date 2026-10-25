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
    let output = Command::new("python3")
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
    println!(
        "cargo:rustc-link-search=native={}",
        library.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=dylib=openroom_rtengine");
    println!(
        "cargo:rustc-link-arg=-Wl,-rpath,{}",
        library.parent().unwrap().display()
    );
    println!(
        "cargo:rustc-env=OPENROOM_RTENGINE_DATA={}",
        result["data"].as_str().unwrap()
    );
}
