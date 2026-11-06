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

use std::{fs, path::{Path, PathBuf}, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
mod abp;

use tauri::webview::{DownloadEvent, NewWindowResponse};
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

/// Tell the server this PC exists (name + launcher version), so Settings → Updates lists it (0.3.1). Best effort.
#[tauri::command]
async fn hello(app: AppHandle, server: String) -> bool {
    let version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(u) = normalize(&server) else { return false };
        let pc = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
        let Ok(mut url) = u.join("desktop/hello") else { return false };
        url.query_pairs_mut().append_pair("pc", &pc).append_pair("v", &version);
        let ca = saved_ca(&app);
        http_client(false, ca.as_deref()).get(url).send().map(|r| r.status().is_success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
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
        let out = quiet(std::process::Command::new("certutil"))
            .args(["-user", "-addstore", "Root"])
            .arg(&crt)
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
// Downloads, new windows and outside links (0.4.0)
//
// A browser has a download shelf, tabs and a back button; a WebView2 window has none of them. Without the hooks
// below a PDF or a package "downloads" into the Downloads folder in silence (a click that seems to do nothing),
// a link with target=_blank opens nowhere, and a link to github.com would replace the office in this window.

/// Where downloads go: the user's Downloads folder, falling back to the desktop, then the temp folder.
fn downloads_dir(app: &AppHandle) -> PathBuf {
    app.path().download_dir().or_else(|_| app.path().desktop_dir()).unwrap_or_else(|_| std::env::temp_dir())
}

/// `report.pdf` → `report (2).pdf` when the name is taken, like a browser does.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..1000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|q| !q.exists()).unwrap_or(p)
}

/// Show the file: a PDF opens in the PC's PDF viewer; anything else (a package zip, a spreadsheet, a backup) is
/// shown selected in Explorer so the person sees where it landed. A folder is simply opened.
fn show_download(path: &Path) {
    let is_pdf = path.extension().map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false);
    #[cfg(windows)]
    {
        let r = if path.is_dir() {
            std::process::Command::new("explorer").arg(path).spawn()
        } else if is_pdf {
            quiet(std::process::Command::new("cmd")).args(["/C", "start", "", &path.to_string_lossy()]).spawn()
        } else {
            std::process::Command::new("explorer").arg(format!("/select,{}", path.to_string_lossy())).spawn()
        };
        let _ = r;
    }
    #[cfg(not(windows))]
    {
        let target = if is_pdf || path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

/// Open an address in the PC's own browser (GitHub, a bank's site, a mailto: link) — never inside this window.
fn open_outside(url: &url::Url) {
    #[cfg(windows)]
    let _ = quiet(std::process::Command::new("cmd")).args(["/C", "start", "", url.as_str()]).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url.as_str()).spawn();
}

/// Is this address the office server (or the shipped start page)? Everything else is "outside".
fn is_ours(app: &AppHandle, url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "tauri.localhost" || url.scheme() == "tauri" {
        return true;
    }
    match normalize(&read_config(app).server) {
        Ok(server) => server.host_str().map(|h| h.eq_ignore_ascii_case(&host)).unwrap_or(false),
        Err(_) => false,
    }
}

/// The one server address this window acts on instead of showing (1.2.0): `/desktop/apply-update`, the link behind
/// "Restart AlexBooks to update now" on Settings → Updates.
fn is_apply_update(url: &url::Url) -> bool {
    url.path().trim_end_matches('/') == "/desktop/apply-update"
}

/// A small message inside the page: the app shows it in its own style when it knows how (1.27.4), else a plain one.
fn page_notice(app: &AppHandle, text: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let js = format!(
            "(function(t){{ if (window.alexbooksNotice) {{ window.alexbooksNotice(t); return; }} \
             var d=document.getElementById('ab-notice'); if (!d) {{ d=document.createElement('div'); d.id='ab-notice'; d.setAttribute('role','status'); \
             d.style.cssText='position:fixed;right:24px;bottom:24px;z-index:9999;background:#1f2933;color:#fff;padding:10px 16px;border-radius:12px;font:600 14px system-ui;box-shadow:0 8px 24px rgba(0,0,0,.25)'; \
             document.body.appendChild(d); }} d.textContent=t; d.hidden=false; clearTimeout(d._t); d._t=setTimeout(function(){{ d.hidden=true; }}, 6000); }})({});",
            serde_json::to_string(text).unwrap_or_default()
        );
        let _ = w.eval(&js);
    }
}

/// The name a download will be saved under: what the server said (``Content-Disposition`` becomes the suggested
/// path), else the last part of the address.
fn download_name(suggested: &Path, url: &url::Url) -> String {
    let from_suggested = suggested.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if !from_suggested.is_empty() {
        return from_suggested;
    }
    url.path_segments().and_then(|mut s| s.next_back()).filter(|n| !n.is_empty()).unwrap_or("download").to_string()
}


/// A console program (tasklist, taskkill, certutil, cmd /C start) run from a windowed app pops a black console
/// window for a moment unless told not to (1.0.2): this is that telling. Returns the command for chaining.
fn quiet(mut cmd: std::process::Command) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);                       // CREATE_NO_WINDOW
    }
    cmd
}

