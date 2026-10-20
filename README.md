# ungoggled

Turn a Raspberry Pi 4 into an HDMI receiver for **DJI Goggles 3** (and,
experimentally, **DJI Goggles 2**), with a local web interface for setup and
monitoring.

Goggles 3 with both **O3 Air Unit** and **O4 Air Unit Pro** have been tested with
live HDMI output. Use the Pi's USB-C port for the goggles and power the Pi
separately.

## Features

- Hardware-decoded video over HDMI, with automatic 1080p mode selection.
- Configurable HDMI resolution, refresh rate and progressive or interlaced scan from the display's supported modes.
- A selectable fallback image when video is lost or the receiver is stopped.
- Optional looped fallback video (MP4 with H.264, up to 1080p and 64 MiB), demuxed
  on upload without re-encoding and hardware-decoded like the live feed.
- Browser preview at up to **640 × 360, 5 fps**, independently switchable.
- Incoming resolution, measured camera fps, HDMI refresh rate and rendered fps.
- Raspberry Pi temperature and a **30-minute history**, sampled once per second
  and held in RAM.
- Wi-Fi access-point SSID and password controls.
- Automatic startup and recovery after USB disconnects or stream changes.
- Web updates with settings retention, startup checks and automatic rollback.
- New-release notices with GitHub release notes and download links.
- Distinct happy/sad beeps from an optional piezo buzzer when the goggles, video or
  HDMI display connect or disconnect, and once at startup.
- Air-unit recording timestamps from the Pi clock, with the Pi clock itself set from
  the browser when they drift.
- Three optional plain GPIO LEDs, one each for goggles, video and HDMI state.
- An optional GPIO power button that shuts the Pi down when pressed.
- Reboot and shut down the Pi from the web interface, with a confirmation step.
- Experimental: browse and play back video files from a USB drive or action camera on the HDMI output. See **USB videos** below.

## Goggles 2 (experimental)

Goggles 2 need a different start sequence than Goggles 3: DJI Fly's arming
requests and keepalives on control port `0x7530`, taken from the MIT-licensed
[dji-o3-usb-decoder](https://github.com/dr00min/o3-usb-decoder) capture. The
receiver remembers which protocol produced video for each pair of goggles,
identified by the AOA strings they send while connecting
(`/var/lib/dji-hdmi/goggles-protocol.json`). Known goggles start with their
protocol; unknown goggles, or identifications seen with both generations, start
with the quick Goggles 3 probe and switch after 8 s without video (Goggles 2
after 12 s). The active protocol is shown under **Diagnostics → Goggles
protocol**. Only the Goggles 3 path is hardware-verified in this project.

To force a protocol or the accessory PID, add to `/etc/default/dji-hdmi` and
restart `dji-hdmi`:

```
UNGOGGLED_GOGGLES=goggles2        # auto (default), goggles3 or goggles2
UNGOGGLED_ACCESSORY_PID=11521     # 0x2d01, the PID DJI Fly presents; default 11520 (0x2d00)
```

## Goggles V1/V2 (experimental, unverified)

The original DJI FPV Goggles (V1/V2) work the other way around from Goggles
2/3: the goggles expose a plain USB bulk interface and something else has to
be the USB *host* (Goggles 2/3 make the Pi the accessory instead). This is
based on the community project [voc-poc](https://github.com/fpv-wtf/voc-poc),
which itself calls the exact stream framing "currently largely unknown" and
only claims it "sort of just works" once piped into ffmpeg. **This path has
not run against real V1/V2 hardware and may not produce a usable picture.**

Plug the goggles into one of the Pi's regular USB-A ports (not the USB-C
gadget port used for Goggles 2/3) with a data-capable cable, then set in
`/etc/default/dji-hdmi` and restart `dji-hdmi`:

```
UNGOGGLED_GOGGLES=v1v2
```

## USB videos (experimental)

Plug a USB drive or an action camera acting as USB mass storage into one of
the Pi's regular USB-A ports (never the USB-C gadget port the goggles use).
It is mounted read-only automatically; video files found on it (`.mp4`,
`.mov`, `.mkv`, `.avi`, `.m4v`, `.insv`, `.lrv`) are listed in the web
interface under **USB videos**, each with a cached thumbnail (a single
extracted frame; the cache lives under the runtime directory, which is
tmpfs, so it is gone again after any reboot or power loss, not left on the
SD card). Playing one takes over the HDMI output from live goggles video and
the fallback loop; play/pause and seeking are controlled from the web
interface, but only the picture goes to the monitor, nothing streams back to
the browser. Unlike the fallback video, there is no size limit and no
re-encoding.

This plays through **mpv**, not GStreamer, so it needs `mpv` installed on
the Pi. It is included in the SD-card image. An update installed from the
web interface also carries mpv's `.deb` files and installs them offline if
missing, so a Pi running an older release picks it up automatically on its
next update, without needing internet access on the Pi itself; that is a
one-time, best-effort step that never affects the rest of the update (see
`scripts/bundle-mpv.sh`). If it still could not install for some reason, the
web interface reports that mpv is missing instead of failing silently.
Playback otherwise uses whatever codec and resolution the file needs and
whatever the Pi's mpv/ffmpeg install can decode for it, so very high
resolutions or HEVC clips may not play smoothly without working hardware
decode. Not hardware-verified.

## Recording (experimental)

Records the live goggles feed to the SD card exactly as received (no
re-encoding), started and stopped from the web interface under
**Recording**. Stops itself, and refuses to start, with less than 2 GB of
free space left on the SD card; the HDMI output then shows a "SD card
full" notice once the goggles feed itself stops (it never interrupts a
live picture just to show that). A separate, always-on ~30-second ring
buffer lets **Save last 30 seconds** capture a moment you did not plan to
record, without having had recording running.

