#!/usr/bin/env python3
# Dev/test tool, run on the Pi as root. Plays a stand-in for the goggles: sends an H.264 file to the app's video socket (8-byte LE sequence
# number + up to 32 KiB of stream per datagram), paced to about real time, and reports which planes
# the display uses while it plays. usage: feed.py FILE SECONDS_OF_FILE [REPEAT]
import glob, json, socket, struct, subprocess, sys, time

SOCK = "/run/dji-hdmi/video.sock"
rep = int(sys.argv[3]) if len(sys.argv) > 3 else 1
data = open(sys.argv[1], "rb").read() * rep
dur = float(sys.argv[2]) * rep
rate = len(data) / dur


def planes():
    out = []
    for f in glob.glob("/sys/kernel/debug/dri/*/state"):
        cur, fmt = None, ""
        for line in open(f).read().splitlines():
            if line.startswith("plane["):
                cur, fmt = line.split(":")[0], ""
            elif "format=" in line:
                fmt = line.strip().split("format=")[1].split()[0]
            elif "crtc-pos=" in line and not line.split("crtc-pos=")[1].startswith("0x0"):
                out.append("%s fmt=%s pos=%s" % (cur, fmt, line.split("crtc-pos=")[1].strip()))
    return sorted(set(out))


def status():
    try:
        raw = subprocess.run(["curl", "-sLk", "--max-time", "2", "http://127.0.0.1/api/status"],
                             capture_output=True, text=True).stdout
        s = json.loads(raw)
        return {k: s.get(k) for k in ("hdmi", "input_width", "input_height", "decoder_starts", "output_fps")}
    except Exception as e:
        return {"err": str(e)}


s = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
seq = pos = 0
t0 = time.time()
shots = [5.0, 11.0]
while pos < len(data):
    chunk = data[pos:pos + 32000]
    s.sendto(struct.pack("<Q", seq) + chunk, SOCK)
    seq += 1
    pos += len(chunk)
    wait = t0 + pos / rate - time.time()
    if wait > 0:
        time.sleep(wait)
    if shots and time.time() - t0 >= shots[0]:
        shots.pop(0)
        print("t=%.1fs" % (time.time() - t0), status(), planes(), flush=True)
print("sent", pos, "bytes in %.1fs" % (time.time() - t0))
time.sleep(2.5)
print("after:", status(), planes())
