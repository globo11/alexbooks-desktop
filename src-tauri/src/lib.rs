//! AlexBooks for Windows — a thin shell around the office's AlexBooks server.
//!
//! The books, the files and the users all live on the server; this program is only the window. Every start opens
//! the shipped start page (`ui/index.html`), which does three quick checks and then shows the server:
//!
//!   1. is the saved server reachable?  (`probe`) — if not, a clear "can't reach" page with Try again;
//!   2. does this PC trust its certificate?  (`probe` → "untrusted", then `trust_ca` with the person's OK);
//!   3. is there a newer AlexBooks for Windows on *that server*?  (`check_update` / `install_update`) — the server
//!      hands out the program it fetched from GitHub, so office PCs never need internet or a token.
//!
//! With no server saved yet it asks "Just this PC" or "Network" (0.2.0): Network asks the LAN ("ALEXBOOKS?" on
//! UDP 8443), falls back to knocking on port 8443 across the subnet, and always allows typing an address.
//! A small native menu (Reload · Connect to a different office · Check for updates · About · Exit) is the only
//! chrome around the server's pages.

use std::{fs, path::PathBuf, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

const PORT: u16 = 8443;

/// What the shell remembers between runs. Only the server address.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    server: String,
}

/// An update found by `check_update`, kept until `install_update` is called.
struct Pending(Mutex<Option<Update>>);

/// `%APPDATA%\com.alextaxservices.alexbooks\` on Windows (the OS picks the folder).
fn config_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    config_dir(app).map(|d| d.join("config.json"))
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

/// The office CA this PC was told to trust (saved by `trust_ca`), as PEM.
fn saved_ca(app: &AppHandle) -> Option<String> {
    config_dir(app).and_then(|d| fs::read_to_string(d.join("ca.pem")).ok())
}

/// Turn what the person typed into the address the window opens: `192.168.1.50` → `https://192.168.1.50:8443/`.
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
        u.set_port(Some(PORT)).ok();
    }
    u.set_path("/");
    u.set_query(None);
    u.set_fragment(None);
    Ok(u)
}

#[tauri::command]
fn get_server(app: AppHandle) -> String {
    read_config(&app).server
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ---------------------------------------------------------------------------------------------------------------
// Finding servers (0.2.0)

/// A server found on the network (or on this PC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Found {
    pub office: String,
    pub url: String,
    pub version: String,
    /// "beacon" = the server answered by name · "scan" = port 8443 is open there, name unknown · "local" = this PC
    pub how: String,
}

fn parse_beacon(data: &[u8]) -> Option<(String, String, String)> {
    let v = serde_json::from_slice::<serde_json::Value>(data).ok()?;
    if v.get("app").and_then(|a| a.as_str()) != Some("alexbooks") {
        return None;
    }
    let s = |k: &str| v.get(k).and_then(|o| o.as_str()).unwrap_or("").to_string();
    Some((s("office"), s("url"), s("version")))
}

/// Ask the LAN "ALEXBOOKS?" (UDP broadcast, port 8443) and collect the servers that answer within ~1.5 s.
fn ask_network(port: u16) -> Vec<Found> {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::time::Instant;
    let mut out: Vec<Found> = Vec::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return out };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    for _ in 0..2 {
        let _ = sock.send_to(b"ALEXBOOKS?", SocketAddrV4::new(Ipv4Addr::BROADCAST, port));
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut buf = [0u8; 16384]; // the answer carries the office certificate since 1.25.7
    while Instant::now() < deadline {
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if let Some((office, url, version)) = parse_beacon(&buf[..n]) {
                if !url.is_empty() && !out.iter().any(|f| f.url == url) {
                    let office = if office.is_empty() { "AlexBooks".into() } else { office };
                    out.push(Found { office, url, version, how: "beacon".into() });
                }
            }
        }
    }
    out
}

