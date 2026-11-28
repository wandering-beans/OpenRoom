fn main() {
    println!("cargo:rerun-if-changed=../../native/rtengine/lensfun/sha256.txt");
    let digest = include_str!("../../native/rtengine/lensfun/sha256.txt").trim();
    println!("cargo:rustc-env=OPENROOM_LENS_DB_VERSION={}", &digest[..12]);
    println!("cargo:rustc-check-cfg=cfg(desktop_rtengine)");
    if matches!(std::env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("windows" | "macos" | "linux")) {
        println!("cargo:rustc-cfg=desktop_rtengine");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var_os("CARGO_FEATURE_VULKAN").is_some()
    {
        println!("cargo:rerun-if-changed=native/vulkan.cpp");
        println!("cargo:rerun-if-changed=native/color.hlsl");
        let output =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("color.spv");
        let status = std::process::Command::new("glslangValidator")
            .args(["-D", "-DOPENROOM_VULKAN=1", "-V", "--target-env", "vulkan1.0"])
            .args(["-S", "comp", "-e", "correct_color", "-o"])
            .arg(&output)
            .arg("native/color.hlsl")
            .status()
            .expect("Install glslang-tools to build the Vulkan color shader");
        assert!(status.success(), "Vulkan color shader compilation failed");
        cc::Build::new()
            .cpp(true)
            .file("native/vulkan.cpp")
            .std("c++17")
            .compile("openroom_vulkan");
        println!("cargo:rustc-link-lib=dl");
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
