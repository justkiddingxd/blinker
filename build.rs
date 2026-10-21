//! macOS: compiles the native layer (src/mac/native, Objective-C) and links
//! the system frameworks it uses. Windows: the icon and version info of the
//! exe (assets/windows/blinker.rc).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => macos(),
        Ok("windows") => windows(),
        _ => {}
    }
}

fn windows() {
    println!("cargo:rerun-if-changed=assets/windows/blinker.rc");
    println!("cargo:rerun-if-changed=assets/windows/blinker.ico");
    // the resource compiler comes with the Windows SDK; a check from
    // another system goes without the icon
    if !cfg!(windows) {
        return;
    }
    // the version and the icon's full path go into a copy of the script
    let v = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let nums: Vec<String> = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0).to_string()).collect();
    let num = |i: usize| nums.get(i).cloned().unwrap_or_else(|| "0".into());
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("windows");
    let icon = dir.join("blinker.ico").display().to_string().replace('\\', "\\\\");
    let rc = std::fs::read_to_string(dir.join("blinker.rc"))
        .unwrap()
        .replace("\"blinker.ico\"", &format!("\"{icon}\""))
        .replace("BK_MAJOR", &num(0))
        .replace("BK_MINOR", &num(1))
        .replace("BK_PATCH", &num(2))
        .replace("BK_VERSION", &format!("\"{v}\""));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("blinker.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
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
