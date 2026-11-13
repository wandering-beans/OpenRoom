fn main() {
    println!("cargo:rustc-check-cfg=cfg(desktop_rtengine)");
    if matches!(std::env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("windows" | "macos" | "linux")) {
        println!("cargo:rustc-cfg=desktop_rtengine");
    }
    println!("cargo:rerun-if-changed=native/metal.mm");
    println!("cargo:rerun-if-changed=native/direct3d.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_DIRECT3D").is_some()
    {
        cc::Build::new()
            .cpp(true)
            .file("native/direct3d.cpp")
            .std("c++17")
            .compile("openroom_direct3d");
        for library in ["d3d11", "dxgi", "d3dcompiler"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || std::env::var_os("CARGO_FEATURE_METAL").is_none()
    {
        return;
    }
    cc::Build::new()
        .cpp(true)
        .file("native/metal.mm")
        .flag("-fobjc-arc")
        .flag("-std=c++17")
        .compile("openroom_metal");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=Foundation");
}
