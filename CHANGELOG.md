# Changelog

## 0.3.10

- Recording: new "Auto-record every flight" toggle, buffering each live session in RAM (tmpfs) and saving it to the SD card as soon as the feed drops, keeping the 5 most recent flights. Manual start/stop recording stays available independently.
- Recording: saved files now get the same cached thumbnail treatment as USB videos, with download/delete/play-on-HDMI in one list.
- USB videos: list the camera's actual product name (e.g. "DJI DJIAction2"), not the raw Linux block device name (e.g. "sdc"), read from the USB device's own descriptor strings rather than the often-generic SCSI INQUIRY vendor/model.
- Checkboxes across the web interface (preview, beeper/LEDs, spectator stream, auto-record) now render as a green/red toggle switch instead of a native checkbox — one shared CSS rule, since every checkbox already used the same wrapper.
- Fixed USB-video thumbnails for real action-cam footage: found by testing against a DJI Action 2 over USB passthrough (see below) rather than only synthetic clips. A caps filter with only `width=` let `videoscale` leave height unchanged and rewrite the pixel-aspect-ratio to compensate instead of actually resizing, invisible at 16:9 but turning a real 4:3 4096x3072 recording into a 160x3072 thumbnail; fixed by forcing `pixel-aspect-ratio=1/1`. Also raised the generation timeout from 5s to 30s: a real 3.7 GB 4K60 HEVC clip took about 20s for GStreamer to read and index before decoding anything.

## 0.3.9

- Experimental spectator stream: phones/tablets on the Pi's own Wi-Fi can watch the live feed in a browser at `/watch.html`, separate from the HDMI output. MPEG-TS over a long-lived HTTP connection, decoded client-side with mpegts.js — real H.264, much lower latency than HLS. Toggle and quality ("High", relayed unmodified, ~1-2 viewers; "Event", transcoded lower-bitrate for more viewers) configurable in the web interface under **Spectator stream**, with the actual Wi-Fi bandwidth trade-off explained there. Verified end-to-end in a real browser against synthetic test video.

## 0.3.8

- Experimental: record the live goggles feed to the SD card (no re-encoding), started/stopped from the web interface, plus a "save last 30 seconds" shortcut backed by an always-on ring buffer. Stops itself, and won't start, below 2 GB free space; the HDMI output shows a "SD card full" notice once the goggles feed stops, without interrupting a live picture to show it. Recordings list in the web interface for download, deletion, or playback on the HDMI output.

## 0.3.7

- The update file now carries mpv's `.deb` packages and installs them offline after a successful update, so a Pi on an older release picks up USB-drive video playback automatically without needing internet on the Pi or a manual `apt install`. Best-effort: installing the packages never affects the rest of the update, succeeded or failed.

## 0.3.6

- Fix USB video seeking: mpv's IPC command array rejected the combined `absolute+exact` seek flag. Found by actually running the thumbnail GStreamer pipeline and the mpv IPC protocol for the first time, after getting sudo on the build host; both now verified to work as coded (except real DRM/KMS output, still unverified without a Pi and monitor).

## 0.3.5

- USB videos: play/pause and seek from the web interface (video stays on the HDMI output only), switching the playback backend from GStreamer to mpv. Needs `mpv` on the Pi, now included in the SD-card image; an already-flashed Pi needs a re-flash or `apt install mpv`.
- USB videos: thumbnails, cached in the tmpfs runtime directory so they never accumulate on the SD card and are gone after every reboot or power loss.

## 0.3.4

- Experimental: browse video files on a USB drive or action camera plugged into a regular USB-A host port and play one on the HDMI output, listed in the web interface. Not hardware-verified for every camera and codec; resolution and codec support depend on what the Pi's GStreamer install can decode.

## 0.3.3

- Experimental, hardware-unverified USB-host receiver for the original DJI FPV Goggles (V1/V2), based on the community-reverse-engineered `voc-poc` protocol. Opt in with `UNGOGGLED_GOGGLES=v1v2`; requires a USB-A host port, not the gadget USB-C port. The exact stream framing is not fully understood even upstream.

## 0.3.2

- Loop an uploaded MP4 (H.264, up to 1080p and 64 MiB) as the HDMI fallback while no video arrives. Uploads are demuxed without re-encoding and use the hardware decoder.
- Experimental DJI Goggles 2 support. The receiver tries the Goggles 3 and Goggles 2 start sequences, keeps the one that produces video and remembers it per pair of goggles by their USB identification. `UNGOGGLED_GOGGLES` and `UNGOGGLED_ACCESSORY_PID` in `/etc/default/dji-hdmi` override the choice.
- List installed versions in the web interface and switch back to an earlier one through the same safe transaction as an update, with automatic restore if it does not start.
- Record updates, switches and boot recoveries in an update history.
- Keep the SD-card image release, the active and preceding releases and one older update.

## 0.3.1

- Set O3/O4 recording timestamps from the Pi clock after the goggles USB session starts.
- Show the Pi clock in the web interface and copy the browser time (and timezone) when they differ by more than a second.
- Optional setting (off by default) to beep every 5 seconds until the Pi clock has been synced from a browser or NTP since boot.
- Default GPIO pins: piezo 6, shutdown button 19, goggles LED 17, video LED 23, HDMI LED 24.
- Offer every HDMI mode the display advertises, including interlaced modes and modes above 1080p60. Automatic mode still prefers progressive 1080p.

## 0.3.0

- Serve the web interface on standard HTTP port 80.
- Upload and install software updates from the web interface, with upload progress and automatic reconnection.
- One complete update file supports Pi 4 installations running 64-bit Raspberry Pi OS, without installing intermediate application releases.
- Preserve compatible HDMI, preview and fallback settings. Keep uploaded images, Wi-Fi settings and login credentials.
- Restore the previous application and settings if the new version fails its startup check. Recover interrupted installations at boot.
- Check GitHub releases from the browser and show newer stable versions, release notes and download links.
- Fix V4L2 hardware decoder negotiation for Goggles streams advertising H.264 level 5.2, without changing the encoded video.
- Fix fresh-image Wi-Fi being disabled by the stock NetworkManager radio state, and persist the configured radio country at boot.
- Add semantic version management and automated release builds containing an SD-card image and an ARM64 update file.
- Halve the SD-card image: the root filesystem is sized to its contents plus working headroom, and documentation, translations and cached package lists the appliance never reads are left out. The partition still expands to the whole card on first boot.
- Ship only the current application release in the SD-card image, instead of also carrying the release directories of earlier builds.
- Show goggles, video and HDMI state in a header that stays visible while scrolling. Goggles state comes from the USB controller, so a cable that is attached but has not finished the accessory handshake is no longer reported as disconnected.
- Report an attached HDMI display only when it answers with an EDID; the image forces an HDMI mode, which leaves the connector reporting "connected" with nothing plugged in.
- Recover on its own when the goggles keep the port configured but the gadget stops receiving events after an update restart, instead of needing a manual service restart.

## 0.2.0

- Live HDMI output for Goggles 3 with O3 Air Unit and O4 Air Unit Pro.
- Browser preview, HDMI controls, fallback images, Pi temperature and a 30-minute transmission history.
- Wi-Fi configuration and preconfigured Raspberry Pi OS Lite images.
- Rename the project to ungoggled. Default Wi-Fi SSID, Wi-Fi password and root password are `ungoggled`.
