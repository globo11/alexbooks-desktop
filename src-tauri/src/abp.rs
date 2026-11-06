//! The `.abp` file: an office's install package wrapped for email (launcher 1.1.0).
//!
//! Same layout as `app/abp.py` on the server: `ABP1` · 16-byte salt · 12-byte nonce · AES-256-GCM ciphertext of the
//! zip, key = PBKDF2-HMAC-SHA256(passphrase lower-cased with single spaces, salt, 200 000 rounds), the magic as
//! associated data. Keep the two in step.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"ABP1";
const ROUNDS: u32 = 200_000;

fn normalize(passphrase: &str) -> String {
    passphrase.to_lowercase().replace('-', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn open(data: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if data.len() < 4 + 16 + 12 + 16 || &data[..4] != MAGIC {
        return Err("That is not an AlexBooks package file (.abp).".into());
    }
    let (salt, nonce, ct) = (&data[4..20], &data[20..32], &data[32..]);
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(normalize(passphrase).as_bytes(), salt, ROUNDS, &mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: MAGIC })
        .map_err(|_| "Wrong passphrase — check the email and type it exactly (dashes or spaces are fine).".into())
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
}
