# User guide: the web console

[← Back to the README](../README.md)

The web console runs on the Pi itself. Join the Pi's Wi-Fi (`ungoggled` /
`ungoggled` on a fresh image) and open **http://192.168.50.1**. It has four tabs:
**Live** (status, preview, recording, spectator stream), **Videos** (USB player and
saved recordings), **Settings** and **System** (updates, versions, clock, power,
graphs, diagnostics). Every panel has a **?** link to the built-in help page.

The receiver starts automatically at boot and recovers by itself after USB
disconnects or stream changes. The Live tab shows the incoming resolution, the
measured camera fps and bitrate, the HDMI mode and refresh rate, and the rendered
fps; the System tab shows the Pi's temperature.

- [Web interface](#web-interface)
- [Beeper, LEDs & power button](#beeper-leds--power-button)
- [USB - ActionCam Video Player (experimental)](#usb---actioncam-video-player-experimental)
- [Recording (experimental)](#recording-experimental)
- [Spectator stream (experimental)](#spectator-stream-experimental)

## Web interface

**Password:** the control interface (settings, recordings, Wi-Fi, everything
except the spectator stream page) needs a password. The first visit asks you to
set one. After 5 wrong passwords in a row, that address is locked out for 5
minutes. Everyone logs in again after an update or reboot.

**Start/stop** (System tab) controls USB reception. **Reconnect goggles** (Live
tab) restarts the USB session. The display controller stays running so it can
show the idle screen.

**Idle screen** (formerly *fallback image*): what the HDMI output shows while
there is no live picture. Built in are a plain dark screen, three looping videos
(*Logo loop*, *Logo loop 3D*, *FPV comic*) and a “No signal” image. You can upload
PNG, JPEG or WebP images (up to 12 MiB, up to 16 images); they are converted to
PNG, limited to 1920 × 1080 and displayed with preserved aspect ratio. You can
also upload up to three looped videos (MP4 with H.264, up to 1080p and 64 MiB),
demuxed on upload without re-encoding and hardware-decoded like the live feed.
The selection survives reboots.

**HDMI mode:** automatic mode prefers progressive 1080p at up to 60 Hz. You can
choose any other mode the display advertises, including interlaced modes such as
1080i for equipment that requires them. Interlaced modes are listed by field
rate, so 1080i60 carries 30 full frames per second. Video is scaled to the
selected resolution. Applying a mode briefly interrupts output. A 60 Hz HDMI signal may carry a 30 fps camera feed: the UI shows those
rates separately. It does not increase the camera frame rate. With a display on
each micro-HDMI port you can also pick the output port. Since 0.3.51 the Pi
also lists 3840x2160 at 50 and 60 Hz when the display offers it
(`hdmi_enable_4kp60`, which also raises the video chip's clock a little): that
works only on the micro-HDMI port next to the USB-C port and needs a
high-speed (HDMI 2.0) cable, and the automatic mode keeps using 1080p. If the
picture turns black after choosing a 4K mode, pick Automatic again and restart
the Pi if it does not come back.

**Stretch to fill the screen:** two switches under **Settings → HDMI**, “Stretch
the live picture to fill the screen” and “Stretch USB videos to fill the screen”,
pull a 4:3 picture over the whole 16:9 screen instead of showing black bars at the
sides. Both are off by default; the USB one also works while a video is playing.
The Pi's display chip refuses to stretch some live picture sizes; those are shown
as they arrive.

**Preview:** this is a small live JPEG preview (up to 640 × 360, 5 fps) for
framing and monitoring, not a broadcast stream. Preview does not change the HDMI
mode. Disable preview to reduce processing and network traffic.

**Last 30 minutes:** the System tab shows the Pi's last half hour as small graphs:
video (bitrate, camera and TV frame rate), Pi health (temperature, CPU, RAM),
network (Wi-Fi, Ethernet) and SD card. Touch or hover over a graph to read the
same moment off all of them. Shading marks periods without live video. Groups can
be switched off under **Settings → Graphs**. History is held in RAM, sampled once
per second; it is cleared when the application restarts and does not write
continuously to the SD card.

**Wi-Fi:** enter the SSID and a new password, then apply. Reconnect your computer
using those credentials. HDMI reception continues during the network change. The
Pi's own access point uses 5 GHz (channel 36). The same panel can also join an
existing Wi-Fi network instead: scan or type its name, enter its password and
join. The Pi's own access point goes down while it is joined elsewhere, and comes
back by itself if joining does not succeed within 30 seconds.
The web interface is intended for your private local network; do not expose port
80 to the Internet.

**HTTPS:** the same interface is also served on port 443 over HTTPS, with a
self-signed certificate generated once on first use and kept in the data
directory from then on. A real, publicly trusted certificate isn't possible
here — that needs a publicly resolvable domain name, which a private access
point at a fixed local IP doesn't have — so the first visit from a new
browser shows a "connection isn't private" warning; proceed past it once
("Advanced" → "Proceed"). Without this, browsers refuse to treat the page as
installable and "Add to Home Screen" falls back to a plain bookmark with the
address bar still showing; over HTTPS, once the warning is accepted, it
installs as a real fullscreen app. Set `--tls-listen ""` to disable it.

**Clock:** air-unit recording timestamps come from the Pi clock. When the Pi's
clock is off, the browser sets it automatically; **Sync Time** (System tab) does
the same by hand.

**System:** reboot or shut down the Pi from the web interface; both ask for
confirmation first. **Download diagnostics** saves a support bundle.

**Terminal:** an optional root shell in the browser, for troubleshooting with
someone who knows Linux. Off by default; switch it on under **Settings →
Terminal** and open it from the System tab.

## Beeper, LEDs & power button

All optional. Goggles, video and HDMI each get a
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

## USB - ActionCam Video Player (experimental)

Plug a USB drive or an action camera acting as USB mass storage into one of
the Pi's regular USB-A ports (never the USB-C gadget port the goggles use).
It is mounted read-only automatically; video files found on it (`.mp4`,
`.mov`, `.mkv`, `.avi`, `.m4v`, `.insv`, `.lrv`) are listed in the web
interface under **USB - ActionCam Video Player** (Videos tab), each with a cached thumbnail (a single
extracted frame; the cache lives under the runtime directory, which is
tmpfs, so it is gone again after any reboot or power loss, not left on the
SD card). Playing one takes over the HDMI output from live goggles video and
the idle screen; play/pause and seeking are controlled from the web
interface, but only the picture goes to the monitor, nothing streams back to
the browser. Unlike the idle-screen video, there is no size limit and no
re-encoding. A line under the time bar shows the pictures per second that reach
the screen and the speed compared with real time: very large action-camera clips
are more than the Pi 4 can decode in real time and play in smooth slow motion.

This plays through **mpv**, not GStreamer, so it needs `mpv` installed on
the Pi. It is included in the SD-card image. An update installed from the
web interface also carries mpv's `.deb` files and installs them offline if
missing, so a Pi running an older release picks it up automatically on its
next update, without needing internet access on the Pi itself; that is a
one-time, best-effort step that never affects the rest of the update (see
`scripts/bundle-mpv.sh`). If it still could not install for some reason, the
web interface reports that mpv is missing instead of failing silently.
Playback otherwise uses whatever codec and resolution the file needs and
whatever the Pi's mpv/ffmpeg install can decode for it. HEVC hardware decode
(e.g. DJI Osmo Nano footage) is hardware-verified with this image's 640MiB
CMA pool; without that headroom GStreamer's own worst-case buffer
pre-allocation can starve the decoder before it even starts (see
[FINDINGS.MD](../FINDINGS.MD)).

## Recording (experimental)

Four independent ways to capture the live goggles feed to the SD card
exactly as received (no re-encoding). The switches are under **Settings →
Recording**, the buttons on the **Live** tab, and every saved file is listed
under **Recordings** on the **Videos** tab, with download, deletion, and playback
on the HDMI output, each with a thumbnail:

- **Auto-record every flight**: a toggle. While on, every live session is
  buffered in RAM (under the runtime tmpfs, not the SD card) for as long as
  the feed lasts, and saved to the SD card the moment the feed drops — no
  start/stop needed. Keeps the 5 most recent flights; the oldest is deleted
  automatically once a new one is saved.
- **Crash buffer (last 30 seconds)**: a toggle, like the goggles' own
  instant-replay. The ~30 s ring buffer always runs in the background at low
  memory cost regardless of this setting; turning it on makes a feed drop
  (e.g. a crash) auto-save it, with no button to press. Keeps the 3 most
  recent clips, a separate rotation from the 5 kept flights.
- **Start recording** / **Stop recording**: manual, writes straight to the
  SD card as it records, independent of the toggles above (all can run at
  once).
- **Save last 30 seconds now**: grabs the current ~30 s ring buffer on
  demand, independent of the crash-buffer toggle, for a moment you did not
  plan to record and the feed never dropped for.

**Record to USB drive instead of SD card** (Settings → Recording) writes
recordings to a plugged-in USB drive; without one they stay on the SD card.

Manual and auto-recording both stop themselves, and auto-recording's save
is skipped, with less than 2 GB of free space left on the SD card; the
HDMI output then shows a "SD card full" notice once the goggles feed
itself stops (it never interrupts a live picture just to show that).

The goggles' own on-screen display (battery, channel, signal bars) is part
of the picture whenever the goggles themselves are configured to add it to
the output, so it is part of a recording the same way; that is a setting
on the goggles/DJI Fly, not something the Pi controls. Not
hardware-verified for the auto-record/RAM-buffer path specifically (the
manual recording and USB-video thumbnail pipeline it shares code with have
been, against real action-cam footage).

## Spectator stream (experimental)

Lets phones and tablets connected to the Pi's own Wi-Fi watch the live feed
in a plain browser, at `/watch.html`, separately from the HDMI output. This
is not the goggles' own low-latency link the pilot flies with — only
something extra for bystanders, with a latency of a second or two. Off by
default; turn it on under **Spectator stream** (Live tab), with a quality choice:
**High quality** relays the original goggles stream unmodified (close to the HDMI
picture, but only realistic for one or two viewers at once, since every
viewer costs roughly that much of the Pi's own Wi-Fi bandwidth); **Low quality**
decodes and re-encodes at a lower resolution and bitrate in software, to let
the Pi's own Wi-Fi carry more simultaneous viewers at reduced quality. No
protocol choice changes that trade-off between viewer count and quality:
the Pi's own Wi-Fi radio is the actual limit (roughly 50–100 Mbit/s total,
shared with the web interface itself), not the streaming method.
**Show QR codes for spectators** gives bystanders one code to join the Wi-Fi and
one to open the stream page. The stream page never asks for the control password.

Delivered as MPEG-TS over one long-lived HTTP connection, decoded in the
browser with [mpegts.js](https://github.com/xqq/mpegts.js) (loaded from a
CDN), not HLS: HLS's segment files add several seconds of latency that
cannot be configured away, which defeats the point for live FPV spectating.
The GStreamer muxing pipeline and `mpegts.js` playback were verified
end-to-end in a real browser against synthetic test video; not against the
live goggles feed on real hardware. **Low quality** additionally needs
`gstreamer1.0-plugins-ugly` (for its software H.264 encoder), which is not
yet offered as an offline-installable update bundle the way mpv is — a Pi
on an older image needs `apt install gstreamer1.0-plugins-ugly` for that
quality level specifically; **High quality** needs nothing beyond what
every image already has.
