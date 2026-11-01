fn main() {
    println!("cargo:rerun-if-changed=native/metal.mm");
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