// ---------------------------------------------------------------------------------------------------------------
// One AlexBooks window per PC (1.0.1)
//
// A second double-click used to open a second window. Now the first window keeps a small lock file with its process
// id and a heartbeat it refreshes every two seconds; a second start reads it:
//   - heartbeat fresh → the first window is alive: it is asked to come to the front (a "wake" file it watches),
//     shows a note, and the second start quits;
//   - heartbeat stale → the first window is stuck (hung, or gone without cleaning up): the person is asked whether
//     to close it, it is ended, and this start carries on as the one window.
// Nothing is lost either way: the books live on the server, and a form in progress is kept by the page's own
// draft autosave and comes back when the page is opened again.

const LOCK_NAME: &str = "alexbooks-window.lock";
const WAKE_NAME: &str = "alexbooks-window.wake";
const STALE_AFTER: u64 = 8;                                    // seconds without a heartbeat = stuck

fn lock_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("AlexBooks")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default)]
struct WindowLock {
    pid: u32,
    alive_at: u64,
}

fn read_lock() -> Option<WindowLock> {
    serde_json::from_str(&fs::read_to_string(lock_dir().join(LOCK_NAME)).ok()?).ok()
}

fn write_lock() {
    let d = lock_dir();
    let _ = fs::create_dir_all(&d);
    let _ = fs::write(d.join(LOCK_NAME), serde_json::to_string(&WindowLock { pid: std::process::id(), alive_at: now_secs() }).unwrap_or_default());
}

fn release_lock() {
    if read_lock().map(|l| l.pid == std::process::id()).unwrap_or(false) {
        let _ = fs::remove_file(lock_dir().join(LOCK_NAME));
    }
}

/// End a stuck AlexBooks window by its process id.
fn end_process(pid: u32) {
    #[cfg(windows)]
    let _ = quiet(std::process::Command::new("taskkill")).args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output();
}

/// Is a process with this id running? (A fresh heartbeat from a process that is gone — closed a second ago,
/// or replaced by an update — must not keep a new start out.)
fn pid_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = quiet(std::process::Command::new("tasklist")).args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]).output();
        return out.map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\""))).unwrap_or(true);
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Decide, before any window exists, whether this start may go on. Returns false when another window took over.
fn claim_single_window() -> bool {
    let Some(mut lock) = read_lock() else { write_lock(); return true; };
    if lock.pid == std::process::id() {
        return true;
    }
    if !pid_running(lock.pid) {
        write_lock();                                          // gone without cleaning up: take over
        return true;
    }
    if now_secs().saturating_sub(lock.alive_at) <= STALE_AFTER {
        // alive: wake it at once, then step aside — unless it is on its way out (an update restarts the program),
        // in which case this start becomes the window
        let _ = fs::write(lock_dir().join(WAKE_NAME), package_from_args(std::env::args()).unwrap_or_else(|| "wake".into()));
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(500));
            match read_lock() {
                None => { write_lock(); return true; }
                Some(l) if l.pid != lock.pid => { lock = l; }
                Some(_) if !pid_running(lock.pid) => { write_lock(); return true; }
                Some(_) => {}
            }
        }
        return false;
    }
    // stuck: ask, then end it
    let answer = rfd::MessageDialog::new()
        .set_title("AlexBooks is already open")
        .set_description("AlexBooks is already open on this PC but is not responding (it may be hidden or frozen).\n\nClose it and open a fresh window? Nothing is lost: the books are on the office server, and a form you were in the middle of comes back from its draft.")
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show();
    if answer != rfd::MessageDialogResult::Yes {
        return false;
    }
    end_process(lock.pid);
    std::thread::sleep(Duration::from_millis(600));
    write_lock();
    true
}

