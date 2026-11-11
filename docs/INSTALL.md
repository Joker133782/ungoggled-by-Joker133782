# Installation and updates

[← Back to the README](../README.md) · Downloads: [DOWNLOAD.md](../DOWNLOAD.md)

- [Hardware and wiring](#hardware-and-wiring)
- [Install the SD-card image](#install-the-sd-card-image)
- [Update without reflashing](#update-without-reflashing)
  - [Switching back to an earlier version](#switching-back-to-an-earlier-version)
  - [Upgrading an installation older than 0.3.0](#upgrading-an-installation-older-than-030)
- [Manual installation on Raspberry Pi OS Lite](#manual-installation-on-raspberry-pi-os-lite)

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

Optional extras (beeper, status LEDs, power button) are described in the
[user guide](USER-GUIDE.md#beeper-leds--power-button). Other goggles:
[GOGGLES.md](GOGGLES.md).

## Install the SD-card image

The image is based on **Raspberry Pi OS Lite 64-bit, Debian 13 Trixie**.

1. Download `ungoggled-VERSION-pi4-arm64.img.xz` from the release files
   (see [DOWNLOAD.md](../DOWNLOAD.md)).
2. Burn it to the microSD card. Recommended writers: Caligula (on Linux), Raspberry Pi Imager (on Windows).
3. Skip any OS customization the tool offers: the image already contains its
   network and login setup.
4. Verify the card, insert it into the Pi, and power on.
5. Join **ungoggled** using the password **ungoggled**.
6. Open **http://192.168.50.1**. The first visit asks you to choose a password
   for the web interface.

### Default credentials

| Setting | Default |
| --- | --- |
| Wi-Fi SSID | `ungoggled` |
| Wi-Fi password | `ungoggled` |
| SSH username | `root` |
| SSH password | `ungoggled` |
| Web interface | `http://192.168.50.1` |

Connect with `ssh root@192.168.50.1`. Change Wi-Fi credentials in the web
interface. From 0.3.52 on the SSH/`root` password is the same as the control
password: the first password you choose in the web interface (and every later
change of it) is also set for `root`. Application updates preserve existing
passwords; these defaults apply to freshly flashed images. Forgotten password:
put an empty file named `reset-password` on the SD card's boot partition (the
small one with `config.txt`) and start the Pi, see the user guide.

Writing the image puts about 3 GiB on the card. The root partition then
expands to fill it on first boot, and each device generates its own
SSH host keys. The image's Wi-Fi radio country is
**DE**; configure the correct country if using it elsewhere.

> Version 0.3.0 fixes two fresh-image issues found in 0.2.0: disabled Wi-Fi and
> V4L2 decoder rejection of the goggles’ H.264 level metadata. Live O3 decoding
> at 1080p30 and O4 captured-video decoding have been checked on Trixie. See
> [validation notes](../FINDINGS.MD) for hardware test details.

## Update without reflashing

From version **0.3.0**, open **Software update** (on the **System** tab) in the
web interface. Either:

- Click **Check for updates**, and if a newer version is found, **Download**
  it (over the viewing phone's own connection, not the Pi's — the Pi itself
  never needs internet access for this) and then **Install now** or
  **Install later**; or
- Click **Upload update** and choose an `ungoggled-VERSION.update.tar.gz`
  file you already have. The Pi checks its contents and checksums either way.

Keep the Pi powered while it updates; the interface reconnects and reloads
when installation finishes.

Only **Raspberry Pi 4 with 64-bit Raspberry Pi OS (ARM64)** is supported.
The update contains the entire application: you can skip intermediate application versions. Only install update
files from releases you trust; checksums detect corruption, not publisher identity.

Compatible HDMI, preview and idle-screen settings are retained, as are uploaded
images, Wi-Fi configuration and passwords. Incompatible application settings use
defaults and are listed after installation. Video briefly stops and the RAM
history resets. If the new application fails its startup check, the previous
application and settings are restored. A recovery service handles interrupted
installations on the next boot. Keep at least **640 MiB** free on the root partition.

The update check runs in the browser when you open the **System** tab, and shows
a notice with release notes when a newer version is available. This uses your
phone's or computer's Internet connection; the Pi itself can stay offline. Uploads
work without Internet access. Release files are also on the
[GitHub releases page](https://github.com/Joker133782/ungoggled-by-Joker133782/releases).
Some updates change boot settings that only take effect after a restart; the
interface then shows a "Restart the Pi once" notice (it never restarts on its own).
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
