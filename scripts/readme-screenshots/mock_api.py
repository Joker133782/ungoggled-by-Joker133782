#!/usr/bin/env python3
"""Throw-away mock of the Pi's HTTP API, for README screenshots only.

Serves a built web UI (web/dist) plus fictional but plausible JSON for every
endpoint the control interface polls, so the UI can be photographed without
a real Pi. All data here is made up: the only names, addresses and passwords
are the documented public defaults (Wi-Fi "ungoggled" / "ungoggled",
http://192.168.50.1). Nothing is written anywhere; POSTs are accepted and
ignored.

    python mock_api.py --dist web/dist --frames FRAMES_DIR [--port 8765]

FRAMES_DIR holds PNG frames from the built-in idle videos (see
extract_frames.py). GET /mock/scene?name=live|usb switches the scene.
"""

import argparse
import io
import json
import math
import mimetypes
import random
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

from PIL import Image

ARGS = None
Y = 31_536_000  # the timestamps below are 2025 ones, shifted into 2026
SCENE = {"name": "live"}
START = time.time() - 2 * 3600 - 17 * 60  # "uptime" 2h 17m
JPEG_CACHE: dict = {}

MODES = [
    "1920x1080@60", "1920x1080@50", "1920x1080@30", "1920x1080@60i",
    "1280x720@60", "1280x720@50", "720x576@50", "720x480@60",
]

# Frame files (relative to --frames) used for every picture the UI shows.
PREVIEW = "default-comic-7.png"
IDLE_THUMBS = {
    "default-loop.h264": "default-loop-0.png",
    "default-loop-3d.h264": "default-loop-3d-0.png",
    "default-comic.h264": "default-comic-2.png",
}
USB_CLIPS = [
    ("DJI_20261018_153204_0007_D.MP4", "default-comic-3.png", 1_284_000_000, Y+1760801524),
    ("DJI_20261018_154810_0008_D.MP4", "default-comic-1.png", 942_000_000, Y+1760802490),
    ("GX010231.MP4", "default-loop-3d-4.png", 2_061_000_000, Y+1760882113),
    ("Sunset cruise.mov", "default-comic-0.png", 618_000_000, Y+1760968840),
]
RECORDINGS = [
    ("flight-1794147932481207563.h264", "default-comic-2.png", 168_400_000, Y+1762611932),
    ("crash-1794147305118020917.h264", "default-comic-4.png", 17_900_000, Y+1762611305),
    ("flight-1794146718804133120.h264", "default-comic-7.png", 231_700_000, Y+1762610718),
    ("recording-1794061412907715041.h264", "default-loop-3d-2.png", 96_300_000, Y+1762525412),
]


def jpeg(frame: str, width: int) -> bytes:
    key = (frame, width)
    if key not in JPEG_CACHE:
        im = Image.open(Path(ARGS.frames) / frame).convert("RGB")
        im = im.resize((width, round(width * im.height / im.width)), Image.LANCZOS)
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=85)
        JPEG_CACHE[key] = buf.getvalue()
    return JPEG_CACHE[key]


def settings() -> dict:
    return {
        "hdmi_mode": "auto",
        "hdmi_connector": None,
        "fallback_image": "default-loop.h264",
        "preview_enabled": True,
        "beeper_pin": 6,
        "power_button_pin": 19,
        "goggles_led_pin": 17,
        "video_led_pin": 23,
        "hdmi_led_pin": 24,
        "beep_until_time_synced": False,
        "stream_enabled": True,
        "stream_quality": "high",
        "auto_record": True,
        "crash_buffer": True,
        "record_to_usb": False,
        "wifi_always_ap": False,
        "terminal_enabled": False,
        "stretch_live": False,
        "stretch_usb": True,
    }


def wave(t: float, base: float, amp: float, period: float, seed: float = 0) -> float:
    return base + amp * math.sin(t / period + seed) + amp * 0.35 * math.sin(t / (period * 0.37) + seed * 2)


def status() -> dict:
    now = time.time()
    usb = SCENE["name"] == "usb"
    s = {
        "version": "0.3.49",
        "phase": "streaming",
        "enabled": True,
        "hdmi": "playing",
        "hdmi_connected": True,
        "usb_state": "configured",
        "bitrate_mbps": round(wave(now, 4.1, 0.5, 3.0), 2),
        "input_width": 1920,
        "input_height": 1080,
        "input_fps": 59.9,
        "input_nominal_fps": 60,
        "output_fps": 59.9 if not usb else 29.9,
        "usb_speed": 1.0 if usb else None,
        "usb_fps": 29.9 if usb else None,
        "usb_clip_fps": 29.97 if usb else None,
        "hdmi_width": 1920,
        "hdmi_height": 1080,
        "hdmi_hz": 60,
        "hdmi_interlaced": False,
        "hdmi_modes": MODES,
        "temperature_c": round(wave(now, 54.0, 0.6, 20.0), 1),
        "uptime_seconds": int(now - START),
        "video_bytes": 2_871_400_000,
        "discarded_bytes": 0,
        "decoder_starts": 3,
        "message": "Video stream started",
        "beeper_error": None,
        "power_button_error": None,
        "goggles_led_error": None,
        "video_led_error": None,
        "hdmi_led_error": None,
        "time_unix_ms": int(now * 1000),
        "time": time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(now)),
        "timezone": "UTC",
        "time_synced": True,
        "fallback_video": True,
        "goggles_protocol": "goggles3",
        "goggles_protocol_state": "video",
        "disk_warning": False,
        "reboot_required": False,
    }
    return s