/// Ask ONE server by name (unicast beacon): its office name and version, if it answers.
fn ask_one(host: &str, port: u16) -> Option<(String, String)> {
    use std::net::{ToSocketAddrs, UdpSocket};
    let addr = (host, port).to_socket_addrs().ok()?.find(|a| a.is_ipv4())?;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(600))).ok()?;
    sock.send_to(b"ALEXBOOKS?", addr).ok()?;
    let mut buf = [0u8; 16384];
    let (n, _) = sock.recv_from(&mut buf).ok()?;
    parse_beacon(&buf[..n]).map(|(o, _, v)| (o, v))
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
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    let mut f = Found { office: "This PC".into(), url: format!("https://localhost:{port}/"), version: String::new(), how: "local".into() };
    if let Some((office, version)) = ask_one("127.0.0.1", port) {
        if !office.is_empty() {
            f.office = office;
        }
        f.version = version;
    }
    Some(f)
}

/// "Network": the servers on the office LAN — by name when they answer, by open port otherwise.
#[tauri::command]
async fn discover() -> Vec<Found> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut found = ask_network(PORT);
        if found.is_empty() {
            found = scan_subnet(PORT);
        }
        found
    })
    .await
    .unwrap_or_default()
}

/// "Just this PC": is the server here?
#[tauri::command]
async fn check_local() -> Option<Found> {
    tauri::async_runtime::spawn_blocking(|| local_server(PORT)).await.unwrap_or(None)
}

// ---------------------------------------------------------------------------------------------------------------
// Reaching the server and trusting its certificate (0.3.0)

/// What `probe` found out about a server address.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Probe {
    /// "ok" · "untrusted" (answers, but this PC does not trust its certificate yet) · "not_alexbooks" ·
    /// "unreachable"
    pub status: String,
    pub office: String,
    pub version: String,
    pub message: String,
    pub url: String,
}

fn http_client(trust_any: bool, ca_pem: Option<&str>) -> reqwest::blocking::Client {
    let mut b = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("AlexBooks-for-Windows");
    if trust_any {
        b = b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    }
    if let Some(pem) = ca_pem {
        if let Ok(c) = reqwest::Certificate::from_pem(pem.as_bytes()) {
            b = b.add_root_certificate(c);
        }
    }
    b.build().expect("http client")
}

/// Turn a reqwest error into one plain sentence.
fn friendly(e: reqwest::Error, host: &str) -> String {
    if e.is_timeout() {
        format!("No answer from {host} — it may be off, asleep, or on another network.")
    } else if e.is_connect() {
        format!("Could not connect to {host}. Is the server PC on and on this network?")
    } else {
        format!("{host}: {}", e.without_url())
    }
}

/// `/healthz` answers `{"ok":true,"app":"AlexBooks","version":"1.25.7"}`; office name is asked by beacon.
fn read_health(r: reqwest::blocking::Response, u: &url::Url) -> Probe {
    let host = u.host_str().unwrap_or("").to_string();
    let v: serde_json::Value = match r.json() {
        Ok(v) => v,
        Err(_) => return Probe { status: "not_alexbooks".into(), message: format!("{host} answers, but it is not an AlexBooks server."), url: u.to_string(), ..Default::default() },
    };
    if v.get("app").and_then(|a| a.as_str()) != Some("AlexBooks") {
        return Probe { status: "not_alexbooks".into(), message: format!("{host} answers, but it is not an AlexBooks server."), url: u.to_string(), ..Default::default() };
    }
    let version = v.get("version").and_then(|a| a.as_str()).unwrap_or("").to_string();
    let mut office = v.get("office").and_then(|a| a.as_str()).unwrap_or("").to_string();
    if office.is_empty() {
        if let Some((o, _)) = ask_one(&host, u.port().unwrap_or(PORT)) {
            office = o;
        }
    }
    if office.is_empty() {
        office = host.clone();
    }
    Probe { status: "ok".into(), office, version, message: String::new(), url: u.to_string() }
}

