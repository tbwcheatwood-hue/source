//! Tauri v2 host for the offline React UI. No stubs.

use tauri::Manager;

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "name": "source-tauri-app",
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": "tauri-v2",
        "offline": true,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Source Desktop App");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Source Tauri app");
}
