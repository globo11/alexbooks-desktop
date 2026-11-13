//! What this window switches off in the page engine (1.5.0).
//!
//! The window draws the office's pages with the engine Windows already has (WebView2, the Chromium engine inside
//! Edge). An engine made for a browser brings a browser's habits with it: a right-click menu with Back / Refresh /
//! Save as / Print, an offer to translate the page, a downloads flyout on Ctrl+J, "caret browsing" on F7, a
//! view-source window on Ctrl+U — and it reports to its maker: a reputation check on addresses (SmartScreen) and a
//! crash dump sent to Microsoft when it falls over. None of that belongs in an accounting program, and a crash dump
//! can hold what was on the screen. This module is the one place that says what is switched off, and how:
//!
//!   * `ENGINE_ARGS` — the switches the engine is started with (no SmartScreen, no translation offer);
//!   * `own_environment` — the engine is started by us, not by the toolkit, so that crash dumps stay in this PC's
//!     own folder instead of going to Microsoft (the toolkit has no setting for that);
//!   * `tidy` — per window: the right-click menu keeps only what a text box needs (Undo, Redo, Cut, Copy, Paste,
//!     Select all) and does not appear at all where it would be empty; the browser-only keys do nothing; password
//!     saving and the reputation check are off.
//!
//! What is deliberately LEFT on, because an office uses it: Ctrl+F (find on the page), Ctrl+P (print the page),
//! Ctrl + / Ctrl - / Ctrl 0 and Ctrl+wheel (text size), F5, Alt+Left / Alt+Right, and every editing key.
//!
//! Everything here is best effort and never stops the window from opening: if the engine cannot be started our way
//! (another program is using this PC's AlexBooks engine folder with other settings — the moment after an update),
//! the window opens the way it did before 1.5.0 and the next start gets it right. `engine.txt` next to the window's
//! lock file says what this start managed, for whoever is asked "is it switched off on this PC?".

/// The switches every AlexBooks window starts the engine with. The first three are the toolkit's own defaults (they
/// must be repeated: giving any switch replaces the defaults) — no "mini menu" over selected text, no Edge PDF
/// extras, no SmartScreen; `Translate` is ours: no "Translate this page?" offer.
pub const ENGINE_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Translate";

