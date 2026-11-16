//! Offline app factory: templates under `~/.source/templates/` + vendor cache helpers.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const BUILTIN_TEMPLATES: &[&str] = &[
    "react-vite-tailwind",
    "tauri-v2-react",
    "python-fastapi-pyside",
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
            if dest.exists() {
                continue;
            }
            let src = self.bundled_templates_dir.join(id);
            if src.exists() {
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
        Ok(dest.to_path_buf())
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

fn write_manifest(id: &str, dest: &Path) -> Result<()> {
    let (title, description) = match id {
        "react-vite-tailwind" => (
            "React + Vite + Tailwind",
            "SPA scaffold with Vite, React, TypeScript, and Tailwind CSS",
        ),
        "tauri-v2-react" => (
            "Tauri v2 + Rust + React",
            "Desktop app scaffold with Tauri v2, Rust backend, React UI",
        ),
        "python-fastapi-pyside" => (
            "Python FastAPI / PySide",
            "API + optional desktop UI with FastAPI and PySide6",
        ),
        _ => (id, "Custom template"),
    };
    let manifest = serde_json::json!({
        "id": id,
        "title": title,
        "description": description,
    });
    fs::write(dest.join("template.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(())
}

fn seed_builtin_template(id: &str, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    match id {
        "react-vite-tailwind" => {
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
        }
        "tauri-v2-react" => {
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
    "tauri": "tauri"
  },
  "dependencies": { "react": "^19.0.0", "react-dom": "^19.0.0", "@tauri-apps/api": "^2.0.0" },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@vitejs/plugin-react": "^4.3.4",
    "typescript": "^5.7.2",
    "vite": "^6.0.3"
  }
}
"#,
            )?;
            fs::create_dir_all(dest.join("src"))?;
            fs::create_dir_all(dest.join("src-tauri/src"))?;
            fs::write(
                dest.join("index.html"),
                "<!doctype html><html><body><div id='root'></div><script type='module' src='/src/main.tsx'></script></body></html>\n",
            )?;
            fs::write(
                dest.join("src/main.tsx"),
                "import { createRoot } from 'react-dom/client';\ncreateRoot(document.getElementById('root')!).render(<h1>Source Tauri App</h1>);\n",
            )?;
            fs::write(
                dest.join("src-tauri/Cargo.toml"),
                "[package]\nname = \"source_tauri_app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ntauri = { version = \"2\", features = [] }\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
            )?;
            fs::write(
                dest.join("src-tauri/src/main.rs"),
                "#![cfg_attr(not(debug_assertions), windows_subsystem = \"windows\")]\nfn main() { tauri::Builder::default().run(tauri::generate_context!()).expect(\"error\"); }\n",
            )?;
            fs::write(
                dest.join("src-tauri/tauri.conf.json"),
                r#"{"productName":"Source Tauri App","version":"0.1.0","identifier":"local.source.tauri","build":{"beforeDevCommand":"pnpm dev","devUrl":"http://127.0.0.1:5173","beforeBuildCommand":"pnpm build","frontendDist":"../dist"},"app":{"windows":[{"title":"Source","width":1024,"height":768}]}}"#,
            )?;
        }
        "python-fastapi-pyside" => {
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
        }
        _ => return Err(anyhow!("no builtin seed for {id}")),
    }
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
        let replaced = text.replacen(
            "name = \"source_tauri_app\"",
            &format!("name = \"{}\"", name.replace('-', "_")),
            1,
        );
        fs::write(&cargo, replaced)?;
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
}
