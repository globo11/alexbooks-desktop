AlexBooks for Windows 0.3.0 — checks on every start, certificate trust, self-update, a menu.

- **Every start**: reaches the office server, then opens it. If it can't, a plain page says why, with *Try again* and *Connect to a different office*.
- **Trust the office server**: the first time, the program fetches the office certificate from the server and asks Windows to install it (answer *Yes* to the Windows prompt). No more certificate warnings.
- **Updates itself from the office server**: newer versions are offered on start (*Update now* / *Not now*) and installed from the server — the office PCs never need internet. Every update is checked against the project's signing key.
- **Menu**: Reload · Connect to a different office… · Check for updates… · About AlexBooks · Exit.

Needs AlexBooks 1.25.7 on the server for certificate trust and updates (older servers still open).
