<p align="center">
  <img src="docs/images/banner.svg" alt="ungoggled: DJI goggles video on any HDMI screen, with a Raspberry Pi 4" width="100%">
</p>

<p align="center">
  <b>Turn a Raspberry Pi 4 into an HDMI receiver for DJI goggles</b> — so everyone at the field can watch the flight on a TV, set up from your phone.
</p>

<p align="center">
  <a href="https://github.com/Joker133782/ungoggled-by-Joker133782/releases/latest"><img src="docs/images/badges/release.svg" alt="Release v0.3.52"></a>
  <a href="LICENSE"><img src="docs/images/badges/license.svg" alt="MIT license"></a>
  <img src="docs/images/badges/platform.svg" alt="Raspberry Pi 4, 64-bit">
  <img src="docs/images/badges/stack.svg" alt="Rust and React">
</p>

## ⬇ Download

<p align="center">
  <a href="https://github.com/Joker133782/ungoggled-by-Joker133782/releases/download/v0.3.52/ungoggled-0.3.52-pi4-arm64.img.xz"><img src="docs/images/download-flash-image.svg" alt="Download flash image: v0.3.52, 684 MB, for a new SD card" width="400"></a>
  &nbsp;
  <a href="https://github.com/Joker133782/ungoggled-by-Joker133782/releases/download/v0.3.52/ungoggled-0.3.52.update.tar.gz"><img src="docs/images/download-update.svg" alt="Download update: v0.3.52, 18 MB, for a Pi that already runs ungoggled" width="400"></a>
</p>

<p align="center">
  <b>Which one?</b> First time → <b>flash image</b> (write it to a microSD card). Already running ungoggled → <b>update</b> (upload it in the web console).<br>
  <a href="DOWNLOAD.md">Step-by-step guide &amp; checksums</a> ·
  <a href="https://github.com/Joker133782/ungoggled-by-Joker133782/releases/tag/v0.3.52">Release notes v0.3.52</a> ·
  <a href="https://github.com/Joker133782/ungoggled-by-Joker133782/releases">All versions</a>
</p>

<p align="center">
  <img src="docs/images/hero-console.jpg" alt="The ungoggled web console on a laptop and on a phone, showing live video status and the preview" width="100%">
</p>

## What is it?

DJI goggles can send their live video out over USB-C. **ungoggled** receives that
stream on a Raspberry Pi 4, decodes it in hardware and puts it on any HDMI screen —
a TV, a monitor or a capture card. The Pi opens its own Wi-Fi, and a web console on
your phone shows what is going on and lets you set everything up. No app, no cloud,
no internet needed at the field.

<p align="center">
  <img src="docs/images/how-it-works.svg" alt="Goggles connect to the Pi over USB-C, the Pi drives an HDMI screen, and your phone joins the Pi's Wi-Fi for the web console" width="100%">
</p>

## Features

<table>
<tr><td width="56" align="center"><img src="docs/images/icons/hdmi.svg" width="40" alt=""></td><td><b>Live HDMI output</b><br>Hardware-decoded, automatic 1080p, or any mode your screen offers (incl. interlaced).</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/preview.svg" width="40" alt=""></td><td><b>Web console, phone first</b><br>Status, live preview and every setting from your phone. Password-protected, installable as an app over HTTPS.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/idle.svg" width="40" alt=""></td><td><b>Idle screens</b><br>Three built-in animated waiting screens, or your own image or looping video.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/usb.svg" width="40" alt=""></td><td><b>USB action-cam player <sup>experimental</sup></b><br>Play clips from a USB drive or action camera on the TV, with stretch-to-fill.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/record.svg" width="40" alt=""></td><td><b>Recording <sup>experimental</sup></b><br>Auto-record every flight, a 30-second crash buffer, or record by hand, without re-encoding.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/spectators.svg" width="40" alt=""></td><td><b>Spectator stream <sup>experimental</sup></b><br>Bystanders watch in their phone's browser on the Pi's Wi-Fi, with QR codes to join.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/update.svg" width="40" alt=""></td><td><b>Safe updates</b><br>Update from the browser; settings are kept and a failed update rolls back by itself.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/wifi.svg" width="40" alt=""></td><td><b>Own Wi-Fi</b><br>5 GHz access point out of the box, or join an existing network.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/beeper.svg" width="40" alt=""></td><td><b>Beeper, LEDs, power button</b><br>Optional GPIO buzzer tones, three status LEDs and a shutdown button.</td></tr>
<tr><td width="56" align="center"><img src="docs/images/icons/graphs.svg" width="40" alt=""></td><td><b>Last 30 minutes</b><br>Bitrate, frame rate, temperature, CPU, RAM, Wi-Fi and SD-card graphs.</td></tr>
</table>

## Screenshots

<p align="center">
  <img src="docs/images/phone-strip-1.jpg" alt="Phone: live dashboard, HDMI settings with the two stretch switches, idle screen picker" width="100%">
