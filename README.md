# AlexBooks for Windows

The office's own window onto the AlexBooks server. The books, the files and the users live on the server; this program only opens it — the way QuickBooks Desktop opens a company file on the office server.

```
 office PC                                   server (the VM)
 AlexBooks.exe ── https ──▶ AlexBooks (FastAPI)   ← all data, all logic, its own updater
   ├ remembers the server address
   ├ its own window + icon + a small menu
   ├ every start: reaches the server → trusts its certificate (once, with your OK) → checks for updates → opens it
   ├ finds the server on the LAN (ALEXBOOKS? beacon, port scan) or takes a typed address
   └ updates itself from the server (/desktop/latest.json), never from the internet
```

## How it is built

`src-tauri/` is a small Rust program (Tauri 2) around WebView2, the Edge engine every Windows 10/11 PC already has, so the installer is about 8 MB. `ui/index.html` is the start page: the first-run choice, the checks on every start, the update and About screens. Nothing is built by hand: GitHub Actions (`.github/workflows/build.yml`) builds `AlexBooks-Setup.exe` on a Windows runner on every push, and publishes a Release on a version tag.

The installer and its updater signature are built with the project's signing key: the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the public half is in `tauri.conf.json`). The AlexBooks server fetches each Release (Settings → Install on a PC) and hands it to the office PCs, which update themselves from the server.

Release: bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `package.json` and `package-lock.json`, add the entry to `CHANGELOG.md`, write `RELEASE-NOTES.md`, commit and push `main`. The workflow creates the tag and the Release itself.

## What is switched off in the engine (1.5.0)

WebView2 is a browser's engine and brings a browser's habits. `src-tauri/src/engine.rs` is the one place that says what this program switches off, and why:

| | How |
|---|---|
| The right-click menu keeps spelling suggestions, Undo / Redo / Cut / Copy / Paste / Select all, and does not appear where that leaves nothing | `ContextMenuRequested` |
| Ctrl+J (downloads panel), F7 (caret browsing), Ctrl+U (view source), F12 and Ctrl+Shift+I/J/C do nothing | `AcceleratorKeyPressed` — an exact list; AltGr combinations are never touched |
| No translation offer, no SmartScreen | engine switches (`ENGINE_ARGS`) and `IsReputationCheckingRequired` |
| A crash report of the engine stays on the PC | the program starts the engine itself with `IsCustomCrashReportingEnabled` (the toolkit has no setting for it) |

Left on, on purpose: Ctrl+F, Ctrl+P, the text size keys, F5, Alt+Left / Alt+Right. What Windows itself collects about the engine ("required diagnostic data") is not a program's to switch off.

The engine refuses a start whose settings differ from one still running in the same folder — the previous version's, for a moment after an update. So the window tries its own start twice, a second and a half apart, and if both are refused the program starts itself once more "the toolkit's way" (`ALEXBOOKS_ENGINE_START`, set by the program for itself), exactly as 1.4 started it; the next start is back to its own. A second start of the program, not a third try in the same one: after a refused start the engine refuses the toolkit's settings in that process too. `%LOCALAPPDATA%\AlexBooks\engine.txt` says what the last start managed; with `ALEXBOOKS_ENGINE_LOG=1` set it also lists each right-click menu the engine offered and each key kept from it.

## The window check

`ci/window-check.ps1` runs in the build, before the Release (and on any `test/…` branch, which builds and checks but never releases): it starts the built program on GitHub's Windows machine against a stand-in office (`ci/office.py`), checks that the window opens it — on a clean start, beside the previous release (the update moment), and on the start after — that the keys above no longer reach the engine while Ctrl+F still does, that a second window opens, and saves screenshots of every right-click and key as the `window-check` artifact. A build that fails it is not released, because the offices' PCs install a Release by themselves.

## What is on the PC

`%APPDATA%\com.alextaxservices.alexbooks\config.json` — the server address; `ca.pem` next to it — the office certificate this PC was told to trust (also added to the user's Windows certificate store). `%LOCALAPPDATA%\AlexBooks\` — the program. Nothing else.

## Keeping the launcher and the app in step

`compat.json` names the oldest AlexBooks the launcher works with (`min_app`). The office server reads it at the release's tag before fetching the installer: a launcher that needs a newer app than the server runs waits on GitHub, and the PCs keep the launcher they have, until the app has updated — then the server fetches it by itself. The other direction is the server's `MIN_LAUNCHER` (app/desktop.py): PCs below it are flagged on Settings → Updates and update at their next start. Raise `min_app` whenever the launcher starts relying on something new on the server (a new address, a new field in `/desktop/hello`).
