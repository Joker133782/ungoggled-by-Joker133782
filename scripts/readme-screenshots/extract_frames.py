#!/usr/bin/env python3
"""Decodes the built-in idle-screen videos (assets/*.h264) and saves eight
evenly spaced full-resolution frames of each as PNG, named
<video>-<0..7>.png, for mock_api.py and compose.py.

    pip install av pillow
    python extract_frames.py FRAMES_DIR
"""

import sys
from pathlib import Path

import av

ASSETS = Path(__file__).resolve().parents[2] / "assets"


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for name in ["default-loop", "default-comic", "default-loop-3d"]:
        with av.open(str(ASSETS / f"{name}.h264"), format="h264") as c:
            frames = list(c.decode(video=0))
        for k in range(8):
            frames[len(frames) * k // 8].to_image().save(out / f"{name}-{k}.png")
        print(name, len(frames), "frames")


if __name__ == "__main__":
    main()