/// Right-click menu entries that stay (the engine's own, unlocalized names). Everything else — back, forward,
/// reload, saveAs, print, share, webCapture, inspectElement, the image and link entries… — is a browser's.
const MENU_KEEP: [&str; 7] = ["undo", "redo", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll"];

/// Which entries of a right-click menu to take out: `items` is (is a separator line, name) in menu order; the answer
/// is their positions. Entries not on `MENU_KEEP` go, and so does every separator left at the top, at the bottom or
/// doubled. When the answer is every position, the menu is not shown at all.
pub fn menu_drop(items: &[(bool, String)]) -> Vec<usize> {
    let mut drop: Vec<usize> = Vec::new();
    let mut kept_since_separator = false;                      // a kept entry since the last kept separator (or the top)
    let mut last_separator: Option<usize> = None;               // the last separator still in the menu
    for (i, (is_separator, name)) in items.iter().enumerate() {
        if *is_separator {
            if kept_since_separator {
                last_separator = Some(i);
                kept_since_separator = false;
            } else {
                drop.push(i);                                   // at the top, or right after another separator
            }
        } else if MENU_KEEP.contains(&name.as_str()) {
            kept_since_separator = true;
        } else {
            drop.push(i);
        }
    }
    if !kept_since_separator {
        if let Some(i) = last_separator {
            drop.push(i);                                       // nothing kept after it: it would end the menu
        }
    }
    drop.sort_unstable();
    drop
}

/// Is this key press one only a browser has a use for? `vk` is the Windows virtual-key code. A short, exact list —
/// never "everything with Ctrl": on a Spanish keyboard AltGr (which Windows reports as Ctrl+Alt) types @ and €, and
/// Ctrl with the arrows, Backspace, Enter, A/C/V/X/Z/Y are how people edit.
pub fn browser_only_key(vk: u32, ctrl: bool, shift: bool, alt: bool) -> bool {
    const F7: u32 = 0x76;                                       // "Turn on caret browsing?"
    const F12: u32 = 0x7B;                                      // developer tools
    if alt {
        return false;                                           // AltGr combinations type characters
    }
    if !ctrl {
        return !shift && (vk == F7 || vk == F12);
    }
    match (shift, char::from_u32(vk).unwrap_or(' ')) {
        (false, 'J') => true,                                   // the engine's downloads flyout (the window has its own notice)
        (false, 'U') => true,                                   // view source
        (true, 'I') | (true, 'J') | (true, 'C') => true,        // developer tools, three ways
        _ => false,
    }
}

/// Where `engine.txt` is written: next to the window's lock file.
fn note_path() -> std::path::PathBuf {
    crate::lock_dir().join("engine.txt")
}

/// Start `engine.txt` afresh for this start of the program.
pub fn note_start(version: &str) {
    let d = crate::lock_dir();
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(note_path(), format!("AlexBooks for Windows {version}\n"));
}

/// Add one line to `engine.txt`.
pub fn note(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(note_path()) {
        let _ = writeln!(f, "{line}");
    }
}

/// Set `ALEXBOOKS_ENGINE_LOG=1` before starting the program and `engine.txt` also lists every right-click menu the
/// engine offered and every key it was not given — for finding out why something does or does not appear.
fn verbose() -> bool {
    std::env::var_os("ALEXBOOKS_ENGINE_LOG").is_some()
}

#[cfg(windows)]
pub use self::win::{own_environment, tidy, Environment};

#[cfg(windows)]
mod win {
    use super::{browser_only_key, menu_drop, note, verbose, ENGINE_ARGS};
    use std::{path::Path, sync::mpsc};
    use webview2_com::{
        take_pwstr, AcceleratorKeyPressedEventHandler, ContextMenuRequestedEventHandler, CoreWebView2EnvironmentOptions,
        CreateCoreWebView2EnvironmentCompletedHandler, Microsoft::Web::WebView2::Win32::*,
    };
    use windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};

    pub type Environment = ICoreWebView2Environment;

    /// One try at starting the engine our way: the same folder, language and switches the toolkit would use, plus
    /// "crash dumps stay here" (`IsCustomCrashReportingEnabled`: Windows does not send them to Microsoft).
    fn create(data_dir: &Path) -> Result<Environment, String> {
        let options = CoreWebView2EnvironmentOptions::default();
        let (tx, rx) = mpsc::channel();
        unsafe {
            options.set_additional_browser_arguments(ENGINE_ARGS.to_string());
            options.set_is_custom_crash_reporting_enabled(true);
            options.set_are_browser_extensions_enabled(false);
            // the person's Windows display language, as the toolkit sets it
            let mut lang = [0u16; MAX_LOCALE_NAME as usize];
            let n = LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut lang), LOCALE_ALLOW_NEUTRAL_NAMES);
            if n > 1 {
                options.set_language(String::from_utf16_lossy(&lang[..(n as usize - 1)]));
            }
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                &HSTRING::from(data_dir.as_os_str()),
                &ICoreWebView2EnvironmentOptions::from(options),
                &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |error_code, environment| {
                    let result = error_code.and_then(|_| environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                    tx.send(result).map_err(|_| windows::core::Error::from(E_UNEXPECTED))
                })),
            )
            .map_err(|e| e.to_string())?;
        }
        match webview2_com::wait_with_pump(rx) {
            Ok(Ok(environment)) => Ok(environment),
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(format!("{e:?}")),
        }
    }

    /// Start the engine our way, in the folder the toolkit keeps it in (so the sign-in and the drafts stay). Must be
    /// called on the main thread, the way the toolkit does it. An error is not fatal: the caller opens the window
    /// the toolkit's way instead.
    pub fn own_environment(data_dir: &Path) -> Result<Environment, String> {
        let _ = std::fs::create_dir_all(data_dir);
        let environment = create(data_dir)?;
        let mut v = PWSTR::null();
        let version = unsafe { environment.BrowserVersionString(&mut v).map(|_| take_pwstr(v)).unwrap_or_default() };
        note(&format!("engine: started by AlexBooks (WebView2 {version}); crash reports stay on this PC"));
        Ok(environment)
    }

    /// The per-window part: settings, the right-click menu, the browser-only keys. Every step is on its own: one that
    /// this PC's engine does not know (an old runtime) is skipped and noted, the rest still happen.
    pub fn tidy(window: &tauri::WebviewWindow) {
        let label = window.label().to_string();
        let done = window.with_webview(move |wv| unsafe {
            let controller = wv.controller();
            let Ok(core) = controller.CoreWebView2() else {
                note(&format!("{label}: no engine handle — nothing switched off"));
                return;
            };
            let mut did: Vec<&str> = Vec::new();
            if let Ok(settings) = core.Settings() {
                if let Ok(s) = settings.cast::<ICoreWebView2Settings4>() {
                    if s.SetIsPasswordAutosaveEnabled(false).is_ok() {
                        did.push("no password saving");
                    }
                }
                if let Ok(s) = settings.cast::<ICoreWebView2Settings8>() {
                    if s.SetIsReputationCheckingRequired(false).is_ok() {
                        did.push("no SmartScreen check");
                    }
                }
            }
            // the right-click menu: only what a text box needs, and no menu where that leaves nothing
            if let Ok(core11) = core.cast::<ICoreWebView2_11>() {
                let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let Ok(items) = args.MenuItems() else { return Ok(()) };
                    let mut count = 0u32;
                    if items.Count(&mut count).is_err() {
                        return Ok(());
                    }
                    let mut seen: Vec<(bool, String)> = Vec::with_capacity(count as usize);
                    for i in 0..count {
                        let Ok(item) = items.GetValueAtIndex(i) else { return Ok(()) };   // cannot read it: leave the menu alone
                        let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND;
                        let _ = item.Kind(&mut kind);
                        let mut name = PWSTR::null();
                        let name = if item.Name(&mut name).is_ok() { take_pwstr(name) } else { String::new() };
                        seen.push((kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR, name));
                    }
                    let drop = menu_drop(&seen);
                    if verbose() {
                        let names: Vec<String> = seen.iter().map(|(sep, n)| if *sep { "-".to_string() } else { n.clone() }).collect();
                        note(&format!("menu offered: {} | taken out: {} of {}", names.join(","), drop.len(), seen.len()));
                    }
                    if drop.len() == seen.len() {
                        let _ = args.SetHandled(true);                                    // nothing left: no menu
                        return Ok(());
                    }
                    for i in drop.iter().rev() {
                        let _ = items.RemoveValueAtIndex(*i as u32);
                    }
                    Ok(())
                }));
                let mut token = 0i64;
                if core11.add_ContextMenuRequested(&handler, &mut token).is_ok() {
                    did.push("right-click menu trimmed");
                }
            }
            // the browser-only keys
            let handler = AcceleratorKeyPressedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_UP;
                if args.KeyEventKind(&mut kind).is_err() {
                    return Ok(());
                }
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut vk = 0u32;
                if args.VirtualKey(&mut vk).is_err() {
                    return Ok(());
                }
                let down = |key: u16| (GetKeyState(key as i32) as u16 & 0x8000) != 0;
                if browser_only_key(vk, down(VK_CONTROL.0), down(VK_SHIFT.0), down(VK_MENU.0)) {
                    let _ = args.SetHandled(true);
                    if verbose() {
                        note(&format!("key not given to the engine: {vk:#04x}"));
                    }
                }
                Ok(())
            }));
            let mut token = 0i64;
            if controller.add_AcceleratorKeyPressed(&handler, &mut token).is_ok() {
                did.push("browser-only keys off");
            }
            note(&format!("{label}: {}", if did.is_empty() { "nothing switched off".to_string() } else { did.join(", ") }));
        });
        if done.is_err() {
            note("could not reach the engine — nothing switched off");
        }
    }
}

