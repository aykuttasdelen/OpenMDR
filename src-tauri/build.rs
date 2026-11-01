use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/ocr_engine.swift");

    #[cfg(target_os = "macos")]
    {
        let swift_src = Path::new("native/ocr_engine.swift");
        let bin_dest = Path::new("native/ocr_engine");

        let should_compile = if !bin_dest.exists() {
            true
        } else if let (Ok(src_meta), Ok(dst_meta)) = (fs::metadata(swift_src), fs::metadata(bin_dest)) {
            match (src_meta.modified(), dst_meta.modified()) {
                (Ok(src_time), Ok(dst_time)) => src_time > dst_time,
                _ => true,
            }
        } else {
            true
        };

        if swift_src.exists() && should_compile {
            let status = Command::new("swiftc")
                .args(["-O", "native/ocr_engine.swift", "-o", "native/ocr_engine"])
                .status();

            if let Ok(st) = status {
                if !st.success() {
                    eprintln!("cargo:warning=swiftc failed to compile native/ocr_engine.swift");
                }
            } else {
                eprintln!("cargo:warning=swiftc is not available");
            }
        }
    }

    tauri_build::build();
}
