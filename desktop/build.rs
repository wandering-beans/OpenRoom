fn main() {
    println!("cargo:rerun-if-changed=native/Preview.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos" {
        let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            other => panic!("Unsupported macOS architecture: {other}"),
        };
        let output = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ae-preview");
        let status = std::process::Command::new("xcrun")
            .args(["swiftc", "-O", "native/Preview.swift", "-o"])
            .arg(output)
            .args(["-target", &format!("{arch}-apple-macosx14.0")])
            .arg("-module-cache-path")
            .arg(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("swift-cache"))
            .status()
            .expect("Swift compiler is required for macOS previews");
        assert!(status.success(), "native renderer compilation failed");
    }
    tauri_build::build()
}