/// The folder the toolkit keeps the engine's own files in (the sign-in cookie, the drafts): the app's local data
/// folder. Ours must be the same one, or everyone would be signed out by the update.
pub fn data_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_local_data_dir().ok()
}

#[cfg(test)]
mod tests {
    use super::{browser_only_key, menu_drop, ENGINE_ARGS};

    fn menu(names: &[&str]) -> Vec<(bool, String)> {
        names.iter().map(|n| (*n == "-", n.to_string())).collect()
    }
    fn left(names: &[&str]) -> Vec<String> {
        let items = menu(names);
        let drop = menu_drop(&items);
        items.iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, (_, n))| n.clone()).collect()
    }

    #[test]
    fn the_page_menu_is_not_shown_at_all() {
        // right-click on an empty part of a page: everything the engine offers is a browser's
        let page = ["back", "forward", "reload", "-", "saveAs", "print", "-", "share", "webCapture", "-", "inspectElement"];
        assert_eq!(menu_drop(&menu(&page)).len(), page.len());
        assert_eq!(menu_drop(&menu(&["openLinkInNewWindow", "-", "saveLinkAs", "copyLinkLocation"])).len(), 4);
        assert!(menu_drop(&[]).is_empty());
    }

    #[test]
    fn a_text_box_keeps_its_editing_menu() {
        let field = ["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll", "-", "share", "-", "inspectElement"];
        assert_eq!(left(&field), ["undo", "redo", "-", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll"]);
        // selected text on a page: Copy stays, alone, with no line above or below it
        assert_eq!(left(&["copy", "-", "print", "share", "-", "inspectElement"]), ["copy"]);
        assert_eq!(left(&["-", "-", "copy", "-", "-", "paste", "-"]), ["copy", "-", "paste"]);
    }

    #[test]
    fn only_a_browsers_keys_are_taken_away() {
        let (j, u, i, c, f7, f12) = (0x4A, 0x55, 0x49, 0x43, 0x76, 0x7B);
        assert!(browser_only_key(j, true, false, false));          // Ctrl+J: downloads flyout
        assert!(browser_only_key(u, true, false, false));          // Ctrl+U: view source
        assert!(browser_only_key(i, true, true, false));           // Ctrl+Shift+I: developer tools
        assert!(browser_only_key(f7, false, false, false));        // F7: caret browsing
        assert!(browser_only_key(f12, false, false, false));
        // what an office uses stays: copy, paste, find, print, text size, reload, select all, undo
        for vk in [c, 0x56, 0x58, 0x5A, 0x59, 0x41, 0x46, 0x50, 0x52, 0xBB, 0xBD, 0x30, 0x74] {
            assert!(!browser_only_key(vk, true, false, false), "Ctrl+{vk:#x} must keep working");
        }
        assert!(!browser_only_key(0x74, false, false, false));     // F5
        assert!(!browser_only_key(0x25, false, false, true));      // Alt+Left
        // AltGr (Ctrl+Alt) types characters on a Spanish keyboard: never touched
        assert!(!browser_only_key(j, true, false, true));
        assert!(!browser_only_key(u, true, false, true));
        assert!(!browser_only_key(0x51, true, false, true));       // AltGr+Q = @
        assert!(!browser_only_key(j, false, false, false));        // the letter j
        assert!(browser_only_key(c, true, true, false));           // Ctrl+Shift+C: the developer tools' element picker
    }

    #[test]
    fn the_toolkits_own_switches_are_kept() {
        // giving any switch replaces the toolkit's defaults, so they must be repeated
        for f in ["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection", "Translate"] {
            assert!(ENGINE_ARGS.contains(f), "{f} is missing");
        }
        assert!(ENGINE_ARGS.starts_with("--disable-features="));
        assert!(!ENGINE_ARGS.contains(' '), "one switch only: every window must start the engine with the very same text");
    }
}
