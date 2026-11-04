First build of AlexBooks for Windows (step 1 of the desktop shell).

- Installs per user (no administrator prompt), puts AlexBooks on the desktop and in the Start menu, shows in "Installed apps".
- First start asks for the office server's address once; every start after that opens AlexBooks in its own window.
- Remembers the window's size and position.

Not yet: the splash checks, finding the server on the network by itself, built-in certificate trust, self-update. Those are the next steps. On a PC that has not run the AlexBooks one-click install (which trusts the office certificate), the window shows a certificate warning page.
