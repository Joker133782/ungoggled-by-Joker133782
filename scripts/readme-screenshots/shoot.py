#!/usr/bin/env python3
"""Takes the README screenshots of the web console against mock_api.py.

Regenerate for a new release (no Pi involved, all data is fictional):

    pip install pillow websocket-client av
    python scripts/readme-screenshots/extract_frames.py FRAMES_DIR
    (build web/ with `npm ci && npm run build`)
    python scripts/readme-screenshots/mock_api.py --dist web/dist --frames FRAMES_DIR &
    python scripts/readme-screenshots/shoot.py RAW_DIR
    python scripts/readme-screenshots/compose.py RAW_DIR FRAMES_DIR docs/images

Drives headless Edge (or Chrome, set BROWSER) over the DevTools protocol.
Every request that is not for the local mock or Google Fonts is blocked;
the in-app update check is answered with "nothing newer published".
"""

import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

import websocket

BROWSER = os.environ.get(
    "BROWSER", r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
)
MOCK = os.environ.get("MOCK", "http://127.0.0.1:8765")
PUBLIC = "http://192.168.50.1"  # mapped onto MOCK by --host-resolver-rules
PORT = 9333
ALLOWED = (MOCK, PUBLIC + "/", "https://fonts.googleapis.com/", "https://fonts.gstatic.com/")


class Tab:
    def __init__(self, ws_url):
        self.ws = websocket.create_connection(ws_url, timeout=60, suppress_origin=True)
        self.n = 0

    def call(self, method, **params):
        self.n += 1
        my = self.n
        self.ws.send(json.dumps({"id": my, "method": method, "params": params}))
        while True:
            msg = json.loads(self.ws.recv())
            if msg.get("id") == my:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})
            self.event(msg)

    def event(self, msg):
        if msg.get("method") != "Fetch.requestPaused":
            return
        p = msg["params"]
        url = p["request"]["url"]
        rid = p["requestId"]
        if url.startswith(ALLOWED):
            self.send("Fetch.continueRequest", requestId=rid)
        elif url.endswith("/latest.json") or p["request"]["method"] == "OPTIONS":
            # The update check: "reachable, nothing newer published".
            cors = [
                {"name": "Access-Control-Allow-Origin", "value": "*"},
                {"name": "Access-Control-Allow-Headers", "value": "*"},
                {"name": "Access-Control-Allow-Methods", "value": "GET, HEAD"},
            ]
            code = 204 if p["request"]["method"] == "OPTIONS" else 404
            self.send("Fetch.fulfillRequest", requestId=rid, responseCode=code, responseHeaders=cors)
        else:
            self.send("Fetch.failRequest", requestId=rid, errorReason="BlockedByClient")

    def send(self, method, **params):
        self.n += 1
        self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))

    def pump(self, seconds):
        end = time.time() + seconds
        self.ws.settimeout(0.1)
        while time.time() < end:
            try:
                self.event(json.loads(self.ws.recv()))
            except websocket.WebSocketTimeoutException:
                pass
        self.ws.settimeout(60)

    def js(self, expr):
        r = self.call("Runtime.evaluate", expression=expr, returnByValue=True, awaitPromise=True)
        return r.get("result", {}).get("value")


def scene(name):
    urllib.request.urlopen(f"{MOCK}/mock/scene?name={name}").read()


# Scrolls so that the element starts just below the (sticky) header.
SCROLL_TO = """(() => {{ const e = {el}; const head = document.querySelector('header');
  const h = head && getComputedStyle(head).position !== 'static' ? head.getBoundingClientRect().height : 0;
  window.scrollTo(0, e.getBoundingClientRect().top + scrollY - h - 12); }})()"""
RECT = """(() => {{ const e = {el}; e.scrollIntoView({{block:'start'}}); const b = e.getBoundingClientRect();
  return [b.left + scrollX, b.top + scrollY, b.width, b.height]; }})()"""


def q(selector, parent=False):
    return f"document.querySelector({json.dumps(selector)})" + (".parentElement" if parent else "")


