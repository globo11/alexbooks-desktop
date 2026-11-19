//! Where a new office keeps its books (1.5.2): on this PC, or in a shared folder on the office's server.
//!
//! The setup page asks. The answer travels to the package's installer as `files/install/where.json` (AlexBooks
//! 2.7.8+ reads it; an older package does not, and the page says so instead of quietly installing on this PC).
//! When the installer is completely done it leaves `%ProgramData%\AlexBooks\install-result.json`; the page waits
//! for that note before it opens the office, so nobody makes the first login while the books are still on their
//! way to the server, and it says so when the folder was asked for and the office ended up on this PC.
//!
//! Nothing here touches the window or Windows itself (the drive-letter lookup is handed in), so all of it is tested.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The file the installer reads, beside its own script.
pub const ANSWER: &str = "where.json";

/// `\\SERVER\share[\...]`, `X:\...`, or any other whole path - nothing relative.
pub fn whole_path(s: &str) -> bool {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix(r"\\") {
        let mut parts = rest.split('\\').filter(|p| !p.is_empty());
        return parts.next().is_some() && parts.next().is_some();
    }
    let b = t.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        return true;
    }
    Path::new(t).is_absolute()
}

/// The first `\\server\share` in what `reg query HKCU\Network\P /v RemotePath` or `net use P:` printed (the words
/// around it change with the language of Windows; the name itself does not).
pub fn first_unc(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let at = l.find(r"\\")?;
        let v = l[at..].trim().trim_end_matches('\\');
        if whole_path(v) { Some(v.to_string()) } else { None }
    })
}

/// A mapped drive letter (P:) belongs to the person; the installer runs as an administrator and does not have it.
/// So the folder is handed on under the server's own name.
pub fn with_share_name(folder: &str, lookup: &dyn Fn(char) -> Option<String>) -> String {
    let t = folder.trim().trim_matches('"').trim();
    let t = if t.len() > 3 { t.trim_end_matches(|c| c == '\\' || c == '/') } else { t };
    let b = t.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        if let Some(remote) = lookup(b[0] as char) {
            let rest = t[2..].trim_end_matches(|c| c == '\\' || c == '/');
            return format!("{}{}", remote.trim_end_matches('\\'), rest);
        }
        if b.len() == 2 {
            return format!("{t}\\");
        }
    }
    t.to_string()
}

/// Does this folder (or the AlexBooks folder inside it) already hold an office's books?
pub fn holds_office(folder: &Path) -> bool {
    [folder.to_path_buf(), folder.join("AlexBooks")].iter().any(|d| d.join("books").join("current.json").is_file())
}

/// A folder this PC can open and save in.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Checked {
    /// the folder under the name the installer can use
    pub folder: String,
    /// on a disk of this PC, not on another machine: the other PCs only reach it if this PC shares it and stays on
    pub local: bool,
}

/// Can this PC open the folder and save in it? What to hand the installer, or what to tell the person.
pub fn check(folder: &str, lookup: &dyn Fn(char) -> Option<String>) -> Result<Checked, String> {
    let f = with_share_name(folder, lookup);
    if f.is_empty() {
        return Err("Pick the shared folder on the office's server.".into());
    }
    if !whole_path(&f) {
        return Err(r"Give the whole name of the folder, for example \\SERVER\office.".into());
    }
    let p = Path::new(&f);
    if !p.is_dir() {
        return Err(format!("This PC cannot open {f}. Check the name, and that the server is on."));
    }
    if holds_office(p) {
        return Err(format!("{f} already holds an office's books. To add this PC to that office, open that folder and double-click Workstation Setup."));
    }
    let probe = p.join(format!(".alexbooks-check-{}", std::process::id()));
    if std::fs::write(&probe, b"ok").is_err() {
        return Err(format!("AlexBooks cannot save files in {f}. Whoever looks after the server has to let this PC change files in that folder."));
    }
    let _ = std::fs::remove_file(&probe);
    let local = !f.starts_with(r"\\");
    Ok(Checked { folder: f, local })
}

/// Does the package's installer read the answer? (`installer` is its `Install AlexBooks.cmd`.)
pub fn asks_where(installer: &Path) -> bool {
    let Some(top) = installer.parent() else { return false };
    std::fs::read(top.join("files").join("install").join("bootstrap.ps1")).map(|b| String::from_utf8_lossy(&b).contains(ANSWER)).unwrap_or(false)
}

/// Leave the answer for the installer: a folder on the server, or `None` for this PC.
pub fn leave_answer(installer: &Path, folder: Option<&str>) -> Result<(), String> {
    let dir = installer.parent().ok_or_else(|| "The package has no installer inside.".to_string())?.join("files").join("install");
    let v = match folder {
        Some(f) => serde_json::json!({ "where": "folder", "folder": f }),
        None => serde_json::json!({ "where": "pc" }),
    };
    // (the mark at the start tells Windows PowerShell the file is UTF-8: a server or folder name may carry an accent)
    std::fs::write(dir.join(ANSWER), format!("\u{feff}{v}")).map_err(|e| format!("Could not prepare the installer: {e}"))
}

/// What the installer said when it was completely done.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InstallResult {
    #[serde(default)]
    pub at: u64,
    /// "folder" or "pc": where the books ended up
    #[serde(default, rename = "where")]
    pub place: String,
    #[serde(default)]
    pub folder: String,
    /// the folder that was asked for ("" when this PC was)
    #[serde(default)]
    pub wanted: String,
    #[serde(default)]
    pub problem: String,
}

