//! macOS: compiles the native layer (src/mac/native, Objective-C) and links
//! the system frameworks it uses. Nothing to do on Windows.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    macos();
}

fn macos() {
    let files = ["app", "capture", "record", "reader", "hud", "input", "net"];
    let mut build = cc::Build::new();
    for f in files {
        let path = format!("src/mac/native/{f}.m");
        println!("cargo:rerun-if-changed={path}");
        build.file(path);
    }
    println!("cargo:rerun-if-changed=src/mac/native/bk.h");
    build
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .flag("-mmacosx-version-min=14.0")
        .flag("-Wno-unused-parameter")
        .warnings(true)
        .compile("blinker_native");
    for fw in [
        "AppKit",
        "Foundation",
        "CoreGraphics",
        "CoreMedia",
        "CoreVideo",
        "AVFoundation",
        "ScreenCaptureKit",
        "VideoToolbox",
        "Carbon",
        "Accelerate",
        "QuartzCore",
        "AudioToolbox",
    ] {
        println!("cargo:rustc-link-lib=framework={fw}");
    }
    // `@available` checks call into clang's runtime, which rustc doesn't link
    if let Ok(out) = std::process::Command::new("clang").arg("-print-resource-dir").output() {
        let dir = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("lib/darwin");
        if dir.join("libclang_rt.osx.a").is_file() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
    }
}