def shoot(tab, out, url, width, height, scale=2, mobile=False, clip=None, scroll=None, wait=4.0, pad=16):
    tab.call("Emulation.setDeviceMetricsOverride", width=width, height=height,
             deviceScaleFactor=scale, mobile=mobile)
    tab.call("Emulation.setTouchEmulationEnabled", enabled=mobile)
    tab.call("Page.navigate", url="about:blank")
    tab.pump(0.3)
    tab.call("Page.navigate", url=url)
    tab.pump(wait)
    params = {"format": "png"}
    if scroll:
        tab.js(SCROLL_TO.format(el=scroll))
        tab.pump(1.5)
    if clip:
        x, y, w, h = tab.js(RECT.format(el=clip))
        tab.pump(1.0)
        params["clip"] = {"x": max(0, x - pad), "y": max(0, y - pad), "width": w + 2 * pad,
                          "height": h + 2 * pad, "scale": 1}
        params["captureBeyondViewport"] = True
    data = tab.call("Page.captureScreenshot", **params)["data"]
    Path(out).write_bytes(base64.b64decode(data))
    print("saved", out)


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    profile = tempfile.mkdtemp(prefix="ungoggled-shots-")
    mock = urllib.parse.urlparse(MOCK)
    proc = subprocess.Popen([
        BROWSER, "--headless=new", f"--remote-debugging-port={PORT}",
        f"--user-data-dir={profile}", "--no-first-run", "--hide-scrollbars",
        "--disable-extensions", "--disable-sync", "--force-color-profile=srgb",
        "--lang=en-US",
        # The UI shows its own address (spectator link); make it the AP's.
        f"--host-resolver-rules=MAP 192.168.50.1:80 {mock.hostname}:{mock.port}",
        "about:blank",
    ])
    try:
        for _ in range(50):
            try:
                targets = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json").read())
                page = next(t for t in targets if t["type"] == "page")
                break
            except Exception:
                time.sleep(0.2)
        tab = Tab(page["webSocketDebuggerUrl"])
        tab.call("Fetch.enable", patterns=[{"urlPattern": "*"}])
        tab.call("Page.enable")
        tab.call("Emulation.setLocaleOverride", locale="en-US")
        tab.call("Emulation.setTimezoneOverride", timezoneId="UTC")
        tab.call("Emulation.setEmulatedMedia", features=[{"name": "prefers-color-scheme", "value": "dark"}])

        app = PUBLIC
        phone = dict(width=390, height=844, scale=3, mobile=True)
        scene("live")
        shoot(tab, out / "desktop-live.png", f"{app}/#live", 1280, 800)
        shoot(tab, out / "phone-live.png", f"{app}/#live", **phone)
        shoot(tab, out / "phone-hdmi.png", f"{app}/#settings", scroll=q(".page > .row"), **phone)
        shoot(tab, out / "phone-idle.png", f"{app}/#settings", scroll=q("#idle-screen"), **phone)
        shoot(tab, out / "phone-update.png", f"{app}/#system", scroll=q("#software-update"), **phone)
        shoot(tab, out / "phone-graphs.png", f"{app}/#system", scroll=q("#graphs"), **phone)
        shoot(tab, out / "desktop-hdmi-recording.png", f"{app}/#settings", 1280, 900, clip=q(".page > .row"))
        shoot(tab, out / "desktop-idle.png", f"{app}/#settings", 1280, 900, clip=q("#idle-screen"))
        shoot(tab, out / "desktop-update.png", f"{app}/#system", 1280, 900, clip=q("#software-update", parent=True))
        shoot(tab, out / "desktop-graphs.png", f"{app}/#system", 1280, 900, clip=q("#graphs"))
        shoot(tab, out / "desktop-recordings.png", f"{app}/#videos", 1280, 900, clip=q(".recording-files"))
        scene("usb")
        shoot(tab, out / "desktop-usb.png", f"{app}/#videos", 1280, 900, clip=q(".usb-videos"))
        shoot(tab, out / "phone-usb.png", f"{app}/#videos", **phone)
        scene("live")
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    main()
