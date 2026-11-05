# What's new — AlexBooks for Windows

## 0.2.0 — Just this PC, or Network

- First run asks **How will this PC use AlexBooks?** — *Just this PC* (AlexBooks runs on this computer; other PCs can join later) or *Network* (the office has a server).
- **Just this PC** looks for AlexBooks on this computer and opens it; if it is not installed here yet, it says so and offers to try again.
- **Network** looks for servers by itself: it asks the office network and lists every AlexBooks server by office name (servers on 1.25.3 or newer); if none answers it knocks on the AlexBooks port across the office network and lists what it finds; and the address can always be typed. Nothing is pre-filled.
- Click a server in the list to connect. The window remembers the choice.

## 0.1.0 — first build

- A real program: `AlexBooks-Setup.exe` installs per user (no admin prompt), desktop and Start-menu icons, listed in Installed apps, uninstalls from there.
- First start: "Connect to your office" — type the server's address once (the IP or name is enough; the port is added). After that it opens straight into AlexBooks in its own window, no address bar, no tabs, the AlexBooks icon in the taskbar.
- Remembers the window size and position.
