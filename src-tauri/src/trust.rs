//! Which certificate authorities this program will install into Windows without a word (1.5.3).
//!
//! To show an office's pages, this program asks Windows to trust the office server's certificate authority. An
//! authority that may sign for ANY name can then pose as any secure website to this Windows user - so a device on
//! the office network that only pretends to be the server could, once trusted, sit between this person and their
//! bank. An AlexBooks server (2.7.13 or newer) hands out an authority that may sign for that computer's own names
//! only. That kind is installed as before. Any other kind is installed only after the person has been told plainly
//! what it could do and said yes.
//!
//! "Its own names only" means: the certificate carries name constraints, marked critical, and every name it may
//! sign for is `localhost`, a name under a suffix that exists only inside networks (.local, .lan, ...), or a single
//! word that is not a real internet ending (a computer called SERVER01 - but not "com", which would cover every
//! .com website). The list of real endings is the public one (publicsuffix.org), in tlds.txt.

use x509_parser::extensions::GeneralName;
use x509_parser::pem::parse_x509_pem;

const INSIDE_ONLY: &[&str] = &[".local", ".lan", ".home", ".internal", ".intranet", ".corp", ".private", ".localdomain", ".home.arpa"];
const TLDS: &str = include_str!("tlds.txt");

/// Is this a name that only means something inside an office's own network?
pub fn office_name(name: &str) -> bool {
    let n = name.trim().trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    if n.is_empty() {
        return false;                                   // an empty name covers every name there is
    }
    if n == "localhost" || INSIDE_ONLY.iter().any(|s| n.ends_with(s) || n == s[1..]) {
        return true;
    }
    if n.contains('.') {
        return false;                                   // a name on the internet, as far as anyone can tell from here
    }
    !TLDS.lines().any(|t| t == n)
}

/// May this authority sign for this office's own names only? `Err` when the text is not a certificate.
pub fn office_only(ca_pem: &str) -> Result<bool, String> {
    let (_, pem) = parse_x509_pem(ca_pem.as_bytes()).map_err(|_| "That is not a certificate.".to_string())?;
    let cert = pem.parse_x509().map_err(|_| "That certificate could not be read.".to_string())?;
    let Ok(Some(ext)) = cert.name_constraints() else { return Ok(false) };
    if !ext.critical {
        return Ok(false);                               // a limit that readers are allowed to skip is not a limit
    }
    let Some(permitted) = &ext.value.permitted_subtrees else { return Ok(false) };
    let mut names = 0;
    for tree in permitted {
        if let GeneralName::DNSName(d) = &tree.base {
            if !office_name(d) {
                return Ok(false);
            }
            names += 1;
        }
    }
    Ok(names > 0)                                       // no names listed = every name allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    // made by ci/make-test-authorities.py (kept beside this file's tests so anyone can make them again)
    const OFFICE: &str = include_str!("../testdata/ca-office.pem");
    const OPEN: &str = include_str!("../testdata/ca-open.pem");
    const DOTCOM: &str = include_str!("../testdata/ca-dotcom.pem");
    const SOFT: &str = include_str!("../testdata/ca-soft.pem");
    const NUMBERS: &str = include_str!("../testdata/ca-numbers-only.pem");
    const BANK: &str = include_str!("../testdata/ca-one-website.pem");

    #[test]
    fn an_authority_for_this_office_only_is_told_apart() {
        assert_eq!(office_only(OFFICE), Ok(true));              // localhost + SERVER01 + frontdesk.local, as 2.7.13 makes it
        assert_eq!(office_only(OPEN), Ok(false));               // no limit at all: every office before 2.7.13, and any impostor
        assert_eq!(office_only(DOTCOM), Ok(false));             // "limited" to com - every .com website
        assert_eq!(office_only(BANK), Ok(false));               // limited to one real website: still not this office's
        assert_eq!(office_only(SOFT), Ok(false));               // the limit is there but not critical
        assert_eq!(office_only(NUMBERS), Ok(false));            // only addresses are limited: every name is allowed
        assert!(office_only("hello").is_err());
    }

    #[test]
    fn names_that_live_inside_an_office() {
        for n in ["localhost", "SERVER01", "front-desk", "DESKTOP-7QK2A1B", "nas.local", "server.office.lan", "pc7.home.arpa", "books.internal"] {
            assert!(office_name(n), "{n}");
        }
        for n in ["com", "COM", ".com", "bank", "tax", "uk", "co.uk", "example.com", "www.mybank.example", "office", "", "."] {
            assert!(!office_name(n), "{n}");
        }
    }
}
