# What's new — AlexBooks for Windows

## 0.4.0 — downloads you can see, links that open, drag-and-drop, back and zoom

- **Downloads show themselves.** A browser has a download bar; this window had nothing, so a PDF or an office package landed in Downloads in silence and a click looked like it did nothing (and three clicks made three copies). Now a *Downloading…* note appears, then *Saved to Downloads: name* — and the file is shown: a PDF opens in the PC's PDF viewer, anything else (a package zip, a spreadsheet, a backup) is selected in an Explorer window. Same name again gets *(2)*, *(3)*, like a browser.
- **Open in a new tab works.** Links the app opens in a new tab (a statement's original file, *Open PDF* on the statement reader) opened nowhere; they now open in a second AlexBooks window.
- **Outside links go to the PC's browser.** GitHub, a bank's website, a mailto: link — Edge or Chrome opens them; the office window stays on the office.
- **Drag-and-drop from Explorer works.** Dropping a statement PDF onto an import box did nothing in the window (the shell was swallowing the drop); the page's drop zones now receive files like they do in a browser.
- **Back, Forward, Reload and zoom.** Alt+Left / Alt+Right (also under the AlexBooks menu), F5, and Ctrl + / Ctrl − / Ctrl 0 to zoom, as in a browser. The menu also has *Open the Downloads folder*.
- The program identifies itself to the server as *AlexBooksDesktop/0.4.0*, so the app can tell when it runs inside the window.

## 0.3.1 — says hello, checks daily

- On every start the program tells its office server its PC name and version, so Settings → Other PCs on the server lists every PC that opens it (and the main office sees the list).
- A window left open for days now looks for a newer launcher once a day and shows the update screen when there is one (Update now / Not now).
- The launcher repository is public now — the program holds no secrets, and office servers fetch it with no token.

## 0.3.0 — checks on every start, certificate trust, self-update, a menu

- **Every start** opens a short start page: *Opening <office>…* → reaches the server → opens it. If the server is off or on another network, a plain **Can't reach the office server** page says why, with *Try again* and *Connect to a different office* — no browser error pages.
- **Trust the office server**: the first time a PC opens a server it does not trust yet, the program fetches the office certificate from the server and asks Windows to install it (Windows shows its own *Do you want to install this certificate?* prompt — answer Yes). No more certificate warnings, and no install file to run.
- **Updates itself from the office server**: on every start it asks *its server* for a newer AlexBooks for Windows (the server fetches releases from GitHub under Settings → Other PCs, AlexBooks 1.25.7+). When there is one: *AlexBooks for Windows x.y.z is ready* with the notes, **Update now** / *Not now*; the download comes from the server, the installer runs by itself and the program reopens. Every update is checked against the project's signing key before it runs.
- **Menu** (AlexBooks): Reload (F5) · Connect to a different office… · Check for updates… · About AlexBooks · Exit. The window title shows the office name.
- Needs AlexBooks 1.25.7 on the server for certificate trust and updates (older servers still open; the checks just skip).

## 0.2.0 — Just this PC, or Network

- First run asks **How will this PC use AlexBooks?** — *Just this PC* (AlexBooks runs on this computer; other PCs can join later) or *Network* (the office has a server).
- **Just this PC** looks for AlexBooks on this computer and opens it; if it is not installed here yet, it says so and offers to try again.
- **Network** looks for servers by itself: it asks the office network and lists every AlexBooks server by office name (servers on 1.25.3 or newer); if none answers it knocks on the AlexBooks port across the office network and lists what it finds; and the address can always be typed. Nothing is pre-filled.
- Click a server in the list to connect. The window remembers the choice.

## 0.1.0 — first build

- A real program: `AlexBooks-Setup.exe` installs per user (no admin prompt), desktop and Start-menu icons, listed in Installed apps, uninstalls from there.
- First start: "Connect to your office" — type the server's address once (the IP or name is enough; the port is added). After that it opens straight into AlexBooks in its own window, no address bar, no tabs, the AlexBooks icon in the taskbar.
- Remembers the window size and position.
