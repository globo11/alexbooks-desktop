"""A stand-in office server for the window check (ci/window-check.ps1): it answers the three addresses AlexBooks for
Windows asks before it opens an office, serves one page, and writes down what that page tells it.

    python office.py <log file> [port]

The page is in Spanish with no "do not translate" mark, has a text box, a line of text and two links (one opens a second window), and reports every
key press and right-click it is given - so the check can tell which keys reached the page and which the window kept
from the engine. Nothing here is part of the program; it never leaves the build machine.
"""
import json
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8765

PAGE = """<!doctype html>
<html lang="es"><head><meta charset="utf-8"><title>Oficina de prueba</title>
<style>
  body{margin:0;font:18px system-ui;background:#f6f4ef;color:#1f2933}
  h1{position:absolute;left:60px;top:30px;margin:0;font-size:30px}
  #texto{position:absolute;left:60px;top:110px;width:820px;margin:0;line-height:1.5}
  #caja{position:absolute;left:60px;top:260px;width:420px;height:44px;font:20px system-ui;padding:0 12px;box-sizing:border-box}
  #enlace{position:absolute;left:60px;top:330px}
  #nueva{position:absolute;left:60px;top:372px}
  #visto{position:absolute;left:60px;top:420px;width:820px;font:14px ui-monospace,Consolas,monospace;white-space:pre-wrap}
</style></head><body>
<h1>Oficina de prueba</h1>
<p id="texto">Esta es la pagina de prueba de la oficina. Aqui se revisan los clientes, los estados de cuenta y las
transacciones del mes. El contador abre cada cliente, importa el estado de cuenta del banco y revisa cada movimiento
antes de cerrar el periodo. Todos los datos se guardan en la carpeta de la oficina.</p>
<input id="caja" value="texto de prueba en la caja">
<a id="enlace" href="/otra">Abrir otra pagina de la oficina</a>
<a id="nueva" href="/otra" target="_blank">Abrir una segunda ventana de la oficina</a>
<div id="visto"></div>
<script>
  var n = 0;
  function say(m) {
    n += 1;
    document.getElementById('visto').textContent += m + '\\n';
    fetch('/log?m=' + encodeURIComponent(m)).catch(function () {});
  }
  window.addEventListener('keydown', function (e) {
    if (['Control', 'Shift', 'Alt'].indexOf(e.key) >= 0) return;
    say('key ' + (e.ctrlKey ? 'ctrl+' : '') + (e.shiftKey ? 'shift+' : '') + (e.altKey ? 'alt+' : '') + e.key.toLowerCase());
  }, true);
  window.addEventListener('contextmenu', function (e) { say('contextmenu on ' + (e.target.id || e.target.tagName.toLowerCase())); }, true);
  window.addEventListener('beforeprint', function () { say('print asked'); });
  window.addEventListener('load', function () { say('loaded ' + navigator.userAgent + ' zoom=' + window.devicePixelRatio); });
</script></body></html>"""


class H(BaseHTTPRequestHandler):
    def _send(self, code, body=b"", kind="text/plain; charset=utf-8"):
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        if u.path == "/healthz":
            return self._send(200, json.dumps({"ok": True, "app": "AlexBooks", "version": "2.6.1", "office": "Window Check"}).encode(), "application/json")
        if u.path == "/desktop/hello":
            return self._send(200, b"ok")
        if u.path == "/log":
            m = urllib.parse.parse_qs(u.query).get("m", [""])[0]
            with open(LOG, "a", encoding="utf-8") as f:
                f.write(m + "\n")
            return self._send(204)
        if u.path in ("/", "/otra"):
            return self._send(200, PAGE.encode(), "text/html; charset=utf-8")
        return self._send(404, b"not here")

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    open(LOG, "a").close()
    ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