/// Bring the window to the front: un-minimize, show, focus — also when it was hidden off every screen.
fn surface(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if w.is_visible().unwrap_or(false) == false {
            let _ = w.center();
            let _ = w.show();
        }
    }
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    release_lock();
    app.restart();
}

// ---------------------------------------------------------------------------------------------------------------
// Set up a new office from an emailed package (1.1.0)
//
// The main office emails `AlexBooks-<office>.abp` (the install package sealed with a passphrase so mail scanners let
// it through). On the PC that will be the office's server: pick the file, type the passphrase, and the installer
// inside runs — the same `Install AlexBooks.cmd` as a hand-delivered package. When it finishes, the local server
// appears on the "Just this PC" list and the window opens it.

/// 1.3.0: double-clicking the .abp attachment opens AlexBooks for Windows with the file (the installer registers the
/// file type). The path arrives as a command-line argument — here, or through the wake file when a window is already
/// open — and waits in this state until the start page asks for it.
pub struct PendingPackage(pub Mutex<Option<String>>);

fn package_from_args<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    args.into_iter().skip(1).find(|a| a.to_ascii_lowercase().ends_with(".abp") && Path::new(a).is_file())
}

#[tauri::command]
fn pending_package(pending: State<'_, PendingPackage>) -> Option<String> {
    pending.0.lock().ok().and_then(|mut p| p.take())
}

#[tauri::command]
async fn pick_package() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new().add_filter("AlexBooks package", &["abp"]).set_title("Pick the package from the email").pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }).await.ok().flatten()
}