def history() -> dict:
    """A 30-minute window that looks like a session: two flights with a
    battery swap (no live picture) in between, and a short dropout."""
    rnd = random.Random(7)
    now = 7800
    samples = []
    for t in range(now - 1800, now):
        x = t - (now - 1800)  # 0..1799
        gap = 760 <= x < 1010 or 1450 <= x < 1462
        live = not gap
        temp = 49 + 6 * (1 - math.exp(-x / 500)) + (0.0 if live else -1.5) + rnd.uniform(-0.3, 0.3)
        samples.append({
            "t": t,
            "bitrate_mbps": round(max(0.5, wave(x, 4.2, 0.9, 25, 1) + rnd.uniform(-0.4, 0.4)), 2) if live else None,
            "input_fps": round(59.9 + rnd.uniform(-0.2, 0.1), 1) if live else None,
            "output_fps": round(59.9 + rnd.uniform(-0.3, 0.1), 1) if live else 30.0,
            "temperature_c": round(temp, 1),
            "phase": "streaming" if live else "waiting_usb",
            "hdmi": "playing" if live else "fallback",
            "cpu_percent": round(max(3, (21 if live else 9) + rnd.uniform(-4, 4)), 1),
            "ram_used_mib": round(612 + (180 if live else 40) + rnd.uniform(-8, 8)),
            "ram_total_mib": 3796,
            "disk_read_kib_s": round(rnd.uniform(0, 30), 1),
            "disk_write_kib_s": round((520 if live else 4) + rnd.uniform(-60, 60), 1) if live else round(rnd.uniform(0, 20), 1),
            "wifi_rx_mbps": round(0.05 + rnd.uniform(0, 0.08), 2),
            "wifi_tx_mbps": round((4.6 + rnd.uniform(-0.6, 0.6)) if live else 0.1 + rnd.uniform(0, 0.05), 2),
            "lan_rx_mbps": 0.0,
            "lan_tx_mbps": 0.0,
        })
    return {"instance": 1, "now": now, "samples": samples}


def images() -> dict:
    entries = []
    names = {
        "default-loop.h264": "Logo loop",
        "default-loop-3d.h264": "Logo loop 3D",
        "default-comic.h264": "FPV comic",
    }
    sizes = {"default-loop.h264": 1651750, "default-loop-3d.h264": 10665196, "default-comic.h264": 2354345}
    for vid, name in names.items():
        entries.append({
            "id": vid, "url": f"/api/images/{vid}", "thumbnail": f"/api/images/{vid}/thumbnail",
            "kind": "video", "name": name, "builtin": True, "size": sizes[vid], "modified": Y+1761000000,
            "width": 1920, "height": 1080, "fps": 30,
        })
    entries.append({
        "id": "no-signal.png", "url": "/api/images/no-signal.png", "thumbnail": "/api/images/no-signal.png/thumbnail",
        "kind": "image", "name": "No signal", "builtin": True, "size": 16013, "modified": Y+1761000000,
        "width": 1920, "height": 1080, "fps": None,
    })
    return {"images": entries, "limits": {"videos": {"used": 0, "max": 3}, "images": {"used": 0, "max": 16}}}


def releases() -> dict:
    return {
        "releases": [
            {"name": "0.3.49-5e1c0a7", "version": "0.3.49", "build": "5e1c0a7", "installed": Y+1762590000, "current": True, "factory": False},
            {"name": "0.3.48-b93f2d1", "version": "0.3.48", "build": "b93f2d1", "installed": Y+1762420000, "current": False, "factory": False},
            {"name": "0.3.46-07ad4c9", "version": "0.3.46", "build": "07ad4c9", "installed": Y+1762250000, "current": False, "factory": False},
            {"name": "0.3.43-c4e8a12", "version": "0.3.43", "build": "c4e8a12", "installed": None, "current": False, "factory": True},
        ],
        "history": [
            {"time": Y+1762590000, "action": "install", "from": "0.3.48-b93f2d1", "to": "0.3.49-5e1c0a7", "version": "0.3.49", "result": "succeeded", "message": "Updated to 0.3.49."},
            {"time": Y+1762420000, "action": "install", "from": "0.3.46-07ad4c9", "to": "0.3.48-b93f2d1", "version": "0.3.48", "result": "succeeded", "message": "Updated to 0.3.48."},
            {"time": Y+1762250000, "action": "install", "from": "0.3.43-c4e8a12", "to": "0.3.46-07ad4c9", "version": "0.3.46", "result": "succeeded", "message": "Updated to 0.3.46."},
        ],
    }


