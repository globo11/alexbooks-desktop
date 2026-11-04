# AlexBooks for Windows

The office's own window onto the AlexBooks server. The books, the files and the users live on the server; this program only opens it — the way QuickBooks Desktop opens a company file on the office server.

```
 office PC                                   server (the VM)
 AlexBooks.exe ── https ──▶ AlexBooks (FastAPI)   ← all data, all logic, its own updater
   ├ remembers the server address
   ├ its own window + icon
   └ (coming) splash checks · finds the server on the LAN · trusts the office CA · updates itself
```

## How it is built

`src-tauri/` is a small Rust program (Tauri 2) around WebView2, the Edge engine every Windows 10/11 PC already has, so the installer is about 8 MB. `ui/` holds the connect page the shell shows before an address is saved. Nothing is built by hand: GitHub Actions (`.github/workflows/build.yml`) builds `AlexBooks-Setup.exe` on a Windows runner on every push, and publishes a Release on a version tag.

Release: bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, add the entry to `CHANGELOG.md`, write `RELEASE-NOTES.md`, commit and push `main`. The workflow creates the tag and the Release itself.

## What is on the PC

`%APPDATA%\com.alextaxservices.alexbooks\config.json` — the server address. `%LOCALAPPDATA%\AlexBooks\` — the program. Nothing else.