fn probe_blocking(app: &AppHandle, server: &str) -> Probe {
    let u = match normalize(server) {
        Ok(u) => u,
        Err(m) => return Probe { status: "unreachable".into(), message: m, ..Default::default() },
    };
    let host = u.host_str().unwrap_or("").to_string();
    let hz = u.join("healthz").expect("url");
    let ca = saved_ca(app);
    // 1. the strict way: the certificate must be trusted by Windows (or be the CA we saved)
    match http_client(false, ca.as_deref()).get(hz.clone()).send() {
        Ok(r) => return read_health(r, &u),
        Err(e) if e.is_timeout() => return Probe { status: "unreachable".into(), message: friendly(e, &host), url: u.to_string(), ..Default::default() },
        Err(_) => {}
    }
    // 2. does it answer at all? (certificate ignored) → then only the trust is missing
    match http_client(true, None).get(hz).send() {
        Ok(r) => {
            let mut p = read_health(r, &u);
            if p.status == "ok" {
                p.status = "untrusted".into();
                p.message = format!("This PC does not trust {host}'s certificate yet.");
            }
            p
        }
        Err(e) => Probe { status: "unreachable".into(), message: friendly(e, &host), url: u.to_string(), ..Default::default() },
    }
}

/// Can this PC reach that server, and does it trust it?
#[tauri::command]
async fn probe(app: AppHandle, server: String) -> Probe {
    tauri::async_runtime::spawn_blocking(move || probe_blocking(&app, &server)).await.unwrap_or_default()
}

/// Fetch the office CA from the server (`/ca.crt`, public) and ask Windows to trust it for the current user.
/// Windows shows its own "Do you want to install this certificate?" prompt — that is the person's consent.
fn trust_ca_blocking(app: &AppHandle, server: &str) -> Result<String, String> {
    let u = normalize(server)?;
    let host = u.host_str().unwrap_or("").to_string();
    let pem = http_client(true, None)
        .get(u.join("ca.crt").expect("url"))
        .send()
        .map_err(|e| friendly(e, &host))?
        .text()
        .map_err(|e| e.to_string())?;
    if !pem.contains("BEGIN CERTIFICATE") {
        return Err(format!("{host} did not hand out its certificate. It may be running an AlexBooks older than 1.25.7 — update the server first."));
    }
    let dir = config_dir(app).ok_or("no config folder")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let crt = dir.join("alexbooks-ca.crt");
    fs::write(&crt, &pem).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("certutil")
            .args(["-user", "-addstore", "Root"])
            .arg(&crt)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .map_err(|e| format!("Could not run certutil: {e}"))?;
        if !out.status.success() {
            return Err("Windows did not add the certificate (the prompt was closed or answered No). Click Trust to try again.".into());
        }
    }
    fs::write(dir.join("ca.pem"), &pem).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    {
        return Err("Trusting the certificate automatically only works on Windows; the certificate was saved for the updater.".into());
    }
    #[allow(unreachable_code)]
    Ok("trusted".into())
}

#[tauri::command]
async fn trust_ca(app: AppHandle, server: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || trust_ca_blocking(&app, &server)).await.map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------------------------------------------
// Opening the server

/// Save the address and send the window to it (the start page calls this after its checks).
#[tauri::command]
fn connect(app: AppHandle, server: String, office: String) -> Result<String, String> {
    let u = normalize(&server)?;
    write_config(&app, &Config { server: u.to_string() })?;
    let w = app.get_webview_window("main").ok_or("no window")?;
    let title = if office.trim().is_empty() { "AlexBooks".to_string() } else { format!("AlexBooks — {}", office.trim()) };
    let _ = w.set_title(&title);
    w.navigate(u.clone()).map_err(|e| e.to_string())?;
    Ok(u.to_string())
}

/// Forget the saved address and show the start page's choice again.
#[tauri::command]
fn forget_server(app: AppHandle) -> Result<(), String> {
    write_config(&app, &Config::default())?;
    show_start(&app, "switch")
}

/// The address of the shipped start page: Tauri serves the app's own files on `http://tauri.localhost` on
/// Windows and `tauri://localhost` elsewhere.
fn start_page_url(mode: &str) -> url::Url {
    #[cfg(windows)]
    let s = "http://tauri.localhost/index.html";
    #[cfg(not(windows))]
    let s = "tauri://localhost/index.html";
    let mut u = url::Url::parse(s).expect("static url");
    if !mode.is_empty() {
        u.set_fragment(Some(mode));
    }
    u
}