/// The installer's note, when it belongs to the install that began at `started` (an older note is another install's).
pub fn read_result(text: &str, started: u64) -> Option<InstallResult> {
    let r: InstallResult = serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).ok()?;
    if r.at + 5 >= started && !r.place.is_empty() { Some(r) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: char) -> Option<String> { None }

    #[test]
    fn a_folder_has_to_be_named_whole() {
        assert!(whole_path(r"\\SERVER\office"));
        assert!(whole_path(r"\\SERVER\office\books 2026"));
        assert!(whole_path(r"P:\"));
        assert!(whole_path(r"D:\Shared\office"));
        assert!(!whole_path(r"\\SERVER"));
        assert!(!whole_path(r"\\SERVER\"));
        assert!(!whole_path("office"));
        assert!(!whole_path(""));
    }

    #[test]
    fn the_server_name_is_read_whatever_language_windows_speaks() {
        let reg = "\r\nHKEY_CURRENT_USER\\Network\\P\r\n    RemotePath    REG_SZ    \\\\NAS01\\office\r\n\r\n";
        assert_eq!(first_unc(reg).as_deref(), Some(r"\\NAS01\office"));
        let net = "Nombre local        P:\r\nNombre remoto       \\\\SERVIDOR\\Oficina Peña\r\nTipo de recurso     Disco\r\n";
        assert_eq!(first_unc(net).as_deref(), Some(r"\\SERVIDOR\Oficina Peña"));
        assert_eq!(first_unc("ERROR: The system was unable to find the specified registry key or value.\r\n"), None);
    }

    #[test]
    fn a_drive_letter_becomes_the_servers_own_name() {
        let p = |c: char| if c == 'P' || c == 'p' { Some(r"\\NAS01\office".to_string()) } else { None };
        assert_eq!(with_share_name(r"P:\", &p), r"\\NAS01\office");
        assert_eq!(with_share_name("P:", &p), r"\\NAS01\office");
        assert_eq!(with_share_name(r"p:\books\2026\", &p), r"\\NAS01\office\books\2026");
        assert_eq!(with_share_name(r"D:\Shared\office\", &p), r"D:\Shared\office");      // a disk of this PC stays as it is
        assert_eq!(with_share_name("D:", &p), r"D:\");
        assert_eq!(with_share_name(r#" "\\SERVER\office\" "#, &none), r"\\SERVER\office");
    }

    #[test]
    fn a_folder_is_tried_before_the_install_starts() {
        let dir = std::env::temp_dir().join(format!("alexbooks-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = dir.to_string_lossy().to_string();
        // it opens and takes a file: good, and nothing is left behind
        let ok = check(&name, &none).unwrap();
        assert_eq!(ok.folder, name);
        assert!(ok.local);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // not there
        assert!(check(&format!("{name}-missing"), &none).unwrap_err().contains("cannot open"));
        assert!(check("office", &none).unwrap_err().contains("whole name"));
        assert!(check("  ", &none).unwrap_err().contains("Pick the shared folder"));
        // an office already lives there: this PC joins it with Workstation Setup, it is not set up over it
        std::fs::create_dir_all(dir.join("AlexBooks").join("books")).unwrap();
        std::fs::write(dir.join("AlexBooks").join("books").join("current.json"), "{}").unwrap();
        assert!(check(&name, &none).unwrap_err().contains("Workstation Setup"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_answer_reaches_a_package_that_asks_and_an_older_one_is_told_apart() {
        let top = std::env::temp_dir().join(format!("alexbooks-answer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        let install = top.join("files").join("install");
        std::fs::create_dir_all(&install).unwrap();
        let cmd = top.join("Install AlexBooks.cmd");
        std::fs::write(&cmd, "@echo off").unwrap();
        std::fs::write(install.join("bootstrap.ps1"), "# an older installer: it asks nothing").unwrap();
        assert!(!asks_where(&cmd));
        std::fs::write(install.join("bootstrap.ps1"), "$whereFile = Join-Path $Source \"install\\where.json\"").unwrap();
        assert!(asks_where(&cmd));
        leave_answer(&cmd, Some(r"\\SERVIDOR\Oficina Peña")).unwrap();
        let raw = std::fs::read(install.join(ANSWER)).unwrap();
        assert_eq!(&raw[..3], &[0xEF, 0xBB, 0xBF]);
        let v: serde_json::Value = serde_json::from_slice(&raw[3..]).unwrap();
        assert_eq!(v["where"], "folder");
        assert_eq!(v["folder"], r"\\SERVIDOR\Oficina Peña");
        leave_answer(&cmd, None).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(install.join(ANSWER)).unwrap()[3..]).unwrap();
        assert_eq!(v["where"], "pc");
        assert!(v.get("folder").is_none());
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn the_installers_note_counts_only_for_this_install() {
        let note = "\u{feff}{\r\n    \"at\":  1795123456,\r\n    \"where\":  \"folder\",\r\n    \"folder\":  \"\\\\\\\\NAS01\\\\office\\\\AlexBooks\",\r\n    \"wanted\":  \"\\\\\\\\NAS01\\\\office\",\r\n    \"problem\":  \"\"\r\n}\r\n";
        let r = read_result(note, 1795123000).unwrap();
        assert_eq!(r.place, "folder");
        assert_eq!(r.folder, r"\\NAS01\office\AlexBooks");
        assert_eq!(r.wanted, r"\\NAS01\office");
        assert_eq!(read_result(note, 1795129999), None);                    // left by an install before this one
        assert_eq!(read_result("not json", 0), None);
        let fell_back = r#"{"at": 10, "where": "pc", "folder": "", "wanted": "\\\\NAS01\\office", "problem": "This PC cannot open it."}"#;
        let r = read_result(fell_back, 10).unwrap();
        assert!(r.place == "pc" && !r.wanted.is_empty() && r.problem.contains("cannot open"));
        // what the page gets: the word "where", as the installer wrote it
        assert!(serde_json::to_string(&r).unwrap().contains("\"where\":\"pc\""));
    }
}
