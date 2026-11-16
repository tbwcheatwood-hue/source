//! Offline app factory: templates under `~/.source/templates/` + vendor cache helpers.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

pub const BUILTIN_TEMPLATES: &[&str] = &[
    "react-vite-tailwind",
    "tauri-v2-react",
    "python-fastapi-pyside",
];

/// Bump when builtin seed contents change so `ensure_seeded` refreshes stale copies.
pub const BUILTIN_TEMPLATE_VERSION: u64 = 2;

pub const DESKTOP_TEMPLATE_ID: &str = "tauri-v2-react";

pub const LINUX_TAURI_APT_PACKAGES: &[&str] = &[
    "libwebkit2gtk-4.1-dev",
    "libgtk-3-dev",
    "librsvg2-dev",
    "patchelf",
    "pkg-config",
    "build-essential",
    "curl",
    "wget",
    "file",
    "libssl-dev",
    "libayatana-appindicator3-dev",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub path: PathBuf,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorCacheStatus {
    pub root: PathBuf,
    pub npm_dir: PathBuf,
    pub pnpm_dir: PathBuf,
    pub cargo_dir: PathBuf,
    pub wheels_dir: PathBuf,
    pub npm_bytes: u64,
    pub pnpm_bytes: u64,
    pub cargo_bytes: u64,
    pub wheels_bytes: u64,
    pub helpers: VendorHelpers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorHelpers {
    pub npm_offline_install: String,
    pub pnpm_offline_install: String,
    pub cargo_vendor: String,
    pub pip_wheelhouse: String,
    pub create_desktop_app: String,
    pub build_desktop: String,
    pub linux_tauri_deps: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopScaffoldResult {
    pub template_id: String,
    pub dest: PathBuf,
    pub name: String,
    pub build_script: PathBuf,
    pub build_command: String,
    pub linux_deps_apt: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopBuildResult {
    pub dest: PathBuf,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub command: String,
}

#[derive(Clone)]
pub struct TemplateManager {
    templates_dir: PathBuf,
    vendor_dir: PathBuf,
    bundled_templates_dir: PathBuf,
}

impl TemplateManager {
    pub fn new(templates_dir: PathBuf, vendor_dir: PathBuf, bundled_templates_dir: PathBuf) -> Self {
        Self {
            templates_dir,
            vendor_dir,
            bundled_templates_dir,
        }
    }

    pub fn ensure_seeded(&self) -> Result<()> {
        fs::create_dir_all(&self.templates_dir)?;
        fs::create_dir_all(self.vendor_dir.join("npm"))?;
        fs::create_dir_all(self.vendor_dir.join("pnpm"))?;
        fs::create_dir_all(self.vendor_dir.join("cargo"))?;
        fs::create_dir_all(self.vendor_dir.join("wheels"))?;

        for id in BUILTIN_TEMPLATES {
            let dest = self.templates_dir.join(id);
            let needs_refresh = template_needs_refresh(&dest);
            if dest.exists() && !needs_refresh {
                continue;
            }
            if dest.exists() {
                fs::remove_dir_all(&dest)?;
            }
            if let Some(src) = resolve_bundled_template(id, &self.bundled_templates_dir) {
                copy_dir(&src, &dest)?;
            } else {
                seed_builtin_template(id, &dest)?;
            }
            write_manifest(id, &dest)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<TemplateInfo>> {
        self.ensure_seeded()?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.templates_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let manifest = entry.path().join("template.json");
            let (title, description) = if manifest.exists() {
                let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&manifest)?)?;
                (
                    v.get("title")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&id)
                        .to_string(),
                    v.get("description")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                (id.clone(), String::new())
            };
            out.push(TemplateInfo {
                builtin: BUILTIN_TEMPLATES.contains(&id.as_str()),
                id,
                title,
                description,
                path: entry.path(),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn scaffold(&self, template_id: &str, dest: &Path, name: Option<&str>) -> Result<PathBuf> {
        self.ensure_seeded()?;
        let src = self.templates_dir.join(template_id);
        if !src.exists() {
            return Err(anyhow!("unknown template {template_id}"));
        }
        if dest.exists() && dest.read_dir()?.next().is_some() {
            return Err(anyhow!(
                "destination {} is not empty",
                dest.display()
            ));
        }
        fs::create_dir_all(dest)?;
        copy_dir(&src, dest)?;
        // Drop template metadata from scaffolded app.
        let _ = fs::remove_file(dest.join("template.json"));
        if let Some(name) = name {
            rewrite_package_name(dest, name)?;
        }
        // Ensure build script is executable on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dest.join("scripts/build-desktop.sh");
            if script.exists() {
                let mut perms = fs::metadata(&script)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&script, perms)?;
            }
        }
        Ok(dest.to_path_buf())
    }

    /// Create a finished Tauri v2 desktop app from the builtin template.
    pub fn create_desktop_app(&self, dest: &Path, name: Option<&str>) -> Result<DesktopScaffoldResult> {
        let app_name = name.unwrap_or("desktop-app");
        let dest = self.scaffold(DESKTOP_TEMPLATE_ID, dest, Some(app_name))?;
        let build_script = dest.join("scripts/build-desktop.sh");
        Ok(DesktopScaffoldResult {
            template_id: DESKTOP_TEMPLATE_ID.to_string(),
            dest: dest.clone(),
            name: app_name.to_string(),
            build_script: build_script.clone(),
            build_command: if build_script.exists() {
                "./scripts/build-desktop.sh".into()
            } else {
                "pnpm tauri build".into()
            },
            linux_deps_apt: linux_tauri_apt_install_command(),
            notes: vec![
                "Complete Tauri v2 + React desktop project — not an HTML substitute.".into(),
                "If WebKitGTK/-dev packages are missing, install them then re-run the build script.".into(),
                "Customize src/ and src-tauri/; keep scripts/build-desktop.sh as the ship path.".into(),
            ],
        })
    }

    /// Run the scaffolded (or Source) desktop build script / tauri build.
    pub fn build_desktop(&self, project: &Path, install_deps: bool) -> Result<DesktopBuildResult> {
        let script = project.join("scripts/build-desktop.sh");
        let command = if script.exists() {
            if install_deps {
                format!("{} --install-deps", script.display())
            } else {
                script.display().to_string()
            }
        } else if project.join("src-tauri").exists() {
            "pnpm tauri build".to_string()
        } else {
            return Err(anyhow!(
                "{} is not a Tauri desktop project (missing scripts/build-desktop.sh and src-tauri)",
                project.display()
            ));
        };

        let mut cmd = if script.exists() {
            let mut c = Command::new("bash");
            c.arg(&script);
            if install_deps {
                c.arg("--install-deps");
            }
            c.current_dir(project);
            c
        } else {
            let mut c = Command::new("pnpm");
            c.args(["tauri", "build"]);
            c.current_dir(project);
            c
        };

        let output = cmd.output().context("desktop build command failed to start")?;
        Ok(DesktopBuildResult {
            dest: project.to_path_buf(),
            ok: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            command,
        })
    }

    pub fn vendor_status(&self) -> Result<VendorCacheStatus> {
        self.ensure_seeded()?;
        let npm = self.vendor_dir.join("npm");
        let pnpm = self.vendor_dir.join("pnpm");
        let cargo = self.vendor_dir.join("cargo");
        let wheels = self.vendor_dir.join("wheels");
        Ok(VendorCacheStatus {
            root: self.vendor_dir.clone(),
            npm_dir: npm.clone(),
            pnpm_dir: pnpm.clone(),
            cargo_dir: cargo.clone(),
            wheels_dir: wheels.clone(),
            npm_bytes: dir_size(&npm),
            pnpm_bytes: dir_size(&pnpm),
            cargo_bytes: dir_size(&cargo),
            wheels_bytes: dir_size(&wheels),
            helpers: VendorHelpers {
                npm_offline_install: format!(
                    "npm ci --offline --cache {}",
                    npm.display()
                ),
                pnpm_offline_install: format!(
                    "pnpm install --offline --store-dir {}",
                    pnpm.display()
                ),
                cargo_vendor: format!(
                    "cargo vendor {} && echo '[source.crates-io]\\nreplace-with = \"vendored-sources\"\\n[source.vendored-sources]\\ndirectory = \"{}\"' >> .cargo/config.toml",
                    cargo.display(),
                    cargo.display()
                ),
                pip_wheelhouse: format!(
                    "pip install --no-index --find-links {} -r requirements.txt",
                    wheels.display()
                ),
                create_desktop_app: format!(
                    "templates_ops op=create_desktop_app template={} (or source/templates/scaffold)",
                    DESKTOP_TEMPLATE_ID
                ),
                build_desktop: "./scripts/build-desktop.sh [--install-deps]".into(),
                linux_tauri_deps: linux_tauri_apt_install_command(),
            },
        })
    }

    /// Populate vendor caches from an existing project (offline helpers).
    pub fn populate_from_project(&self, project: &Path, kind: &str) -> Result<VendorCacheStatus> {
        match kind {
            "npm" => {
                let cache = self.vendor_dir.join("npm");
                let status = std::process::Command::new("npm")
                    .args(["pack", "--pack-destination"])
                    .arg(&cache)
                    .current_dir(project)
                    .status()
                    .context("npm pack")?;
                if !status.success() {
                    // Fallback: copy node_modules if present
                    let nm = project.join("node_modules");
                    if nm.exists() {
                        copy_dir(&nm, &cache.join("node_modules"))?;
                    }
                }
            }
            "pnpm" => {
                let store = self.vendor_dir.join("pnpm");
                let _ = std::process::Command::new("pnpm")
                    .args(["store", "path"])
                    .current_dir(project)
                    .output();
                let _ = std::process::Command::new("pnpm")
                    .args(["fetch", "--store-dir"])
                    .arg(&store)
                    .current_dir(project)
                    .status();
            }
            "cargo" => {
                let vendor = self.vendor_dir.join("cargo");
                let status = std::process::Command::new("cargo")
                    .args(["vendor"])
                    .arg(&vendor)
                    .current_dir(project)
                    .status()
                    .context("cargo vendor")?;
                if !status.success() {
                    return Err(anyhow!("cargo vendor failed"));
                }
            }
            "pip" | "wheels" => {
                let wheels = self.vendor_dir.join("wheels");
                let req = project.join("requirements.txt");
                if req.exists() {
                    let status = std::process::Command::new("pip")
                        .args(["download", "-r"])
                        .arg(&req)
                        .arg("-d")
                        .arg(&wheels)
                        .status()
                        .context("pip download")?;
                    if !status.success() {
                        return Err(anyhow!("pip download failed"));
                    }
                } else {
                    return Err(anyhow!("no requirements.txt in {}", project.display()));
                }
            }
            other => return Err(anyhow!("unknown vendor kind {other}")),
        }
        self.vendor_status()
    }
}

pub fn linux_tauri_apt_install_command() -> String {
    format!(
        "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y {}",
        LINUX_TAURI_APT_PACKAGES.join(" ")
    )
}

fn resolve_bundled_template(id: &str, bundled_templates_dir: &Path) -> Option<PathBuf> {
    let primary = bundled_templates_dir.join(id);
    if primary.exists() {
        return Some(primary);
    }
    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join(id);
    if fallback.exists() {
        return Some(fallback);
    }
    None
}

fn template_needs_refresh(dest: &Path) -> bool {
    if !dest.exists() {
        return true;
    }
    // Incomplete legacy seeds lacked a real src-tauri host.
    if dest.file_name().and_then(|s| s.to_str()) == Some(DESKTOP_TEMPLATE_ID) {
        let complete = dest.join("src-tauri/src/lib.rs").exists()
            && dest.join("scripts/build-desktop.sh").exists()
            && dest.join("src/App.tsx").exists();
        if !complete {
            return true;
        }
    }
    let manifest = dest.join("template.json");
    if !manifest.exists() {
        return true;
    }
    let Ok(text) = fs::read_to_string(&manifest) else {
        return true;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return true;
    };
    let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(0);
    version < BUILTIN_TEMPLATE_VERSION
}

fn write_manifest(id: &str, dest: &Path) -> Result<()> {
    let (title, description, kind) = match id {
        "react-vite-tailwind" => (
            "React + Vite + Tailwind",
            "SPA scaffold with Vite, React, TypeScript, and Tailwind CSS",
            "web",
        ),
        "tauri-v2-react" => (
            "Tauri v2 + Rust + React",
            "Complete offline desktop app scaffold (Tauri v2, React, calculator-class UI)",
            "desktop",
        ),
        "python-fastapi-pyside" => (
            "Python FastAPI / PySide",
            "API + optional desktop UI with FastAPI and PySide6",
            "desktop",
        ),
        _ => (id, "Custom template", "app"),
    };
    let manifest = serde_json::json!({
        "id": id,
        "title": title,
        "description": description,
        "version": BUILTIN_TEMPLATE_VERSION,
        "kind": kind,
        "build": if kind == "desktop" { "./scripts/build-desktop.sh" } else { "pnpm build" },
    });
    fs::write(dest.join("template.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(())
}

fn seed_builtin_template(id: &str, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    match id {
        "react-vite-tailwind" => seed_react_vite(dest)?,
        "tauri-v2-react" => seed_tauri_v2_react(dest)?,
        "python-fastapi-pyside" => seed_python_fastapi(dest)?,
        _ => return Err(anyhow!("no builtin seed for {id}")),
    }
    Ok(())
}

fn seed_react_vite(dest: &Path) -> Result<()> {
    fs::write(
        dest.join("package.json"),
        r#"{
  "name": "source-app",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.4",
    "autoprefixer": "^10.4.20",
    "postcss": "^8.4.49",
    "tailwindcss": "^3.4.17",
    "typescript": "^5.7.2",
    "vite": "^6.0.3"
  }
}
"#,
    )?;
    fs::write(
        dest.join("index.html"),
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Source App</title>
  </head>
  <body class="bg-slate-950 text-slate-50">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#,
    )?;
    fs::create_dir_all(dest.join("src"))?;
    fs::write(
        dest.join("src/main.tsx"),
        r#"import React from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
"#,
    )?;
    fs::write(
        dest.join("src/App.tsx"),
        r#"export function App() {
  return (
    <main className="min-h-screen grid place-items-center p-8">
      <div>
        <h1 className="text-4xl font-semibold tracking-tight">Source App</h1>
        <p className="mt-3 text-slate-400">React + Vite + Tailwind scaffold.</p>
      </div>
    </main>
  );
}
"#,
    )?;
    fs::write(
        dest.join("src/index.css"),
        "@tailwind base;\n@tailwind components;\n@tailwind utilities;\n",
    )?;
    fs::write(
        dest.join("tailwind.config.js"),
        "/** @type {import('tailwindcss').Config} */\nexport default { content: ['./index.html','./src/**/*.{js,ts,jsx,tsx}'], theme: { extend: {} }, plugins: [] }\n",
    )?;
    fs::write(
        dest.join("postcss.config.js"),
        "export default { plugins: { tailwindcss: {}, autoprefixer: {} } }\n",
    )?;
    fs::write(
        dest.join("vite.config.ts"),
        "import { defineConfig } from 'vite';\nimport react from '@vitejs/plugin-react';\nexport default defineConfig({ plugins: [react()] });\n",
    )?;
    fs::write(
        dest.join("tsconfig.json"),
        r#"{"compilerOptions":{"target":"ES2022","lib":["ES2022","DOM","DOM.Iterable"],"module":"ESNext","skipLibCheck":true,"moduleResolution":"bundler","jsx":"react-jsx","strict":true,"noEmit":true},"include":["src"]}"#,
    )?;
    Ok(())
}

fn seed_tauri_v2_react(dest: &Path) -> Result<()> {
    // Prefer copying the shipped bundle when available (keeps seed and files identical).
    if let Some(src) = resolve_bundled_template(DESKTOP_TEMPLATE_ID, Path::new("")) {
        copy_dir(&src, dest)?;
        return Ok(());
    }

    fs::create_dir_all(dest.join("src"))?;
    fs::create_dir_all(dest.join("scripts"))?;
    fs::create_dir_all(dest.join("src-tauri/src"))?;
    fs::create_dir_all(dest.join("src-tauri/capabilities"))?;
    fs::create_dir_all(dest.join("src-tauri/icons"))?;

    fs::write(
        dest.join("package.json"),
        r#"{
  "name": "source-tauri-app",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "tauri": "tauri",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build",
    "build:desktop": "./scripts/build-desktop.sh"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "@tauri-apps/api": "^2.1.1"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.1.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.4",
    "typescript": "^5.7.2",
    "vite": "^6.0.3"
  }
}
"#,
    )?;
    fs::write(
        dest.join("index.html"),
        "<!doctype html><html lang=\"en\"><head><meta charset=\"UTF-8\"/><meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\"/><title>Source Desktop App</title></head><body><div id=\"root\"></div><script type=\"module\" src=\"/src/main.tsx\"></script></body></html>\n",
    )?;
    fs::write(
        dest.join("vite.config.ts"),
        "import { defineConfig } from 'vite';\nimport react from '@vitejs/plugin-react';\nexport default defineConfig({ plugins: [react()], clearScreen: false, server: { port: 5173, strictPort: true } });\n",
    )?;
    fs::write(
        dest.join("tsconfig.json"),
        r#"{"compilerOptions":{"target":"ES2022","lib":["ES2022","DOM","DOM.Iterable"],"module":"ESNext","skipLibCheck":true,"moduleResolution":"bundler","jsx":"react-jsx","strict":true,"noEmit":true},"include":["src"]}"#,
    )?;
    fs::write(
        dest.join("src/main.tsx"),
        "import React from 'react';\nimport { createRoot } from 'react-dom/client';\nimport { App } from './App';\nimport './styles.css';\ncreateRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);\n",
    )?;
    fs::write(
        dest.join("src/App.tsx"),
        r##"import { useState } from "react";
export function App() {
  const [n, setN] = useState("0");
  return (
    <main style={{ minHeight: "100vh", display: "grid", placeItems: "center", fontFamily: "IBM Plex Sans, sans-serif", background: "#0c1017", color: "#f4efe6" }}>
      <section>
        <h1>Source Desktop</h1>
        <p>Offline calculator-class UI (Tauri v2)</p>
        <output style={{ display: "block", fontSize: 28 }}>{n}</output>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 4rem)", gap: 8 }}>
          {"7894561230".split("").map((d) => (
            <button key={d} type="button" onClick={() => setN((cur) => (cur === "0" ? d : cur + d))}>{d}</button>
          ))}
          <button type="button" onClick={() => setN("0")}>C</button>
        </div>
      </section>
    </main>
  );
}
"##,
    )?;
    fs::write(dest.join("src/styles.css"), "html,body,#root{height:100%;margin:0}\n")?;
    fs::write(
        dest.join("src-tauri/Cargo.toml"),
        r#"[package]
name = "source_tauri_app"
version = "0.1.0"
edition = "2021"
[lib]
name = "source_tauri_app_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
[[bin]]
name = "source-tauri-app"
path = "src/main.rs"
[build-dependencies]
tauri-build = { version = "2", features = [] }
[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"#,
    )?;
    fs::write(dest.join("src-tauri/build.rs"), "fn main() { tauri_build::build() }\n")?;
    fs::write(
        dest.join("src-tauri/src/main.rs"),
        "#![cfg_attr(not(debug_assertions), windows_subsystem = \"windows\")]\nfn main() { source_tauri_app_lib::run(); }\n",
    )?;
    fs::write(
        dest.join("src-tauri/src/lib.rs"),
        r#"#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({ "name": "source-tauri-app", "version": env!("CARGO_PKG_VERSION"), "runtime": "tauri-v2", "offline": true })
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running Source Tauri app");
}
"#,
    )?;
    fs::write(
        dest.join("src-tauri/tauri.conf.json"),
        r#"{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Source Desktop App",
  "version": "0.1.0",
  "identifier": "local.source.tauri.app",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://127.0.0.1:5173",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{ "label": "main", "title": "Source Desktop App", "width": 420, "height": 640 }],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.png"]
  }
}
"#,
    )?;
    fs::write(
        dest.join("src-tauri/capabilities/default.json"),
        r#"{"$schema":"https://schema.tauri.app/config/2/capability","identifier":"default","description":"Default","windows":["main"],"permissions":["core:default"]}"#,
    )?;
    // Minimal valid 1x1 PNG.
    let png_1x1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    for name in ["32x32.png", "128x128.png", "icon.png"] {
        fs::write(dest.join("src-tauri/icons").join(name), png_1x1)?;
    }
    fs::write(
        dest.join("scripts/build-desktop.sh"),
        r#"#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
INSTALL_DEPS=0
for arg in "$@"; do [[ "$arg" == "--install-deps" ]] && INSTALL_DEPS=1; done
if [[ "$(uname -s)" == "Linux" ]] && ! dpkg -s libwebkit2gtk-4.1-dev >/dev/null 2>&1; then
  DEPS="libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf pkg-config build-essential curl wget file libssl-dev libayatana-appindicator3-dev"
  if [[ "$INSTALL_DEPS" -eq 1 ]]; then
    sudo DEBIAN_FRONTEND=noninteractive apt-get update -y
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y $DEPS
  else
    echo "Missing WebKitGTK. Install: sudo apt-get install -y $DEPS" >&2
    echo "Or: ./scripts/build-desktop.sh --install-deps" >&2
  fi
fi
if command -v pnpm >/dev/null 2>&1; then pnpm install --frozen-lockfile=false; pnpm tauri build
else npm install; npx tauri build; fi
echo "Desktop build complete → src-tauri/target/release/bundle"
"#,
    )?;
    fs::write(
        dest.join("README.md"),
        "# Source Tauri v2 desktop app\n\nComplete desktop scaffold. Build with `./scripts/build-desktop.sh`.\nHTML-only is not a substitute for this target.\n",
    )?;
    Ok(())
}

