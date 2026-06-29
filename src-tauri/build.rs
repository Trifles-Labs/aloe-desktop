fn main() {
    copy_vosk_runtime_libs();
    tauri_build::build();
}

fn target_exe_dir() -> Option<std::path::PathBuf> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").ok()?);
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the exe lives at target/<profile>.
    out_dir.ancestors().nth(3).map(std::path::Path::to_path_buf)
}

/// `vosk-sys` links `libvosk.lib` at build time (via `.cargo/config.toml`), but the
/// matching `libvosk.dll` + its MinGW runtime DLLs still need to sit next to the
/// compiled executable for the dynamic loader to find at process start.
fn copy_vosk_runtime_libs() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vendor_dir = manifest_dir.join("vendor").join("vosk");
    let Some(exe_dir) = target_exe_dir() else { return };

    if target_os == "linux" {
        let src = vendor_dir.join("libvosk.so");
        println!("cargo:rerun-if-changed={}", src.display());
        if src.exists() {
            let _ = std::fs::copy(&src, exe_dir.join("libvosk.so"));
        }
        return;
    }

    if target_os != "windows" {
        return;
    }

    for name in ["libvosk.dll", "libgcc_s_seh-1.dll", "libstdc++-6.dll", "libwinpthread-1.dll"] {
        let src = vendor_dir.join(name);
        println!("cargo:rerun-if-changed={}", src.display());
        if src.exists() {
            let _ = std::fs::copy(&src, exe_dir.join(name));
        }
    }
}
