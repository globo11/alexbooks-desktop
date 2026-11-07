# What's new — AlexBooks for Windows

## 1.3.3 — on the server PC itself

- "Could not connect to localhost" on the very PC that runs the office: Windows answered "localhost" with an IPv6 address first and the program gave up where a browser quietly tries IPv4 next. It now goes to 127.0.0.1 for localhost.
- Right after a package install, the window waits for the new server to come up (up to a minute) instead of reporting it unreachable while it is still starting.

## 1.3.2 — when the package is missing

- Set up a new office without a package on the PC says where it is: the attachment at the very bottom of the AlexBooks email — click it, come back, and it is picked up by itself (the screen looks again every time it opens).

## 1.3.1 — packages that find themselves, and die on time

- At a first start, a package just downloaded to Downloads or the Desktop is picked up by itself: click the attachment first or the installer first, either way AlexBooks lands on Set up a new office with the file in hand.
- A package older than a week (AlexBooks 2.2.7+ writes the date inside) installs nothing: "This package expired — ask the main office for a new one."

## 1.3.0 — double-click the package

- The `.abp` package from the new-office email now opens with a double-click: AlexBooks for Windows registers the file type when it installs, and opening the file lands straight on **Set up a new office** with the file already picked — only the passphrase to type. Works whether AlexBooks is closed or already open.

## 1.2.1 — the new icon

- The AlexBooks icon everywhere: the window, the taskbar, the Start menu, the installer and the start page.
- Messages now point to Settings → Updates, where AlexBooks for Windows lives in the office app since AlexBooks 2.2.1.

## 1.2.0 — updates install by themselves

- A newer AlexBooks for Windows no longer asks "Update now?". When this PC starts AlexBooks and the office server has a newer version, it shows **Updating…** with a turning ring, downloads it, restarts by itself, and opens the office. Nothing to click.
- While AlexBooks stays open, a newer version is no longer pushed into the middle of your work: a small notice says it installs at the next start (once per version).
- **Restart AlexBooks to update now** on Settings → Install on a PC does it right away from inside the app.

## 1.1.0 — set up a new office from an emailed package

- **Set up a new office** on the first screen: pick the `.abp` package file the main office emailed, type the passphrase from the same email, and the installer inside runs on this PC (Windows asks for permission once). When it finishes, this window opens the new office.

## 1.0.2 — no flashing windows

- Opening AlexBooks while it is already open flashed a black command window several times before the open window came forward. The helper programs it asks (is the other window running? bring it up) now run invisibly, and the open window is called to the front at once instead of after a wait.
- The same fix for opening a downloaded PDF, a link in the PC's browser, and closing a stuck window: no black window flashes.

## 1.0.1 — one window per PC

- **Opening AlexBooks again brings the open window to the front** instead of opening a second one. A note in the window says so.
- **A stuck window can be closed from the icon.** If AlexBooks is open but not responding — frozen, or hidden off every screen — the next double-click asks whether to close it and open a fresh window. Nothing is lost: the books are on the office server, and a form in progress comes back from its draft.
- **AlexBooks menu → Restart AlexBooks** for a window that is visible but misbehaving.

## 1.0.0 — AlexBooks for Windows

- The AlexBooks program for the office PCs: one icon, opens the office's AlexBooks server, keeps itself up to date from that server, and shows downloads, PDFs and new windows the way a browser would.