def usb_status() -> dict:
    if SCENE["name"] == "usb":
        return {"playing": True, "name": USB_CLIPS[0][0], "paused": False, "position": 74.0 + (time.time() % 5),
                "duration": 312.0, "drives": ["USB Flash Drive"], "scan": None}
    return {"playing": False, "drives": ["USB Flash Drive"], "scan": None}


ROUTES = {
    "/api/auth": lambda: {"configured": True, "authenticated": True},
    "/api/status": status,
    "/api/settings": settings,
    "/api/images": images,
    "/api/hdmi-connectors": lambda: [{"id": 0, "name": "HDMI-A-1", "connected": True}, {"id": 1, "name": "HDMI-A-2", "connected": False}],
    "/api/history": history,
    "/api/recording": lambda: {"recording": False},
    "/api/recordings": lambda: {"recordings": [
        {"name": n, "size": s, "modified": m, "thumbnail": True} for n, _f, s, m in RECORDINGS]},
    "/api/usb-videos": lambda: {"videos": [
        {"id": f"v{i}", "name": n, "device": "USB Flash Drive", "size": s, "mtime": m, "thumbnail": True}
        for i, (n, _f, s, m) in enumerate(USB_CLIPS)]},
    "/api/usb-videos/status": usb_status,
    "/api/wifi": lambda: {"available": True, "ssid": "ungoggled", "applying": False},
    "/api/wifi/client": lambda: {"active": False},
    "/api/wifi/share": lambda: {"mode": "ap", "ssid": "ungoggled", "security": "WPA", "password": "ungoggled"},
    "/api/wifi/scan": lambda: {"networks": []},
    "/api/terminal": lambda: {"enabled": False, "active": False, "idle_seconds": 0, "max_seconds": 1800},
    "/api/update": lambda: {"available": True, "current_version": "0.3.49", "current_build": "5e1c0a7"},
    "/api/update/releases": releases,
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, code, body: bytes, ctype: str):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def json(self, obj, code=200):
        self.send(code, json.dumps(obj).encode(), "application/json")

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length:
            self.rfile.read(length)
        self.json({"ok": True, "message": "ok"})

    do_DELETE = do_POST

    def do_GET(self):
        url = urlparse(self.path)
        path = unquote(url.path)
        if path == "/mock/scene":
            SCENE["name"] = parse_qs(url.query).get("name", ["live"])[0]
            return self.json(SCENE)
        if path in ROUTES:
            return self.json(ROUTES[path]())
        if path == "/api/preview.jpg":
            return self.send(200, jpeg(PREVIEW, 640), "image/jpeg")
        if path.startswith("/api/images/"):
            ident = path.split("/")[3]
            if ident == "no-signal.png":
                return self.send(200, jpeg_file(Path(ARGS.assets) / "no-signal.png", 320), "image/jpeg")
            if ident in IDLE_THUMBS:
                return self.send(200, jpeg(IDLE_THUMBS[ident], 320), "image/jpeg")
        if path.startswith("/api/usb-videos/v") and path.endswith("/thumbnail"):
            i = int(path.split("/")[3][1:])
            return self.send(200, jpeg(USB_CLIPS[i][1], 320), "image/jpeg")
        if path.startswith("/api/recordings/") and path.endswith("/thumbnail"):
            name = path.split("/")[3]
            for n, f, _s, _m in RECORDINGS:
                if n == name:
                    return self.send(200, jpeg(f, 320), "image/jpeg")
        if path.startswith("/api/"):
            return self.json({"error": "not mocked"}, 404)
        # Static files from the built web UI.
        rel = "index.html" if path in ("/", "") else path.lstrip("/")
        file = (Path(ARGS.dist) / rel).resolve()
        if Path(ARGS.dist).resolve() in file.parents and file.is_file():
            ctype = mimetypes.guess_type(file.name)[0] or "application/octet-stream"
            return self.send(200, file.read_bytes(), ctype)
        self.send(404, b"not found", "text/plain")


def jpeg_file(path: Path, width: int) -> bytes:
    key = (str(path), width)
    if key not in JPEG_CACHE:
        im = Image.open(path).convert("RGB")
        im = im.resize((width, round(width * im.height / im.width)), Image.LANCZOS)
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=85)
        JPEG_CACHE[key] = buf.getvalue()
    return JPEG_CACHE[key]


def main():
    global ARGS
    p = argparse.ArgumentParser()
    p.add_argument("--dist", required=True)
    p.add_argument("--frames", required=True)
    p.add_argument("--assets", default=str(Path(__file__).resolve().parents[2] / "assets"))
    p.add_argument("--port", type=int, default=8765)
    ARGS = p.parse_args()
    mimetypes.add_type("application/javascript", ".js")
    mimetypes.add_type("image/svg+xml", ".svg")
    server = ThreadingHTTPServer(("127.0.0.1", ARGS.port), Handler)
    print(f"mock on http://127.0.0.1:{ARGS.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