Recordings and saved clips are listed in the web interface for download,
deletion, or playback on the HDMI output. The goggles' own on-screen
display (battery, channel, signal bars) is part of the picture whenever
the goggles themselves are configured to add it to the output, so it is
part of a recording the same way; that is a setting on the goggles/DJI
Fly, not something the Pi controls. Not hardware-verified.

## Hardware and wiring

You need a Raspberry Pi 4, microSD card of at least 8 GB, a USB-C data cable,
a micro-HDMI cable, Goggles 3, and a paired O3 or O4 Pro air unit.

1. Power the Pi separately through a suitable GPIO supply or PoE arrangement.
2. Connect **Goggles USB-C → Pi USB-C**. The Pi's rectangular USB-A ports do not
   provide the USB role used by this receiver.
3. Connect your display to **HDMI0**, nearest the Pi's USB-C port.
4. Confirm a live camera image inside the goggles.

For clean video, turn **Settings → Camera → Advanced Camera Settings → Camera
View Recording → OFF** in the goggles. Enable it to include their overlays.
The receiver cannot remove overlays already embedded in the incoming image.

## Install the SD-card image

The image is based on **Raspberry Pi OS Lite 64-bit, Debian 13 Trixie**.

1. Download `ungoggled-0.3.1-pi4-arm64.img.xz` from the release files.
2. Burn it to the microSD card. Recommended writers: Caligula (on linux), Raspberry Pi Imager (on windows)
3. Skip any OS customization the tool offers: the image already contains its
   network and login setup.
4. Verify the card, insert it into the Pi, and power on.
5. Join **ungoggled** using the password **ungoggled**.
6. Open **http://192.168.50.1**.

### Default credentials

| Setting | Default |
| --- | --- |
| Wi-Fi SSID | `ungoggled` |
| Wi-Fi password | `ungoggled` |
| SSH username | `root` |
| SSH password | `ungoggled` |
| Web interface | `http://192.168.50.1` |

Connect with `ssh root@192.168.50.1`. Change Wi-Fi credentials in the web
interface and the root password with `passwd` if desired. Application updates
preserve existing passwords; these defaults apply to freshly flashed images.

Writing the image puts about 3 GiB on the card. The root partition then
expands to fill it on first boot, and each device generates its own
SSH host keys. The image's Wi-Fi radio country is
**DE**; configure the correct country if using it elsewhere.

> Version 0.3.0 fixes two fresh-image issues found in 0.2.0: disabled Wi-Fi and
> V4L2 decoder rejection of the goggles’ H.264 level metadata. Live O3 decoding
> at 1080p30 and O4 captured-video decoding have been checked on Trixie. See
> [validation notes](FINDINGS.MD) for hardware test details.

## Use the web interface

**Start/stop** controls USB reception. **Reconnect goggles** restarts the USB
session. The display controller stays running so it can show the fallback image.

**Fallback image:** upload PNG, JPEG or WebP (up to 12 MiB), then select a
thumbnail. Images are converted to PNG, limited to 1920 × 1080 and displayed with
preserved aspect ratio. Up to 16 images can be stored. The selection survives
reboots; a “No signal” image is included.

