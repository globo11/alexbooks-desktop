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

The installer and its updater signature are built with the project's signing key: the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the public half is in `tauri.conf.json`). The AlexBooks server fetches each Release (Settings → Other PCs) and hands it to the office PCs, which update themselves from the server.

Release: bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, add the entry to `CHANGELOG.md`, write `RELEASE-NOTES.md`, commit and push `main`. The workflow creates the tag and the Release itself.

## What is on the PC

`%APPDATA%\com.alextaxservices.alexbooks\config.json` — the server address; `ca.pem` next to it — the office certificate this PC was told to trust (also added to the user's Windows certificate store). `%LOCALAPPDATA%\AlexBooks\` — the program. Nothing else.