fn seed_python_fastapi(dest: &Path) -> Result<()> {
    fs::write(
        dest.join("requirements.txt"),
        "fastapi>=0.115.0\nuvicorn[standard]>=0.32.0\nPySide6>=6.7.0\nhttpx>=0.27.0\n",
    )?;
    fs::write(
        dest.join("main.py"),
        r#"from fastapi import FastAPI

app = FastAPI(title="Source FastAPI")

@app.get("/health")
def health():
    return {"ok": True, "service": "source-fastapi"}
"#,
    )?;
    fs::write(
        dest.join("desktop.py"),
        r#"import sys
from PySide6.QtWidgets import QApplication, QLabel, QWidget, QVBoxLayout

def main():
    app = QApplication(sys.argv)
    w = QWidget()
    layout = QVBoxLayout(w)
    layout.addWidget(QLabel("Source PySide shell"))
    w.setWindowTitle("Source")
    w.resize(480, 320)
    w.show()
    sys.exit(app.exec())

if __name__ == "__main__":
    main()
"#,
    )?;
    fs::write(
        dest.join("README.md"),
        "# Python FastAPI / PySide\n\n```bash\npip install -r requirements.txt\nuvicorn main:app --reload\npython desktop.py\n```\n",
    )?;
    Ok(())
}