**HDMI mode:** automatic mode prefers progressive 1080p at up to 60 Hz. You can
choose any other mode the display advertises, including interlaced modes such as
1080i for equipment that requires them. Interlaced modes are listed by field
rate, so 1080i60 carries 30 full frames per second. Video is scaled to the
selected resolution. Applying a mode briefly interrupts output. A 60 Hz HDMI signal may carry a 30 fps camera feed: the UI shows those
rates separately. It does not increase the camera frame rate.

**Preview:** this is a small live JPEG preview for framing and monitoring, not a
broadcast stream. Preview does not change the HDMI mode. Disable preview to
reduce processing and network traffic.

**History:** choose bitrate, frame rate or temperature, and hover over the chart
to inspect a sample. Shading marks periods without live video. History is cleared
when the application restarts; it does not write continuously to the SD card.

**Wi-Fi:** enter the SSID and a new password, then apply. Reconnect your computer
using those credentials. HDMI reception continues during the network change.
The web interface is intended for your private local network and has no separate
login; do not expose port 80 to the Internet.

**Beeper, LEDs & power button:** all optional. Goggles, video and HDMI each get a
distinct happy tone when they connect and a distinct sad tone when they disconnect,
plus one chime at startup. Defaults are piezo **GPIO6**, shutdown button **GPIO19**
(NuclearHazard V5 hat, shuts the Pi down when pressed), goggles LED **GPIO17**,
video LED **GPIO23** and HDMI LED **GPIO24**. Once applied, **Test buzzer** plays a
short beep to confirm the wiring. The three status LEDs are plain (non-addressable)
LEDs, each wired to its own GPIO with a series resistor; each simply lights up
while its own state (goggles attached, video live, HDMI connected) is true, and
turns off otherwise — no special driver needed. **Beep every 5 seconds until the
clock is synced** (off by default) keeps chirping after boot until a browser has
opened the web interface (or NTP has synced the clock), so you don't record with a
wrong date.

**System:** reboot or shut down the Pi from the web interface; both ask for
confirmation first.

## Update without reflashing

From version **0.3.0**, open **Software update** in the web interface:

