//! The `.abp` file: an office's install package wrapped for email (launcher 1.1.0).
//!
//! Same layout as `app/abp.py` on the server: `ABP1` · 16-byte salt · 12-byte nonce · AES-256-GCM ciphertext of the
//! zip, key = PBKDF2-HMAC-SHA256(passphrase lower-cased with single spaces, salt, 200 000 rounds), the magic as
//! associated data. `ABP2` (1.4.2 / AlexBooks 2.4.9) puts a readable label first — 2-byte length + JSON
//! {"office","license","built"} — bound into the ciphertext as associated data, so the screen can say which package
//! a file is before the passphrase is typed. Keep the two in step.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"ABP1";
const MAGIC2: &[u8] = b"ABP2";
const ROUNDS: u32 = 200_000;

/// What an ABP2 file says about itself (empty strings for an ABP1 file).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Label {
    #[serde(default)] pub office: String,
    #[serde(default)] pub license: String,
    #[serde(default)] pub built: String,
}

/// (associated data, label, the bytes from the salt on) for either layout.
fn split(data: &[u8]) -> Result<(&[u8], Label, &[u8]), String> {
    let bad = || "That is not an AlexBooks package file (.abp).".to_string();
    if data.len() >= 6 && &data[..4] == MAGIC2 {
        let n = u16::from_be_bytes([data[4], data[5]]) as usize;
        if data.len() < 6 + n { return Err(bad()); }
        let label: Label = serde_json::from_slice(&data[6..6 + n]).map_err(|_| bad())?;
        return Ok((&data[..6 + n], label, &data[6 + n..]));
    }
    if data.len() >= 4 && &data[..4] == MAGIC {
        return Ok((MAGIC, Label::default(), &data[4..]));
    }
    Err(bad())
}

/// The label of a sealed file, readable without the passphrase (all-empty for an older file).
pub fn label(data: &[u8]) -> Result<Label, String> {
    split(data).map(|(_, l, _)| l)
}

fn normalize(passphrase: &str) -> String {
    passphrase.to_lowercase().replace('-', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn open(data: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    let (aad, lbl, rest) = split(data)?;
    if rest.len() < 16 + 12 + 16 {
        return Err("That is not an AlexBooks package file (.abp).".into());
    }
    let (salt, nonce, ct) = (&rest[..16], &rest[16..28], &rest[28..]);
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(normalize(passphrase).as_bytes(), salt, ROUNDS, &mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| {
            if lbl.license.is_empty() {
                "Wrong passphrase — check the email and type it exactly (dashes or spaces are fine).".to_string()
            } else {
                format!("Wrong passphrase for this package ({}{}). Use the passphrase from the email that names {} — an older AlexBooks email's passphrase will not open this file. Dashes or spaces are fine.",
                        lbl.license, if lbl.built.is_empty() { String::new() } else { format!(", built {}", lbl.built) }, lbl.license)
            }
        })
}

/// Unpack the zip into `dir` and return the path of `Install AlexBooks.cmd` inside it. Paths that try to climb out
/// of the folder are refused.
pub fn unpack(zip_bytes: &[u8], dir: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("The package is damaged: {e}"))?;
    let mut installer: Option<PathBuf> = None;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name().map(|p| p.to_path_buf()) else { continue };
        let out = dir.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
        if rel.file_name().map(|n| n == "Install AlexBooks.cmd").unwrap_or(false) {
            installer = Some(out);
        }
    }
    installer.ok_or_else(|| "The package has no installer inside.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_what_the_server_sealed() {
        // Sealed by app/abp.py with passphrase "cedar-falcon-honey-42" around the bytes b"PK zip".
        let hex = include_str!("../tests/sample.abp.hex").trim();
        let data: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert_eq!(open(&data, "Cedar Falcon Honey 42").unwrap(), b"PK zip");
        assert!(open(&data, "cedar-falcon-honey-41").is_err());
        assert!(open(b"PK\x03\x04", "x").is_err());
    }

    #[test]
    fn reads_the_label_of_an_abp2_and_opens_it() {
        // Sealed by app/abp.py 2.4.9 with passphrase "cedar-falcon-honey-42", label office/AB-0007/2026-11-09T10:52, around b"PK zip".
        let hex = include_str!("../tests/sample2.abp.hex").trim();
        let data: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let l = label(&data).unwrap();
        assert_eq!((l.office.as_str(), l.license.as_str(), l.built.as_str()), ("Nelcida Tax (Nelcida Perez)", "AB-0007", "2026-11-09T10:52"));
        assert_eq!(open(&data, "cedar falcon honey 42").unwrap(), b"PK zip");
        let err = open(&data, "cedar-falcon-honey-41").unwrap_err();
        assert!(err.contains("AB-0007") && err.contains("2026-11-09T10:52"));
        assert_eq!(label(b"ABP1xxxx").unwrap(), Label::default());
    }
}

/// 1.3.1: a package carries the day it stops working (``expires`` in files/install/office.json, UTC, written by
/// AlexBooks 2.2.7+). Past it, the package installs nothing — a forwarded email found weeks later is harmless.
/// Older packages, and move packages, have no date and are taken as before.
pub fn check_expiry(zip_bytes: &[u8]) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("The package is damaged: {e}"))?;
    let mut text = String::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        if f.name().ends_with("/files/install/office.json") {
            f.read_to_string(&mut text).map_err(|e| e.to_string())?;
            break;
        }
    }
    let Some(stamp) = json_string(&text, "expires") else { return Ok(()) };
    let Some(expires) = unix_seconds(&stamp) else { return Ok(()) };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    if now > expires {
        return Err(format!("This package expired on {} — packages work for a week. Ask the main office for a new one; it takes them a minute.", &stamp[..10]));
    }
    Ok(())
}

/// The string value of one top-level key in a small JSON object (no dependency for one field).
fn json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = text.find(&needle)? + needle.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("null") { return None; }
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// "2026-11-13T22:45:00+00:00" (UTC) → seconds since 1970. Only the first 19 characters are read.
fn unix_seconds(stamp: &str) -> Option<i64> {
    let s = stamp.get(..19)?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, sec) = (t.next()??, t.next()??, t.next()??);
    // days from civil (Howard Hinnant)
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

#[cfg(test)]
mod expiry_tests {
    #[test]
    fn dates() {
        assert_eq!(super::unix_seconds("1970-01-02T00:00:00+00:00"), Some(86400));
        assert_eq!(super::unix_seconds("2026-11-13T22:45:00+00:00"), Some(1794609900));
        assert_eq!(super::json_string(r#"{"office": "X", "expires": "2026-11-13T22:45:00+00:00"}"#, "expires").as_deref(), Some("2026-11-13T22:45:00+00:00"));
        assert_eq!(super::json_string(r#"{"expires": null}"#, "expires"), None);
    }
}