fn rewrite_package_name(dest: &Path, name: &str) -> Result<()> {
    let pkg = dest.join("package.json");
    if pkg.exists() {
        let mut v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&pkg)?)?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("name".into(), serde_json::Value::String(name.into()));
        }
        fs::write(&pkg, serde_json::to_string_pretty(&v)?)?;
    }
    let cargo = dest.join("src-tauri/Cargo.toml");
    if cargo.exists() {
        let text = fs::read_to_string(&cargo)?;
        let crate_name = name.replace('-', "_");
        let replaced = text
            .replacen(
                "name = \"source_tauri_app\"",
                &format!("name = \"{crate_name}\""),
                1,
            )
            .replacen(
                "name = \"source_tauri_app_lib\"",
                &format!("name = \"{crate_name}_lib\""),
                1,
            )
            .replacen(
                "name = \"source-tauri-app\"",
                &format!("name = \"{name}\""),
                1,
            );
        fs::write(&cargo, replaced)?;
        let main_rs = dest.join("src-tauri/src/main.rs");
        if main_rs.exists() {
            let main_txt = fs::read_to_string(&main_rs)?;
            fs::write(
                &main_rs,
                main_txt.replace("source_tauri_app_lib", &format!("{crate_name}_lib")),
            )?;
        }
    }
    let conf = dest.join("src-tauri/tauri.conf.json");
    if conf.exists() {
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&conf)?) {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("productName".into(), serde_json::Value::String(name.into()));
            }
            fs::write(&conf, serde_json::to_string_pretty(&v)?)?;
        }
    }
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_and_scaffolds_react_template() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = TemplateManager::new(
            dir.path().join("templates"),
            dir.path().join("vendor"),
            dir.path().join("bundled"),
        );
        let list = mgr.list().unwrap();
        assert!(list.iter().any(|t| t.id == "react-vite-tailwind"));
        let dest = dir.path().join("out-app");
        mgr.scaffold("react-vite-tailwind", &dest, Some("demo-app"))
            .unwrap();
        assert!(dest.join("package.json").exists());
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dest.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["name"], "demo-app");
        assert!(!dest.join("template.json").exists());
    }

    #[test]
    fn create_desktop_app_is_complete_tauri_project() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = TemplateManager::new(
            dir.path().join("templates"),
            dir.path().join("vendor"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates"),
        );
        let dest = dir.path().join("calc-app");
        let result = mgr.create_desktop_app(&dest, Some("calc-app")).unwrap();
        assert_eq!(result.template_id, DESKTOP_TEMPLATE_ID);
        assert!(dest.join("src-tauri/src/lib.rs").exists());
        assert!(dest.join("src-tauri/tauri.conf.json").exists());
        assert!(dest.join("src/App.tsx").exists());
        assert!(dest.join("scripts/build-desktop.sh").exists());
        assert!(!dest.join("template.json").exists());
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dest.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["name"], "calc-app");
    }

    #[test]
    fn refreshes_stale_desktop_template() {
        let dir = tempfile::tempdir().unwrap();
        let templates = dir.path().join("templates");
        let stale = templates.join(DESKTOP_TEMPLATE_ID);
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("template.json"), r#"{"id":"tauri-v2-react","version":1}"#).unwrap();
        fs::write(stale.join("package.json"), r#"{"name":"stale"}"#).unwrap();
        let mgr = TemplateManager::new(
            templates.clone(),
            dir.path().join("vendor"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates"),
        );
        mgr.ensure_seeded().unwrap();
        assert!(templates.join(DESKTOP_TEMPLATE_ID).join("src-tauri/src/lib.rs").exists());
        assert!(templates.join(DESKTOP_TEMPLATE_ID).join("scripts/build-desktop.sh").exists());
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(templates.join(DESKTOP_TEMPLATE_ID).join("template.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["version"], BUILTIN_TEMPLATE_VERSION);
    }
}
