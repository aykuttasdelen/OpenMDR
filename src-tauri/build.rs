use std::path::Path;
use std::process::Command;

fn main() {
    #[cfg(target_os = "macos")]
    {
        let swift_src = Path::new("native/ocr_engine.swift");
        let bin_dest = Path::new("native/ocr_engine");

        if swift_src.exists() && !bin_dest.exists() {
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