#[tauri::command]
async fn install_package(app: AppHandle, path: String, passphrase: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let data = fs::read(&path).map_err(|e| format!("Could not read the file: {e}"))?;
        let zip_bytes = abp::open(&data, &passphrase)?;
        let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "package".into());
        let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("packages").join(stem);
        let installer = abp::unpack(&zip_bytes, &dir)?;
        // the .cmd asks Windows for elevation itself and shows its own progress window
        #[cfg(windows)]
        quiet(std::process::Command::new("cmd")).args(["/C", "start", "", &installer.to_string_lossy()]).spawn().map_err(|e| format!("Could not start the installer: {e}"))?;
        #[cfg(not(windows))]
        let _ = installer;
        Ok(dir.to_string_lossy().to_string())
    }).await.map_err(|e| e.to_string())?
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
    if !claim_single_window() {
        return;                                                // another AlexBooks window is up and was told to come to the front
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Pending(Mutex::new(None)))
        .manage(PendingPackage(Mutex::new(package_from_args(std::env::args()))))
        .invoke_handler(tauri::generate_handler![
            get_server, app_version, connect, forget_server, discover, check_local, probe, hello, trust_ca, check_update, install_update, restart_app, pick_package, install_package, pending_package
        ])
        .setup(|app| {
            let menu = Menu::with_items(
                app,
                &[&Submenu::with_items(
                    app,
                    "AlexBooks",
                    true,
                    &[
                        &MenuItem::with_id(app, "back", "Back", true, Some("Alt+Left"))?,
                        &MenuItem::with_id(app, "forward", "Forward", true, Some("Alt+Right"))?,
                        &MenuItem::with_id(app, "reload", "Reload", true, Some("F5"))?,
                        &MenuItem::with_id(app, "downloads", "Open the Downloads folder", true, None::<&str>)?,
                        &MenuItem::with_id(app, "restart", "Restart AlexBooks", true, None::<&str>)?,
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
                    "back" => app.get_webview_window("main").map(|w| w.eval("history.back()")).unwrap_or(Ok(())).map_err(|e| e.to_string()),
                    "forward" => app.get_webview_window("main").map(|w| w.eval("history.forward()")).unwrap_or(Ok(())).map_err(|e| e.to_string()),
                    "downloads" => { show_download(&downloads_dir(app)); Ok(()) }
                    "restart" => { release_lock(); app.restart(); }
                    "switch" => show_start(app, "switch"),
                    "update" => show_start(app, "update"),
                    "about" => show_start(app, "about"),
                    _ => Ok(()),
                };
            });
            // 0.3.1: while the window stays open for days, look for a newer launcher now and then (every hour since
            // 0.4.1 — the office fetches one and expects the PCs to follow the same day). 1.2.0: a newer one is no
            // longer pushed in the person's face mid-work; a small notice says it installs at the next start (once
            // per version), and Settings → Updates has the "restart now" link for whoever wants it today.
            // 1.0.1: the lock's heartbeat every two seconds, and a look for a "wake" from a second start every second
            let h_lock = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut n: u32 = 0;
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    n += 1;
                    if n % 2 == 0 { write_lock(); }
                    let wake = lock_dir().join(WAKE_NAME);
                    if wake.exists() {
                        let what = fs::read_to_string(&wake).unwrap_or_default();
                        let _ = fs::remove_file(&wake);
                        surface(&h_lock);
                        if what.trim().to_ascii_lowercase().ends_with(".abp") && Path::new(what.trim()).is_file() {
                            // 1.3.0: a package was double-clicked while this window was open — go set that office up
                            if let Some(p) = h_lock.try_state::<PendingPackage>() { if let Ok(mut g) = p.0.lock() { *g = Some(what.trim().to_string()); } }
                            let _ = show_start(&h_lock, "package");
                        } else {
                            page_notice(&h_lock, "AlexBooks is already open — this is it.");
                        }
                    }
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut told_about = String::new();
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    let server = read_config(&handle).server;
                    if server.is_empty() { continue; }
                    let Ok(u) = normalize(&server) else { continue };
                    let Ok(endpoint) = u.join("desktop/latest.json") else { continue };
                    let ca = saved_ca(&handle).and_then(|pem| reqwest::Certificate::from_pem(pem.as_bytes()).ok());
                    let updater = handle.updater_builder().endpoints(vec![endpoint]).ok().map(|b| b.timeout(Duration::from_secs(10))
                        .configure_client(move |c| match &ca { Some(cert) => c.add_root_certificate(cert.clone()), None => c }).build().ok()).flatten();
                    if let Some(updater) = updater {
                        if let Ok(Some(upd)) = updater.check().await {
                            if told_about != upd.version {
                                told_about = upd.version.clone();
                                page_notice(&handle, &format!("AlexBooks for Windows {} is ready — it installs by itself the next time AlexBooks starts. To do it now: Settings → Updates → Restart AlexBooks to update now.", upd.version));
                            }
                        }
                    }
                }
            });
            // Always start on the shipped page: it checks the saved server, the certificate and updates, then opens it.
            let h_dl = app.handle().clone();
            let h_nav = app.handle().clone();
            let h_new = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("AlexBooks")
                .inner_size(1280.0, 860.0)
                .min_inner_size(900.0, 600.0)
                .user_agent(&format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AlexBooksDesktop/{}", env!("CARGO_PKG_VERSION")))
                .zoom_hotkeys_enabled(true)                     // Ctrl + / Ctrl - / Ctrl 0, like a browser
                .disable_drag_drop_handler()                    // 0.4.0: let the page's own drop zones receive files from Explorer
                .on_download(move |_wv, ev| {
                    match ev {
                        DownloadEvent::Requested { url, destination } => {
                            let name = download_name(destination, &url);
                            *destination = unique_path(&downloads_dir(&h_dl), &name);
                            page_notice(&h_dl, &format!("Downloading {name}…"));
                        }
                        DownloadEvent::Finished { path, success, .. } => match (success, path) {
                            (true, Some(p)) => {
                                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                page_notice(&h_dl, &format!("Saved to Downloads: {name}"));
                                show_download(&p);
                            }
                            _ => page_notice(&h_dl, "The download did not finish."),
                        },
                        _ => {}
                    }
                    true
                })
                .on_navigation(move |url| {
                    if is_apply_update(url) && is_ours(&h_nav, url) {
                        // 1.2.0: "Restart AlexBooks to update now" on Settings → Updates — the server's own
                        // page only explains the link; this window runs the update screen instead
                        let h = h_nav.clone();
                        tauri::async_runtime::spawn(async move { let _ = show_start(&h, "apply"); });
                        false
                    } else if is_ours(&h_nav, url) || matches!(url.scheme(), "about" | "blob" | "data") {
                        true
                    } else {
                        open_outside(url);                      // github.com, mailto:, a bank — the PC's browser, not this window
                        false
                    }
                })
                .on_new_window(move |url, _features| {
                    if is_ours(&h_new, &url) {
                        // target=_blank on our own server (a PDF, a statement's original): a second AlexBooks window
                        let label = format!("w{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
                        let _ = WebviewWindowBuilder::new(&h_new, label, WebviewUrl::External(url))
                            .title("AlexBooks").inner_size(1100.0, 820.0).zoom_hotkeys_enabled(true).build();
                    } else {
                        open_outside(&url);
                    }
                    NewWindowResponse::Deny
                })
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building AlexBooks")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                release_lock();                                // a clean close leaves no lock behind
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{download_name, is_apply_update, normalize, parse_beacon, unique_path};

    #[test]
    fn package_argument() {
        let dir = std::env::temp_dir().join(format!("abp-arg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("AlexBooks-Wendy.ABP");
        std::fs::write(&f, b"ABP1").unwrap();
        let fs_ = f.to_string_lossy().to_string();
        assert_eq!(super::package_from_args(vec!["exe".into(), fs_.clone()]), Some(fs_.clone()));
        assert_eq!(super::package_from_args(vec!["exe".into()]), None);
        assert_eq!(super::package_from_args(vec!["exe".into(), "C:\\nope\\missing.abp".into()]), None);
        assert_eq!(super::package_from_args(vec![fs_.clone()]), None, "the program's own path is never a package");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_update_link() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(is_apply_update(&u("https://vm:8443/desktop/apply-update")));
        assert!(is_apply_update(&u("https://192.168.1.117:8443/desktop/apply-update/")));
        assert!(!is_apply_update(&u("https://vm:8443/desktop/latest.json")));
        assert!(!is_apply_update(&u("https://vm:8443/admin/pcs")));
    }

    #[test]
    fn one_window_per_pc() {
        let dir = std::env::temp_dir().join(format!("alexbooks-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LOCALAPPDATA", &dir);
        assert!(super::claim_single_window());                                        // nobody: this start owns the window
        assert_eq!(super::read_lock().unwrap().pid, std::process::id());
        assert!(super::claim_single_window());                                        // our own lock: still fine
        // another, live window (fresh heartbeat, process running): this start wakes it and steps aside
        let mut child = if cfg!(windows) {                                             // something that stays alive ~20 s
            std::process::Command::new("ping").args(["-n", "20", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap()
        } else {
            std::process::Command::new("sleep").arg("20").spawn().unwrap()
        };
        let other = super::WindowLock { pid: child.id(), alive_at: super::now_secs() };
        std::fs::write(super::lock_dir().join(super::LOCK_NAME), serde_json::to_string(&other).unwrap()).unwrap();
        assert!(!super::claim_single_window());
        assert!(super::lock_dir().join(super::WAKE_NAME).exists());
        super::release_lock();                                                        // not ours: left alone
        assert!(super::read_lock().is_some());
        // the same lock once that process is gone (closed, or replaced by an update): this start takes over
        let _ = child.kill(); let _ = child.wait();
        assert!(super::claim_single_window());
        assert_eq!(super::read_lock().unwrap().pid, std::process::id());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloads_get_a_name_and_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("alexbooks-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let u = url::Url::parse("https://192.0.2.10:8443/clients/6/reports/pl.pdf?start=2025-01-01").unwrap();
        assert_eq!(download_name(std::path::Path::new(""), &u), "pl.pdf");                    // no suggestion: the address
        let suggested = std::env::temp_dir().join("P&L 2025.pdf");
        assert_eq!(download_name(&suggested, &u), "P&L 2025.pdf");                             // the server's Content-Disposition wins
        assert_eq!(unique_path(&dir, "office.zip"), dir.join("office.zip"));
        std::fs::write(dir.join("office.zip"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "office.zip"), dir.join("office (2).zip"));
        std::fs::write(dir.join("office (2).zip"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "office.zip"), dir.join("office (3).zip"));
        assert_eq!(unique_path(&dir, "README"), dir.join("README"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
