//! Stage `source-daemon` sidecar next to Tauri binaries before `tauri_build`.
//! Prefer artifacts from the workspace `target/{release,debug}` directories.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    match ensure_sidecar() {
        Ok(path) => eprintln!("source-desktop: sidecar ready at {}", path.display()),
        Err(e) => println!(
            "cargo:warning=source-daemon sidecar not staged ({e}). Build source-daemon first or use scripts/build-desktop.*"
        ),
    }
    tauri_build::build()
}

fn ensure_sidecar() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let triple = env::var("TARGET").unwrap_or_else(|_| host_triple().unwrap_or_default());
    if triple.is_empty() {
        return Err("could not determine target triple".into());
    }

    let bin_dir = manifest_dir.join("binaries");
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;

    let file_name = if triple.contains("windows") {
        format!("source-daemon-{triple}.exe")
    } else {
        format!("source-daemon-{triple}")
    };
    let dest = bin_dir.join(&file_name);
    println!("cargo:rerun-if-changed={}", dest.display());
    println!("cargo:rerun-if-env-changed=TARGET");

    if dest.exists() {
        return Ok(dest);
    }

    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| "cannot resolve workspace root".to_string())?;

    let bin_name = if triple.contains("windows") {
        "source-daemon.exe"
    } else {
        "source-daemon"
    };

    let candidates = [
        workspace_root.join("target").join("release").join(bin_name),
        workspace_root.join("target").join("debug").join(bin_name),
        workspace_root
            .join("target")
            .join(&triple)
            .join("release")
            .join(bin_name),
        workspace_root
            .join("target")
            .join(&triple)
            .join("debug")
            .join(bin_name),
    ];

    for src in &candidates {
        println!("cargo:rerun-if-changed={}", src.display());
        if src.exists() {
            fs::copy(src, &dest).map_err(|e| format!("copy {}: {e}", src.display()))?;
            return Ok(dest);
        }
    }

    Err(format!(
        "missing {file_name}; run cargo build -p source-daemon from repo root"
    ))
}

fn host_triple() -> Option<String> {
    let out = Command::new("rustc").args(["-vV"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}
