use std::path::Path;
use std::process::Command;

fn strip_optional_quotes(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn load_oauth_env_from_dotenv() {
    const KEYS: [&str; 2] = ["OPENMD_GOOGLE_CLIENT_ID", "OPENMD_GOOGLE_CLIENT_SECRET"];
    for key in KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-changed=../.env.local");

    let missing_keys: Vec<&str> = KEYS
        .into_iter()
        .filter(|key| {
            std::env::var(key)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true)
        })
        .collect();

    if missing_keys.is_empty() {
        return;
    }

    let dotenv_path = Path::new("../.env.local");
    let Ok(contents) = std::fs::read_to_string(dotenv_path) else {
        return;
    };

    for raw_line in contents.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
        let Some((raw_key, raw_val)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if !missing_keys.contains(&key) {
            continue;
        }
        let clean_val = strip_optional_quotes(raw_val.trim()).replace(['\r', '\n'], "");
        if !clean_val.is_empty() {
            println!("cargo:rustc-env={key}={clean_val}");
        }
    }
}

fn main() {
    load_oauth_env_from_dotenv();

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

