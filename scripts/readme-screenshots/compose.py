#!/usr/bin/env python3
"""Turns the raw screenshots from shoot.py and the idle-video frames from
extract_frames.py into the README images under docs/images/.

    python compose.py RAW_DIR FRAMES_DIR OUT_DIR
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

BG = (12, 15, 20)
PANEL = (17, 21, 29)
BORDER = (31, 39, 51)
ACCENT = (79, 209, 197)


def rounded(im, radius):
    mask = Image.new("L", im.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, im.width - 1, im.height - 1), radius, fill=255)
    out = Image.new("RGBA", im.size)
    out.paste(im.convert("RGBA"), (0, 0), mask)
    return out


def fit_width(im, w):
    return im.resize((w, round(im.height * w / im.width)), Image.LANCZOS)


def fit_height(im, h):
    return im.resize((round(im.width * h / im.height), h), Image.LANCZOS)


def glow_canvas(w, h, spots):
    """Dark canvas with soft teal light spots, like the web UI's background."""
    canvas = Image.new("RGB", (w, h), BG)
    light = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(light)
    for x, y, r, a in spots:
        d.ellipse((x - r, y - r, x + r, y + r), fill=a)
    light = light.filter(ImageFilter.GaussianBlur(min(w, h) // 6))
    teal = Image.new("RGB", (w, h), (24, 70, 72))
    return Image.composite(teal, canvas, light)


def shadow(canvas, box, radius, blur=28, strength=170):
    sh = Image.new("L", canvas.size, 0)
    x0, y0, x1, y1 = box
    ImageDraw.Draw(sh).rounded_rectangle((x0, y0 + 14, x1, y1 + 14), radius, fill=strength)
    sh = sh.filter(ImageFilter.GaussianBlur(blur))
    canvas.paste(Image.new("RGB", canvas.size, (0, 0, 0)), (0, 0), sh)


def browser(shot, width):
    """A minimal browser window: title bar with three dots, rounded corners."""
    shot = fit_width(shot.convert("RGB"), width)
    bar = 44
    win = Image.new("RGB", (width, shot.height + bar), PANEL)
    d = ImageDraw.Draw(win)
    for i, c in enumerate([(224, 82, 82), (227, 166, 69), ACCENT]):
        d.ellipse((22 + i * 26, 15, 36 + i * 26, 29), fill=c)
    d.rounded_rectangle((width // 2 - 170, 10, width // 2 + 170, 34), 12, fill=(21, 26, 34))
    win.paste(shot, (0, bar))
    win = rounded(win, 18)
    ImageDraw.Draw(win).rounded_rectangle((0, 0, width - 1, win.height - 1), 18, outline=BORDER, width=2)
    return win


def phone(shot, height):
    shot = fit_height(shot.convert("RGB"), height)
    bezel = 14
    body = Image.new("RGB", (shot.width + 2 * bezel, shot.height + 2 * bezel), (5, 7, 10))
    body.paste(rounded(shot, 34), (bezel, bezel), rounded(shot, 34))
    body = rounded(body, 46)
    ImageDraw.Draw(body).rounded_rectangle((0, 0, body.width - 1, body.height - 1), 46, outline=(46, 56, 70), width=3)
    return body


def save(im, path, quality=88):
    im = im.convert("RGB")
    if path.suffix == ".png":
        im.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).save(path, optimize=True)
    else:
        im.save(path, quality=quality, optimize=True, progressive=True)
    print(f"{path.name}: {im.size[0]}x{im.size[1]}, {path.stat().st_size // 1024} KB")


def main():
    raw, frames, out = (Path(a) for a in sys.argv[1:4])
    out.mkdir(parents=True, exist_ok=True)
    load = lambda n: Image.open(raw / n)

    # Hero: desktop browser window with a phone in front of it.
    W, H = 2000, 1180
    hero = glow_canvas(W, H, [(420, 220, 520, 120), (1700, 1000, 520, 90)])
    win = browser(load("desktop-live.png"), 1560)
    wx, wy = 70, 60
    shadow(hero, (wx, wy, wx + win.width, wy + win.height), 18)
    hero.paste(win, (wx, wy), win)
    ph = phone(load("phone-live.png"), 1040)
    px, py = W - ph.width - 70, H - ph.height - 50
    shadow(hero, (px, py, px + ph.width, py + ph.height), 46, blur=34, strength=220)
    hero.paste(ph, (px, py), ph)
    save(fit_width(hero, 1600), out / "hero-console.jpg")

    # Desktop panels.
    for name in ["hdmi-recording", "idle", "usb", "update", "graphs", "recordings"]:
        im = load(f"desktop-{name}.png").convert("RGB")
        save(fit_width(im, 1400), out / f"console-{name}.jpg", quality=86)

    # Phone strip: three phones side by side.
    for strip, names in {"phone-strip-1": ["live", "hdmi", "idle"],
                         "phone-strip-2": ["usb", "update", "graphs"]}.items():
        phones = [phone(load(f"phone-{n}.png"), 1100) for n in names]
        gap, margin = 60, 60
        w = sum(p.width for p in phones) + gap * 2 + margin * 2
        h = phones[0].height + margin * 2
        c = glow_canvas(w, h, [(w // 2, h // 2, w // 3, 80)])
        x = margin
        for p in phones:
            shadow(c, (x, margin, x + p.width, margin + p.height), 46, blur=26)
            c.paste(p, (x, margin), p)
            x += p.width + gap
        save(fit_width(c, 1400), out / f"{strip}.jpg", quality=86)

    # Idle screens: one frame from each built-in video.
    picks = ["default-loop-0.png", "default-comic-2.png", "default-loop-3d-0.png"]
    tiles = [rounded(fit_width(Image.open(frames / p).convert("RGB"), 760), 22) for p in picks]
    gap = 28
    w = sum(t.width for t in tiles) + gap * 4
    h = tiles[0].height + gap * 2
    c = Image.new("RGB", (w, h), BG)
    x = gap
    for t in tiles:
        c.paste(t, (x, gap), t)
        ImageDraw.Draw(c).rounded_rectangle((x, gap, x + t.width - 1, gap + t.height - 1), 22, outline=BORDER, width=2)
        x += t.width + gap
    save(fit_width(c, 1600), out / "idle-screens.jpg", quality=86)


if __name__ == "__main__":
    main()
