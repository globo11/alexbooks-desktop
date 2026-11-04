//! AlexBooks for Windows — a thin shell around the office's AlexBooks server.
//!
//! The books, the files and the users all live on the server; this program is only the window. On every start it
//! reads the saved server address (`config.json` in the user's AlexBooks folder) and opens it in its own window —
//! no address bar, no tabs, the AlexBooks icon in the taskbar. The first time, with no address saved yet, it shows
//! the "Connect to your office" page (`ui/connect.html`) instead, and the address typed there is saved for good.
//!
//! Step 1 of the desktop shell: open, remember the address, remember the window size. The splash checks,
//! LAN discovery, certificate handling and self-update come in the next steps.

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// What the shell remembers between runs. Only the server address for now.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    server: String,
}

/// `%APPDATA%\com.alextaxservices.alexbooks\config.json` on Windows (the OS picks the folder).
fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

fn read_config(app: &AppHandle) -> Config {
    config_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let p = config_path(app).ok_or("no config folder")?;
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&p, serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Turn what the person typed into the address the window opens: `192.168.1.117` → `https://192.168.1.117:8443/`.
fn normalize(input: &str) -> Result<url::Url, String> {
    let mut s = input.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Err("Type the server's address.".into());
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("https://{s}");
    }
    let mut u = url::Url::parse(&s).map_err(|_| "That does not look like an address.".to_string())?;
    if u.port().is_none() && u.scheme() == "https" {
        u.set_port(Some(8443)).ok();
    }
    u.set_path("/");
    u.set_query(None);
    Ok(u)
}

#[tauri::command]
fn get_server(app: AppHandle) -> String {
    read_config(&app).server
}

/// Save the address and send the window to it.
#[tauri::command]
fn connect(app: AppHandle, server: String) -> Result<String, String> {
    let u = normalize(&server)?;
    write_config(&app, &Config { server: u.to_string() })?;
    let w = app.get_webview_window("main").ok_or("no window")?;
    w.navigate(u.clone()).map_err(|e| e.to_string())?;
    Ok(u.to_string())
}

/// Forget the saved address (Settings → "Connect to a different office", later), then show the connect page.
#[tauri::command]
fn forget_server(app: AppHandle) -> Result<(), String> {
    write_config(&app, &Config::default())?;
    let w = app.get_webview_window("main").ok_or("no window")?;
    w.navigate(connect_page_url()).map_err(|e| e.to_string())
}

/// The address of the shipped connect page: Tauri serves the app's own files on `http://tauri.localhost` on
/// Windows and `tauri://localhost` elsewhere.
fn connect_page_url() -> url::Url {
    #[cfg(windows)]
    let s = "http://tauri.localhost/connect.html";
    #[cfg(not(windows))]
    let s = "tauri://localhost/connect.html";
    url::Url::parse(s).expect("static url")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![get_server, connect, forget_server])
        .setup(|app| {
            let cfg = read_config(app.handle());
            // A saved address opens straight away; otherwise the connect page (a local file shipped in the app).
            let start = match normalize(&cfg.server) {
                Ok(u) if !cfg.server.is_empty() => WebviewUrl::External(u),
                _ => WebviewUrl::App("connect.html".into()),
            };
            WebviewWindowBuilder::new(app, "main", start)
                .title("AlexBooks")
                .inner_size(1280.0, 860.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AlexBooks");
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn addresses_are_completed() {
        assert_eq!(normalize("192.168.1.117").unwrap().as_str(), "https://192.168.1.117:8443/");
        assert_eq!(normalize(" https://192.168.1.117:8443/clients/3 ").unwrap().as_str(), "https://192.168.1.117:8443/");
        assert_eq!(normalize("http://office-pc:8080").unwrap().as_str(), "http://office-pc:8080/");
        assert!(normalize("").is_err());
    }
}
