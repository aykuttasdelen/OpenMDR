use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/ocr_engine.swift");

    #[cfg(target_os = "macos")]
    {
        let swift_src = Path::new("native/ocr_engine.swift");

        if swift_src.exists() {
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