</p>

<details>
<summary><b>More screenshots</b> (USB player, recordings, updates, graphs)</summary>
<br>

<p align="center"><img src="docs/images/phone-strip-2.jpg" alt="Phone: USB video player, software update, last 30 minutes graphs" width="100%"></p>

<p align="center"><img src="docs/images/console-hdmi-recording.jpg" alt="Settings: HDMI output mode and stretch switches, recording switches" width="100%"><br><sub><b>Settings → HDMI and Recording</b></sub></p>
<p align="center"><img src="docs/images/console-idle.jpg" alt="Settings: idle screen picker with the built-in videos" width="100%"><br><sub><b>Settings → Idle screen</b></sub></p>
<p align="center"><img src="docs/images/console-usb.jpg" alt="Videos: USB action-cam player playing a clip on the HDMI output" width="100%"><br><sub><b>Videos → USB action-cam player</b></sub></p>
<p align="center"><img src="docs/images/console-recordings.jpg" alt="Videos: saved flights and crash clips" width="100%"><br><sub><b>Videos → Recordings</b></sub></p>
<p align="center"><img src="docs/images/console-update.jpg" alt="System: software update and installed versions" width="100%"><br><sub><b>System → Software update and versions</b></sub></p>
<p align="center"><img src="docs/images/console-graphs.jpg" alt="System: last 30 minutes graphs" width="100%"><br><sub><b>System → Last 30 minutes</b></sub></p>

<sub>Screenshots use made-up demo data.</sub>
</details>

## Idle screens

What the TV shows while there is no live picture — before the goggles connect, or
when the signal drops. Three animated screens are built in (*Logo loop*, *FPV comic*,
*Logo loop 3D*); you can also upload your own image or video.

<p align="center">
  <img src="docs/images/idle-screens.jpg" alt="The three built-in idle screens: Logo loop, FPV comic, Logo loop 3D" width="100%">
</p>

## Supported goggles

| Goggles | Air unit | Status |
|---|---|---|
| **DJI Goggles 3** | O3 Air Unit, O4 Air Unit Pro | ✅ Tested with live HDMI output |
| DJI Goggles 2 | — | 🧪 Experimental; only the Goggles 3 path is hardware-verified |
| DJI FPV Goggles V1/V2 | — | 🧪 Experimental, unverified: has not run against real V1/V2 hardware |

Other systems (for example Walksnail or DJI Goggles N3) have not been tested. Details: [docs/GOGGLES.md](docs/GOGGLES.md).

## Quick start

You need a **Raspberry Pi 4**, a microSD card (8 GB or more), a USB-C data cable,
a micro-HDMI cable, the goggles with a paired air unit, and a separate power supply
for the Pi.

1. **Flash** the [image](DOWNLOAD.md) to the microSD card (Raspberry Pi Imager or
   Caligula; skip any OS customisation) and put it in the Pi.
2. **Wire it up:** goggles USB-C → **Pi USB-C** port, screen → **HDMI0** (next to
   the USB-C port). Power the Pi separately (GPIO supply or PoE), then switch on.
3. **Join the Wi-Fi** `ungoggled`, password `ungoggled`.
4. **Open http://192.168.50.1** and choose a password for the web console.

For clean video without the goggles' on-screen display, set **Settings → Camera →
Advanced Camera Settings → Camera View Recording → OFF** in the goggles.
SSH login on a fresh image: `root` / `ungoggled` — see
[DOWNLOAD.md](DOWNLOAD.md#default-credentials) for all defaults.

## Documentation

| Page | What's inside |
|---|---|
| [**DOWNLOAD.md**](DOWNLOAD.md) | Which file you need, flashing, updating, checksums |
| [**docs/INSTALL.md**](docs/INSTALL.md) | Hardware and wiring, SD-card image, updates and version switching, manual install on Raspberry Pi OS |
| [**docs/USER-GUIDE.md**](docs/USER-GUIDE.md) | The web console: HDMI, idle screen, preview, Wi-Fi, HTTPS, USB player, recording, spectator stream, beeper and LEDs |
| [**docs/GOGGLES.md**](docs/GOGGLES.md) | Goggles 3, Goggles 2 and V1/V2 details and settings |
| [**docs/DEVELOPMENT.md**](docs/DEVELOPMENT.md) | Building from source, releases, services and files |
| [**CHANGELOG.md**](CHANGELOG.md) | What changed in each version |

## Credits and license

*ungoggled by Joker133782* is a fork of
[TimoLehnertz/ungoggled](https://github.com/TimoLehnertz/ungoggled), licensed under
the MIT license (see [LICENSE](LICENSE), Copyright (c) 2026 Timo Lehnertz).
Protocol research credits and third-party licenses are in
[THIRD_PARTY.md](THIRD_PARTY.md); the built-in idle-screen videos are original to
this project and MIT licensed.

Independent project, not affiliated with DJI or Cosmostreamer.