fn show_start(app: &AppHandle, mode: &str) -> Result<(), String> {
    let w = app.get_webview_window("main").ok_or("no window")?;
    let _ = w.set_title("AlexBooks");
    w.navigate(start_page_url(mode)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------------------------
// Self-update from the office server (0.3.0)

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: String,
}

/// Ask the office server for a newer AlexBooks for Windows (`/desktop/latest.json`, signed with the project's key).
#[tauri::command]
async fn check_update(app: AppHandle, server: String, pending: State<'_, Pending>) -> Result<Option<UpdateInfo>, String> {
    let u = normalize(&server)?;
    let endpoint = u.join("desktop/latest.json").map_err(|e| e.to_string())?;
    let ca = saved_ca(&app).and_then(|pem| reqwest::Certificate::from_pem(pem.as_bytes()).ok());
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(10))
        .configure_client(move |b| match &ca {
            Some(c) => b.add_root_certificate(c.clone()),
            None => b,
        })
        .build()
        .map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(upd)) => {
            let info = UpdateInfo { version: upd.version.clone(), current: upd.current_version.clone(), notes: upd.body.clone().unwrap_or_default() };
            *pending.0.lock().map_err(|e| e.to_string())? = Some(upd);
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(tauri_plugin_updater::Error::ReleaseNotFound) => Ok(None),
        Err(e) => Err(format!("{e}")),
    }
}

/// Download and install the update found by `check_update`. On Windows the installer restarts the program.
#[tauri::command]
async fn install_update(app: AppHandle, pending: State<'_, Pending>) -> Result<(), String> {
    let upd = pending.0.lock().map_err(|e| e.to_string())?.take().ok_or("Check for updates first.")?;
    let mut got: u64 = 0;
    let a = app.clone();
    upd.download_and_install(
        move |chunk, total| {
            got += chunk as u64;
            let _ = a.emit("update-progress", serde_json::json!({ "got": got, "total": total }));
        },
        || {},
    )
    .await
    .map_err(|e| format!("{e}"))?;
    app.restart();
}

// ---------------------------------------------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Pending(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            get_server, app_version, connect, forget_server, discover, check_local, probe, trust_ca, check_update, install_update
        ])
        .setup(|app| {
            let menu = Menu::with_items(
                app,
                &[&Submenu::with_items(
                    app,
                    "AlexBooks",
                    true,
                    &[
                        &MenuItem::with_id(app, "reload", "Reload", true, Some("F5"))?,
                        &PredefinedMenuItem::separator(app)?,
                        &MenuItem::with_id(app, "switch", "Connect to a different office…", true, None::<&str>)?,
                        &MenuItem::with_id(app, "update", "Check for updates…", true, None::<&str>)?,
                        &MenuItem::with_id(app, "about", "About AlexBooks", true, None::<&str>)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::quit(app, Some("Exit"))?,
                    ],
                )?],
            )?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, ev| {
                let _ = match ev.id().as_ref() {
                    "reload" => app.get_webview_window("main").map(|w| w.eval("location.reload()")).unwrap_or(Ok(())).map_err(|e| e.to_string()),
                    "switch" => show_start(app, "switch"),
                    "update" => show_start(app, "update"),
                    "about" => show_start(app, "about"),
                    _ => Ok(()),
                };
            });
            // Always start on the shipped page: it checks the saved server, the certificate and updates, then opens it.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
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
    use super::{normalize, parse_beacon};

    #[test]
    fn addresses_are_completed() {
        assert_eq!(normalize("192.168.1.50").unwrap().as_str(), "https://192.168.1.50:8443/");
        assert_eq!(normalize(" https://192.168.1.50:8443/clients/3 ").unwrap().as_str(), "https://192.168.1.50:8443/");
        assert_eq!(normalize("http://office-pc:8080").unwrap().as_str(), "http://office-pc:8080/");
        assert!(normalize("").is_err());
    }

    #[test]
    fn beacon_answers_are_read_and_noise_ignored() {
        let (o, u, v) = parse_beacon(br#"{"app":"alexbooks","office":"Main Office","url":"https://10.0.0.5:8443/","version":"1.25.7","ca":"-----BEGIN CERTIFICATE-----\n..."}"#).unwrap();
        assert_eq!((o.as_str(), u.as_str(), v.as_str()), ("Main Office", "https://10.0.0.5:8443/", "1.25.7"));
        assert!(parse_beacon(b"{\"app\":\"other\"}").is_none());
        assert!(parse_beacon(b"garbage").is_none());
    }
}