1. Download `ungoggled-VERSION.update.tar.gz` from [GitHub releases](https://github.com/TimoLehnertz/ungoggled/releases).
2. Click **Upload update** and choose the file. The Pi checks its contents and checksums.
3. Review the version and click **Install**. Keep the Pi powered while it updates.
4. The interface reconnects and reloads when installation finishes.

Only **Raspberry Pi 4 with 64-bit Raspberry Pi OS (ARM64)** is supported.
The update contains the entire application: you can skip intermediate application versions. Only install update
files from releases you trust; checksums detect corruption, not publisher identity.

Compatible HDMI, preview and fallback settings are retained, as are uploaded
images, Wi-Fi configuration and passwords. Incompatible application settings use
defaults and are listed after installation. Video briefly stops and the RAM
history resets. If the new application fails its startup check, the previous
application and settings are restored. A recovery service handles interrupted
installations on the next boot. Keep at least **640 MiB** free on the root partition.

The browser checks GitHub hourly and shows a notice for a newer stable release.
Click it for release notes and a download link. This uses your browser's Internet
connection; the Pi itself can stay offline. Uploads work without Internet access.
Application updates do not replace Raspberry Pi OS, its kernel or media packages.
Downgrades are not supported.

### Switching back to an earlier version

**Installed versions** lists the releases kept on the card: the active one, the
one before it, one older update and the release from the SD-card image. Choose
**Switch to this version** to activate one. This runs the same transaction as an
update: settings, service files and the recovery journal are saved first, the
release's own settings migration runs, and the previous state returns
automatically if the release fails its health check or power is lost. The
**Update history** records every update, switch and boot recovery. Releases older
than 0.3.2 do not show this list; to return to a newer one from them, upload its
update file again.

### Upgrading an installation older than 0.3.0

Older software has no upload endpoint. Run this **once** over SSH to add it;
subsequent updates use the web interface. The same commands remain available as
a manual deployment path (replace `0.3.0` with the release you downloaded):

```sh
scp ungoggled-0.3.0.update.tar.gz root@192.168.50.1:/tmp/
ssh root@192.168.50.1
update_dir=$(mktemp -d /tmp/ungoggled-update.XXXXXX)
cd "$update_dir"
tar -xzf /tmp/ungoggled-0.3.0.update.tar.gz
./install.sh
```

The installer starts a background update and prints the web address. Installation
status is also saved in `/var/lib/ungoggled-update/state.json`; logs are available
with `journalctl -u 'ungoggled-update-*'`. A read-only legacy root filesystem
must be remounted writable before installing or changing settings.

## Manual installation on Raspberry Pi OS Lite

Install the runtime packages:

```sh
sudo apt update
sudo apt install gstreamer1.0-tools gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav curl
```

Add this to `/boot/firmware/config.txt` and reboot:

```ini
[all]
dtoverlay=dwc2,dr_mode=peripheral
```

Then install an ARM64 update as above, using your own Pi login and
`sudo ./install.sh` when logged in as a non-root user. Use the Pi's existing network address to
open the web interface on port 80. The application installer preserves your network configuration;
the preconfigured SD image additionally supplies the ungoggled access point.
The receiver needs exclusive use of the USB device controller and HDMI display.

## Build from source

Requires Rust stable with edition 2024 support, Node.js 24+, Python 3.11+ and npm.

```sh
cd web
npm ci
npm run build
cd ..
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

On the Pi, install the resulting binary and web assets with:

```sh
sudo scripts/install-pi.sh target/release/ungoggled
```

To build the ARM64 update on Linux:

```sh
rustup target add aarch64-unknown-linux-musl
scripts/build-update.sh
```

Build the SD image with `scripts/build-image.sh`. It uses the checksum-pinned
official base image and installs packages inside an isolated ARM64 filesystem.
The Linux build host needs QEMU's registered `qemu-aarch64` binfmt handler,
subordinate UID/GID mappings, `unshare`, `newuidmap`, `newgidmap`, e2fsprogs,
curl, xz, Rust and Node.js. It does not write to a physical SD card. Outputs
are written to `dist/`: one image, one update, and their checksum files.
Intermediate bundles, release notes and build reports stay in `build/`.

For a hardware-free integration check, install GStreamer with its software H.264
and JPEG plugins, then run:

```sh
cargo build
python3 tests/smoke.py target/debug/ungoggled /path/to/captured-video.h264
```

## Versions and releases

`Cargo.toml` is the version source. Use stable semantic versions (`MAJOR.MINOR.PATCH`):

```sh
python3 scripts/version.py --set 0.3.1
# Add a matching "## 0.3.1" entry to CHANGELOG.md.
python3 scripts/version.py --check
```

Commit the changes and push a matching `v0.3.1` tag when ready to publish.
The release workflow builds for ARM64 and runs the tests, then creates a GitHub
release with notes from `CHANGELOG.md` and these assets:

- `ungoggled-VERSION-pi4-arm64.img.xz` — fresh SD-card installation.
- `ungoggled-VERSION.update.tar.gz` — complete application update for ARM64.
- SHA-256 checksum files for both downloads.

The update package format and migration entry point stay compatible with the
0.3.0 updater. Each target release handles migration from older settings; no
intermediate release is required. Build locally with `scripts/build-update.sh`
and `scripts/build-image.sh`. See [CHANGELOG.md](CHANGELOG.md) for release notes
and [the updater contract](docs/UPDATES.md) for maintenance details.

## Service and files

The service and persistent paths retain their original `dji-hdmi` names for
compatibility with existing installations. The application executable is
`ungoggled`.

| Location | Purpose |
| --- | --- |
| `dji-hdmi.service` | Receiver, renderer and HTTP server |
| `ungoggled-update-recovery.service` | Recover interrupted installations at boot |
| `/var/lib/ungoggled-update/` | Update progress, staging and rollback snapshots |
| `/opt/dji-hdmi/current` | Active application release |
| `/opt/dji-hdmi/releases/` | Installed releases |
| `/var/lib/dji-hdmi/` | Settings and fallback images |
| `/run/dji-hdmi/` | USB/video socket and GStreamer cache |
| `/etc/default/dji-hdmi` | Decoder/runtime configuration |
| `/etc/NetworkManager/system-connections/dji-hdmi.nmconnection` | Image's Wi-Fi AP profile |

```sh
sudo systemctl restart dji-hdmi
journalctl -u dji-hdmi -f
```

Rust handles USB, DJI framing, display management, settings and the Axum HTTP API.
GStreamer handles decoding and the small JPEG preview. The web application uses
React, TypeScript, Rsbuild and Tailwind. USB workers are supervised independently
of the persistent renderer.

Historical protocol research and
legacy setup details are retained in
[FINDINGS.MD](FINDINGS.MD). Dependency attribution is in
[THIRD_PARTY.md](THIRD_PARTY.md).

Independent project, not affiliated with DJI or Cosmostreamer. Browser broadcast
streaming, recording management and remote camera controls are outside the
current feature set.
