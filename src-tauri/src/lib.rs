//! AlexBooks for Windows — a thin shell around the office's AlexBooks server.
//!
//! The books, the files and the users all live on the server; this program is only the window. On every start it
//! reads the saved server address (`config.json` in the user's AlexBooks folder) and opens it in its own window —
//! no address bar, no tabs, the AlexBooks icon in the taskbar. The first time, with no address saved yet, it shows
//! the "Connect to your office" page (`ui/connect.html`) instead, and the address typed there is saved for good.
//!
//! Step 2 (0.2.0): the first run asks "Just this PC" or "Network". Just this PC looks for AlexBooks on localhost;
//! Network asks the LAN ("ALEXBOOKS?" on UDP 8443 — servers from 1.25.3 answer with their office name), falls back
//! to knocking on port 8443 across this PC's subnet for older servers, and always allows typing an address.
//! Certificate handling and self-update are the next steps.

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

/// A server found on the network (or on this PC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Found {
    pub office: String,
    pub url: String,
    pub version: String,
    /// "beacon" = the server answered by name · "scan" = port 8443 is open there, name unknown · "local" = this PC
    pub how: String,
}

/// Ask the LAN "ALEXBOOKS?" (UDP broadcast, port 8443) and collect the servers that answer within ~1.5 s.
fn ask_network(port: u16) -> Vec<Found> {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::time::{Duration, Instant};
    let mut out: Vec<Found> = Vec::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return out };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let targets = [SocketAddrV4::new(Ipv4Addr::BROADCAST, port)];
    for t in &targets {
        for _ in 0..2 {
            let _ = sock.send_to(b"ALEXBOOKS?", t);
        }
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) {
                    if v.get("app").and_then(|a| a.as_str()) == Some("alexbooks") {
                        let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                        if !url.is_empty() && !out.iter().any(|f| f.url == url) {
                            out.push(Found {
                                office: v.get("office").and_then(|o| o.as_str()).unwrap_or("AlexBooks").to_string(),
                                url,
                                version: v.get("version").and_then(|o| o.as_str()).unwrap_or("").to_string(),
                                how: "beacon".into(),
                            });
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }
    out
}

/// This PC's address on the LAN (the one it would use to reach the internet), to know which /24 to scan.
fn own_lan_ip() -> Option<std::net::Ipv4Addr> {
    use std::net::{IpAddr, UdpSocket};
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:9").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

/// Older servers do not answer the beacon: knock on port 8443 of every address in this PC's /24 (quick TCP
/// connect, 254 in parallel, ~0.5 s) and list the ones that are open.
fn scan_subnet(port: u16) -> Vec<Found> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let Some(me) = own_lan_ip() else { return Vec::new() };
    let o = me.octets();
    let handles: Vec<_> = (1u8..=254)
        .map(|h| {
            std::thread::spawn(move || {
                let ip = std::net::Ipv4Addr::new(o[0], o[1], o[2], h);
                let addr = SocketAddr::from((ip, port));
                TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok().map(|_| ip)
            })
        })
        .collect();
    let mut out = Vec::new();
    for h in handles {
        if let Ok(Some(ip)) = h.join() {
            out.push(Found { office: format!("AlexBooks server at {ip}"), url: format!("https://{ip}:{port}/"), version: String::new(), how: "scan".into() });
        }
    }
    out
}

/// Is AlexBooks running on THIS PC? (port 8443 on localhost)
fn local_server(port: u16) -> Option<Found> {
    use std::net::TcpStream;
    use std::time::Duration;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    let mut f = Found { office: "This PC".into(), url: format!("https://localhost:{port}/"), version: String::new(), how: "local".into() };
    // ask it by name too, so the list shows the office
    if let Ok(sock) = std::net::UdpSocket::bind("127.0.0.1:0") {
        let _ = sock.set_read_timeout(Some(Duration::from_millis(400)));
        let _ = sock.send_to(b"ALEXBOOKS?", ("127.0.0.1", port));
        let mut buf = [0u8; 2048];
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) {
                if let Some(o) = v.get("office").and_then(|o| o.as_str()) { f.office = o.to_string(); }
                if let Some(ver) = v.get("version").and_then(|o| o.as_str()) { f.version = ver.to_string(); }
            }
        }
    }
    Some(f)
}

/// "Network": the servers on the office LAN — by name when they answer, by open port otherwise.
#[tauri::command]
async fn discover() -> Vec<Found> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut found = ask_network(8443);
        if found.is_empty() {
            found = scan_subnet(8443);
        }
        found
    })
    .await
    .unwrap_or_default()
}

/// "Just this PC": is the server here?
#[tauri::command]
async fn check_local() -> Option<Found> {
    tauri::async_runtime::spawn_blocking(|| local_server(8443)).await.unwrap_or(None)
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
        .invoke_handler(tauri::generate_handler![get_server, connect, forget_server, discover, check_local])
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
