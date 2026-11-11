# Verified investigation: Goggles 3 → Pi 4 → HDMI

Updated: 2026-09-16. Original notes reviewed at `/home/timo/dev/FINDINGS.MD`.
This document supersedes those notes where conclusions differ.

## What Cosmostreamer is

Cosmostreamer is a commercial hardware/software video bridge. Its Goggles product
receives a compatible DJI goggles feed, outputs it over HDMI and optionally
re-streams video, adds telemetry, and controls supported camera functions.
The vendor explicitly lists Goggles 3, O3 and O4 Pro. Those are **vendor claims**,
not proof that our independent implementation works.

- Product: https://cosmostreamer.com/products/djigoggles2/
- Wiring and hardware: https://cosmostreamer.com/products/djigoggles2/diy/

## Audit of the previous findings

| Earlier claim | Result of verification |
| --- | --- |
| PC enumerates Goggles 3 as `2ca3:0020` | Confirmed through local sysfs; Pi host enumeration independently agrees. |
| Pi is a Raspberry Pi 4, runs 32-bit Cosmostreamer | Confirmed `armv7l`, kernel `5.10.63-v7l+`, Raspbian 10 Buster. |
| Pi USB-C uses `dwc2` peripheral mode and gadgetfs | Confirmed boot config, UDC, module, device nodes and kernel logs. |
| AOA alone starts video automatically | Unsupported and contradicted by public protocol implementations. AOA establishes transport; DJI registration/keepalives still matter. |
| No open implementation exists | Too broad. There is a working, explicitly restricted G2/O3 implementation and an N3/O4 mobile protocol reference. Neither proves G3 compatibility. |
| Simply present an accessory PID | Not sufficient on the G2 reference: it needed the host-driven AOA transition first. G3 later resumed an already-negotiated session with the accessory PID; a fresh cold boot remains a separate test. |
| FunctionFS is required/preferred | Both drivers are available. Live testing exposed lost AOA setup events in gadgetfs on this kernel; FunctionFS completed all six AOA strings, START, re-enumeration, and live O4 reception. No kernel replacement was needed. |
| Pi 5 cannot do this | Vendor does not support it and it lacks the Pi 4 H.264 hardware decoder. That is not proof that software decoding is impossible. Pi 5 is outside our target. |
| Pi has a display connected | Driver status alone is insufficient: this image forces a 640×480 output and initially returned **zero EDID bytes**. UI now distinguishes monitor detection from connector status. |
| Invalid Cosmostreamer license / SD-card corruption | Not assumed as a protocol explanation. Kernel did report unclean FAT filesystem on mmcblk0p1; no filesystem repair or license alteration performed. |
| Previous investigation restored all state | Not fully true: two stopped shell processes from the earlier traces remained. We did not resume these obsolete tracing jobs. |

The original detailed IF4/DUML routing, SDK registration theories, license state,
and RNDIS conclusions have **not** been reproduced in this session and must not
be treated as established facts.

## Independently checked protocol sources

1. Android AOA specification:
   https://source.android.com/docs/core/interaction/accessories/aoa
   Requests 51 (protocol), 52 (identifying strings), 53 (start), then USB
   re-enumeration. `18d1:2d00` is accessory-only; `2d01` includes an ADB interface.
   The first successful G3 bench run used **2d01** as a compatibility experiment, with a single accessory interface and no ADB function. The specification-correct **2d00** subsequently completed full AOA negotiation and live reception on September 16. It is the source default and the current deployed setting; cold-accessory flags are no longer used.
2. Linux gadgetfs kernel implementation and UAPI, checked against v5.10:
   https://github.com/torvalds/linux/blob/v5.10/drivers/usb/gadget/legacy/inode.c
   https://github.com/torvalds/linux/blob/v5.10/include/uapi/linux/usb/gadgetfs.h
   Control ACK for host-to-device setup is a zero-length **read**, not write.
   Unsupported requests stall by opposite-direction I/O. Events are 12 bytes on
   both 32- and 64-bit systems. FS bulk max-packet is 64; HS is 512.
3. DJI mobile protocol at commit `50c71b65fdb6825783f724e5cd3c1f09c2aa88ce`:
   https://github.com/samuelsadok/dji_protocol/blob/50c71b65fdb6825783f724e5cd3c1f09c2aa88ce/usb_mobile_protocol.md
   https://github.com/samuelsadok/dji_protocol/blob/50c71b65fdb6825783f724e5cd3c1f09c2aa88ce/scripts/video_out_mobile.py
   This is an **iOS/MFI** reference tested on N3 and O4 Pro, not an AOA G3 test.
4. G2/O3 Pi endpoint at commit `64c5801ef0201a7a6070332df42e77d208fe10b2`:
   https://github.com/dr00min/o3-usb-decoder/tree/64c5801ef0201a7a6070332df42e77d208fe10b2
   It expressly excludes G3 and O4 support. Useful independent evidence for
   AOA framing, identity responses and host-driven accessory negotiation.
5. N3 Android implementation:
   https://github.com/jtm2kfpv-git/DJI-Unchained-VOC
   Its source verification script checks byte equality to the reference packets;
   that does not independently establish that their declared lengths are right.

## Important correction in the public N3 reference

Both documented `MAGIC_PACKETS` contain two extra bytes after the DUML frame:

- Packet 1 declares 45 DUML bytes, supplies 47. The valid 45-byte prefix already
  ends with CRC `d0 93`. Extra suffix `92 3a` is outside the declared frame.
- Packet 2 declares 27 DUML bytes, supplies 29. The valid 27-byte prefix already
  ends with CRC `58 a6`. Extra suffix `34 18` is outside the declared frame.

Confirmed using independent Rust bitwise CRCs and the G2 project's Python CRC
implementation. Copying the alleged payloads literally includes an old checksum
as application data. Our implementation uses the correctly bounded payloads and
recomputes both checksums. Tests compare complete output to both valid prefixes.
Whether extra trailing bytes are tolerated by the device does not make them part
of the declared DUML message.

Our initial minimal session sends `00:99` to address `0x28` and `00:88` to `0x3c`,
every three seconds, with fresh sequence numbers. The G2 AOA reference sends control on `0x7530`; the N3 Android application
and iOS reference send on `0x5749`. The prototype now defaults to `0x5749`
(decimal 22345), configurable with `--control-port`. Incoming telemetry can
arrive on `0x7530`. Video uses `0x574a`.
**Verified on the attached G3 + O4 Pro:** these corrected registrations produce live video over AOA. Replies include `28→02 00:99` and `3c→02 00:88`. We do not replay the G2 project's entire captured app session,
which includes unrelated commands.

## Hardware evidence so far

| Test | Result |
| --- | --- |
| ARM musl executable on supplied Buster Pi | PASS: runs, reports controller and DRM diagnostics. |
| gadgetfs descriptor binding | PASS: independent Rust worker binds `fe980000.usb`, endpoint nodes appear. |
| React static UI and status API over Wi-Fi | PASS: HTTP 200 at port 8090. |
| Generated H.264 1920×1080, 60 fps → `v4l2h264dec` → fakesink | PASS: parsed/decoded caps match, reaches EOS without errors. |
| Same generated clip → `kmssink sync=false` | PASS at software pipeline level: decoded caps, PLAYING, EOS. No visual confirmation yet. |
| `kmssink force-modesetting=true` on this legacy image | FAIL: cannot configure video mode. Removed forced modesetting; overlay path succeeds. |
| Goggles attached to Pi USB-A | Confirmed `2ca3:0020` on xHCI port `1-1.2`; wrong USB role. Requested move to Pi USB-C. |
| Goggles 3 + O4 Pro live USB | PASS: FunctionFS AOA handshake and approximately 6–7 Mbps H.264, 1920×1080 High level 5.2, with zero framing resync bytes over more than 15 minutes. |
| O4 Pro live hardware decode → HDMI | PASS: owner confirmed continuous live camera feed. After stopping competing media helpers and enabling a writable plugin cache, approximately 30 fps is measured. |
| Goggles 3 + O3 live USB and HDMI | PENDING hardware test. |
| Receiver process replacement after successful negotiation | PASS: normal AOA negotiation with default `18d1:2d00` returns to live streaming without a goggles reboot. Earlier cold-accessory experiments are no longer necessary. |
| Monitor attached after headless start | PASS: Rust DRM mode selection upgraded 1024×768 fallback to 1920×1080 at 60 Hz while retaining 1080p30 camera input. |
| Low-power-state toggles and clean feed | PASS: owner retested full-HD clean output and toggled Auto Low-Power State off/on, reporting “All works well.” Exact blackout duration was not measured. |
| Pi reboot with Goggles 3 + O4 Pro connected | PASS: standalone Wi-Fi and receiver start automatically; full AOA negotiation, 1080p input, one decoder start and zero USB resync bytes observed. |
| Physical USB reconnect / air-unit power cycle / O3 switch | Separate physical tests still pending. |

The Pi clock is set to February 2020. Its journal timestamps are not the test
date. Capture bounded samples and use session-relative statistics.

## Implementation decisions

- Rust owns USB via configfs/FunctionFS, framing, control session, worker supervision and HTTP API. The legacy gadgetfs experiment remains selectable for diagnosis.
- GStreamer runtime handles the Pi hardware decoder and DRM overlay. This avoids
  linking a modern Rust GStreamer dependency against the old Pi runtime.
- Queue is bounded. Decode restarts at an H.264 SPS after loss; arbitrary compressed
  chunks are not dropped and then presented as an intact stream.
- Raw capture is opt-in and capped, so it cannot fill the Pi RAM filesystem
  without a deliberate larger limit.
- No Pi OS replacement was needed. A temporary development service runs from
  `/tmp`; a dedicated-install script is supplied separately.


## Wiring clarification from the owner

The previous working Cosmostreamer USB-A setup was **Goggles V2 + O3**,
not Goggles 3 + O4 Pro. Thus it does not demonstrate a G3 USB-A video path.
The owner prefers charging while connected but is willing to assess USB-C
once its benefits are established. G3 has now enumerated our Pi peripheral
at USB high speed and sent AOA GET_PROTOCOL and SEND_STRING requests.
The first run exposed gadgetfs EIDRM on closely spaced control requests;
recovering from a superseded setup request is now implemented. Full AOA
transition and live O4 video subsequently succeeded with FunctionFS.

### FunctionFS transition

After a goggles reboot, all observed gadgetfs failures occur during the fast
SEND_STRING sequence; some requests are overwritten and the final START is not
processed. Catching EIDRM alone did not fix the stalled control state. The new
FunctionFS transport delegates vendor requests with ALL_CTRL_RECIP and
CONFIG0_SETUP, reads each OUT data phase explicitly, and changes PID only after
ACKing START. It is bound successfully on the existing Pi kernel.

## O4 decode investigation and legacy runtime workaround

The first USB capture is an ordinary 1920×1080, 8-bit 4:2:0 H.264 High stream.
The SPS advertises level 5.2 and 30 fps timing; timing metadata alone does not
prove the measured frame rate. It decodes correctly in software on the PC.
Captures start midstream, so errors before the first SPS/PPS are expected.

On this **Buster / GStreamer 1.14.4** image:

- `v4l2h264dec` fails on the O4 capture with no valid frames and early EPIPE/EOS.
  Rewriting level 5.2 to 4.2 did **not** fix it. We do not modify the SPS in the application.
- FFmpeg's `h264_v4l2m2m` and `h264_mmal` both decode the same captured footage,
  establishing that the Pi's H.264 hardware can decode it.
- The installed `omxh264dec` fails while creating `OMX.broadcom.egl_render`.
- A separate build of upstream **gst-omx 1.14.4 without GStreamer GL support**
  removes that unnecessary EGL component. It decodes the original, unmodified
  O4 stream and renders through `kmssink`. No vendor executable is used.
- The isolated plugin lives at `/tmp/dji-hdmi/plugins/libgstomx.so`; the development
  unit sets `GST_PLUGIN_PATH` and uses `--decoder omxh264dec`.
- A brief monitor test reported by the owner showed captured camera footage.
  This was file playback, not yet a visual confirmation of the continuous live path.

This is an old-image compatibility workaround. Use a current Pi OS with its
V4L2 decoder for a fresh installation, and test that runtime rather than assuming
this Buster result applies to all GStreamer versions.

Source archive (unmodified):
https://gstreamer.freedesktop.org/src/gst-omx/gst-omx-1.14.4.tar.xz
SHA256: `969870e75c1f75c96f8783530e2c2932fc3afbfd976eb0c466f51dae268ea3d4`

Related upstream evidence about old V4L2 plugin compatibility:
https://github.com/FDH2/UxPlay/wiki/Gstreamer-Video4Linux2-plugin-patches
https://github.com/raspberrypi/linux/issues/4592

The control UI now uses `fpsdisplaysink` rendered-frame counters as evidence of
output, rather than GStreamer's PLAYING state alone. The video queue has a 2 MiB
byte limit as well as a message limit; overflow discards queued compressed data
and resumes at a fresh SPS. USB reception never waits for the video decoder.

### Monitor connection and subsequent stability work

The owner confirmed no monitor was attached for the earlier zero-EDID readings.
On connection the Pi returned 256 EDID bytes and listed 1920×1080 modes.
The existing boot mode remained 640×480; decoded input resolution must not be
confused with the current HDMI signal resolution.

The live OMX path ran approximately 24–25 fps for more than two minutes before
monitor testing. It later stalled and repeatedly overran its queue. Further
inspection found active Cosmostreamer display, second-camera, screen-capture,
audio and recorder helpers. These were stopped, leaving networking and the
hardware watchdog running. Their process supervisors were paused as well.

GStreamer was also rescanning plugins on every restart because its usual cache
location is on the read-only filesystem. The test unit now sets
`GST_REGISTRY=/tmp/dji-hdmi/gst-registry.bin`; the supplied systemd unit uses its
own writable runtime directory. After these changes, the live output reported
approximately **29 fps**, with no USB resync bytes. This is an observation from
the latest short run, not a completed sustained or hotplug validation.

A transient FunctionFS bulk-write EAGAIN had restarted one USB session. The
source now retries temporary backpressure, with a two-second deadline, and has
a regression test covering interrupted, partial and temporarily blocked writes.
This USB retry change is deployed on the Pi.

During the monitor test the Pi became unreachable from the PC. Reconnecting the
PC Wi-Fi association restored access; Pi uptime and RAM files survived, so this
was not a Pi reboot. The owner had only reconnected the goggles USB cable. The
underlying cause of the Wi-Fi interruption is unconfirmed. A permanent installation was subsequently staged on September 16 (see below);
the current development service still runs from `/tmp` until reboot.

## September 16: full-HD output and transition recovery

The owner confirmed live HDMI but reported low resolution, long blackouts when
toggling Auto Low-Power State, and embedded goggles OSD. Two bounded O4 captures
both had the same 1920×1080 SPS with 30 fps timing. This does not prove that every
transmission-mode transition preserves those parameters.

The receiver now owns the HDMI primary framebuffer through Rust DRM ioctls and
selects progressive 1920×1080 at up to 60 Hz when advertised by the monitor.
GStreamer renders the camera onto an overlay. A background EDID probe upgrades
a headless fallback when the monitor is attached; probing on the decoder thread
was found to block long enough to disrupt playback on this old kernel. HDMI
status now reports output width, height and refresh separately from input caps.
The live hotplug test upgraded to 1080p60 and resumed approximately 30 fps output.

The H.264 path parses NAL boundaries across USB reads, detects changed SPS, and
restarts after an actual input gap or missing decoded frames. Input gaps use
arrival timestamps, so slow decoder processing cannot manufacture a fake gap.
Repeated SPS does not trigger a restart. Blackout duration during actual goggles
mode changes remains subject to the physical test.

OSD is embedded in the raw USB capture, including goggles menus, not added by
our HDMI renderer. Cosmostreamer documents Camera View Recording OFF under
Settings → Camera → Advanced Camera Settings for a clean feed:
https://cosmostreamer.com/products/djigoggles2/diy/#Clean_HDMI_video_without_OSD.3F
The owner has now set this, and `research/captures/o4-pro-clean.h264` contains
1920×1080 video without the previously visible OSD. The extracted first frame
was visually checked on the PC. No undocumented camera-setting command
or pixel masking is implemented.

### Persistent installation staged

The receiver, React assets and legacy OMX plugin are installed under `/usr/local`.
The corresponding unmodified gst-omx source archive and build script are saved
under `/usr/local/share/dji-hdmi/source`. The systemd receiver uses a writable
registry under `/run/dji-hdmi` and settings in `/etc/default/dji-hdmi`.

For this specific Buster image, `scripts/install-bench-pi.sh` saves the existing
hostapd configuration and installs a standalone AP unit using the existing
`wifi0` udev name and 192.168.50.1 address. Existing dnsmasq stays enabled.
Cosmostreamer startup in rc.local is replaced with an empty script; the old
rc.local and dhcpcd.conf are backed up in `/var/lib/dji-hdmi/previous-boot`.
PID 1 is configured to feed the hardware watchdog after reboot. The live
watchdog owner and current networking processes were left running.

Units are enabled and syntax-checked, but no reboot has yet validated this boot
configuration. Root was returned to read-only after installation. O3 remains
untested. Do not describe these pending hardware checks as completed.

SSH connections later stalled despite a working HTTP API. A new SSH connection
with `IPQoS=none` completed immediately and sustained an 8 MB capture transfer.
This workaround is verified; the precise Wi-Fi/DSCP handling fault is not.

The live receiver was subsequently switched from `dji-hdmi-dev.service` to the
installed `dji-hdmi.service`. Its preparation step and process start succeeded.
The new build also logs the reason for camera-gap, SPS-change and monitor-mode
restarts, and exposes `decoder_starts` through the status API.

The installed service then completed normal AOA negotiation and sustained
1080p HDMI output at approximately 29.97 fps for over two minutes with
`decoder_starts: 1` and zero USB resynchronization bytes. Local-network HTTP
access was intermittent during this period even while SSH and loopback API
requests worked; it subsequently recovered. Root cause remains unconfirmed.

### Owner confirmation, September 16

The owner completed the requested HDMI test, including clean video, image
sharpness, and Auto Low-Power State off/on transitions, and reported that all
works well. The monitor was disconnected afterwards. This confirms the O4 Pro
visual test; it does not establish a numerical blackout duration or O3 support.

### Reboot validation, September 16

After `systemctl reboot`, the Pi returned with its standalone AP and installed
receiver active. The PC rejoined the saved Wi-Fi network. The receiver performed
all six AOA identifying-string exchanges and resumed 1920×1080 O4 video without
a manual application restart. Status showed one decoder start, zero discarded
USB framing bytes, and approximately 30 output fps over the observed run.
No monitor was connected: HDMI used a 1024×768 fallback; the prior hotplug test
already verified upgrading to 1080p. This reboot is an API/decoder verification,
not an additional visual HDMI test.

The boot journal confirms PID 1 owns the Broadcom hardware watchdog with a
10-second timeout. A separate Cosmostreamer `splashscreen.service` was discovered
starting its omxiv framebuffer viewer outside rc.local; it was stopped and
disabled, and the migration script now handles that service. Video continued.

The pre-existing rpi-eeprom-update service fails its package checksum check on
this image. This is separate from the receiver; no EEPROM flashing or repair
was attempted. Receiver, hostapd and DHCP startup succeeded. O3 compatibility
remains pending a hardware test.

### Owner confirmation: O3 and O4

After the earlier O3-pending notes above, the owner tested both O3 Air Unit and
O4 Air Unit Pro and reported both working flawlessly. This establishes support
on the original Pi/Buster installation; it is not a physical Trixie validation.

### Version 0.2 validation, September 17

The Rust renderer now belongs to the long-lived server. The USB worker sends
sequence-numbered compressed chunks over a bounded Unix datagram path. A
persistent primary DRM framebuffer displays the selected fallback while
GStreamer uses an overlay for camera video. Uploaded PNG/JPEG/WebP images are
converted to bounded-size PNGs. Settings/images persist on disk; the 1,800-sample
history and JPEG preview remain in RAM.

O4 Pro hardware observations on the supplied Buster Pi:

- Input: 1920×1080, nominal 30 fps; measured approximately 29–30 fps.
- Output: approximately 30 rendered fps; HDMI upgraded from headless 1024×768
  to 1920×1080 at 60 Hz when the monitor was connected.
- Browser JPEG preview: 640×360, HTTP 200, clean image inspected locally.
- Owner visually confirmed live HDMI, “NO SIGNAL” after unplugging goggles,
  and live recovery after reconnecting. The monitor was then disconnected.
- Stream history recorded the loss and recovery. Temperature was around 45 °C.
- An initial regression came from kmssink/libdrm searching unrelated DRM drivers
  longer than the new input timeout. Setting `driver-name=vc4` eliminated that
  probe delay; observed steady playback then used one decoder until hotplug or
  USB reconnection.
- The release installer replaced the persistent systemd application successfully.
  Legacy root is now writable so settings and Wi-Fi changes survive reboot.

Automated checks: 15 Rust tests, Clippy with warnings denied, TypeScript/Rsbuild,
and an HTTP integration test covering upload/conversion/selection, controls,
settings, captured H.264 decoding, preview and expiry after signal loss.

The custom SD image uses the official September 15, 2026 Raspberry Pi OS Lite
ARM64 Trixie image, SHA-256
`cdf4f3bfac35ae947b46e4e767f935453810549779ac3290e05a6754aee627e5`.
Its build validates systemd services, user/sudo permissions, NetworkManager's
AP keyfile, required GStreamer plugins and the ARM64 HTTP server under QEMU.
Filesystem extraction through debugfs drops special mode bits; the build now
restores those bits from the original inode metadata before packaging.

A physical boot of the new image and V4L2 hardware decoding on its 6.18 kernel
remain untested. The new v0.2 renderer has been tested with O4; the owner's O3
confirmation predates this renderer update. Manual HDMI mode selection and the
new image's NetworkManager AP still need physical checks on that image.

The legacy hostapd Wi-Fi API was also exercised with the current SSID and
password. It applied successfully, returned no error, and the AP remained
reachable. The decoder-start count stayed unchanged across the network restart.

### Image login defaults, September 18

At the owner's request, newly built images now use the documented defaults:
Wi-Fi SSID `DJI-HDMI`, Wi-Fi password `djistreamer`, and SSH `root` / `root`.
The separate credential file and generated `dji` account have been removed.
Image validation checks the NetworkManager password, the root password hash,
and sshd's effective root/password-login policy. SSH host keys are still
created separately on each device's first boot. Application-only updates
continue to preserve the installed device's existing credentials.

### Project rename: ungoggled, September 18

The project, application executable, browser UI, USB product label, image
hostname and downloadable artifacts are now named `ungoggled`. The source
checkout directory remains `dji-hdmi`. Existing service names, data directories,
USB gadget paths, environment variables and HTTP control headers remain stable
so installed devices retain their configuration and update compatibility.
The release installer restores the previous service definition during rollback,
including its old executable name.

Fresh image defaults are Wi-Fi SSID `ungoggled`, Wi-Fi password `ungoggled`,
and SSH username `root` with password `ungoggled`. These supersede the defaults
recorded above. Installing an application update preserves existing credentials.

Rename checks: all 15 Rust tests, Clippy, the web build and the captured-video
HTTP integration test passed. In an isolated ARM64 image, the release installer
was exercised with mocked service/HTTP commands: a failed renamed-executable
restart restored the old service and release, and a successful update installed
`ungoggled` while preserving the existing runtime and Wi-Fi configuration.

## HEVC (DJI Osmo Nano) USB-playback decode, 2026-10-31

Choppy/slow HDMI playback of HEVC clips (not the live H.264 goggles feed) on
a real 2GiB Pi 4 traced to two independent, now-fixed issues. Not a kernel
regression and not fragmentation from boot, as earlier same-day working
theories in this investigation assumed; both were superseded by the
following, verified on real hardware.

**Root cause 1 — CMA pool too small for GStreamer's own worst-case
allocation.** A V4L2 stateless-API client (`v4l2slh265dec`) pre-sizes its
capture-buffer pool off the stream's *declared* H.265 level before the
decoder starts. DJI Osmo Nano 3840x2880 10-bit footage declares level 5.2,
underselling its real resolution, so that pre-sized pool alone reaches
~440 MiB — which, plus the decoder's own ~45 MiB of working buffers,
doesn't fit this image's default 512 MiB CMA pool (`total_count=131072`
pages in the kernel's CMA tracepoints; an earlier note here said 256 MiB,
which was wrong), so `STREAMON` reliably fails with `-ENOMEM`:

```
cma: __cma_alloc: linux,cma: alloc failed, req-size: 3072 pages, ret: -16
rpi-hevc-dec feb00000.codec: Failed to alloc 0x300000 Coeff0 buffer
```

Fix (deployed, and in `scripts/build-image.sh` for every future image):
raise the default CMA pool to 640MiB via `dtoverlay=vc4-kms-v3d,cma-size=0x28000000`
in `config.txt`. Verified: the same clip that reliably fails at the default
512MiB decodes in real time at 640MiB, ~9% CPU of one core. **768MiB does not
work on this 2GiB Pi** — it overflows the space available below the 1GiB
boundary the HVS can scan out from, and the reservation silently falls
back to ~8MiB instead (`OF: reserved mem: failed to allocate memory for
node 'linux,cma': size 768 MiB`), which is far worse than the default. Do
not raise this without testing on real hardware first.

A second, architecturally cleaner fix also exists but is not deployed: a
kernel patch (`drivers/media/platform/raspberrypi/hevc_dec/`) that makes
the driver actually honour a `memory-region` device-tree property for its
own internal buffers (it currently silently ignores one — an overlay
reserving a separate pool for the decoder has no effect today). Prepared
as a PR for `raspberrypi/linux` (`rpi-6.18.y`), pending a Signed-off-by.

**Root cause 2 — thumbnail generation and USB playback could run the
decoder concurrently.** `storage.rs`'s background thumbnailing and
`video.rs`'s USB playback had no coordination; both can trigger an HEVC
decode through the same CMA pool. Reproduced on real hardware: starting
both at once segfaulted mpv (`MPV_EXIT=139`) with a live
`cma: __cma_alloc` failure in dmesg, even at the 640MiB pool size — the
free memory at that point was fragmented into small chunks, no single
contiguous run large enough for the request, so a bigger pool alone would
not reliably prevent this. Fix (deployed): thumbnail generation now stands
aside entirely while a USB video is playing, and stops partway through its
pass if playback starts, picking up anything still missing on a later
rescan (`Store::spawn_thumbnailing` in `src/storage.rs`). That check only
runs *between* files, though, so the thumbnail already in progress kept its
~440 MiB for up to 30 s after playback started — see the follow-up below,
which removed thumbnails from the hardware decoder altogether.

## USB playback follow-up: audio-only playback, choppy picture, 2026-10-31

Found by driving the real service through its HTTP API with a real Osmo
Nano plugged in (18 clips: 12 at 3840x2880 29.97 fps, 6 at 3840x2880
50 fps, all HEVC Main 10, ~120 Mbit/s, 3 with a 270° rotation flag).

**"mpv spawned by the service never plays" — CMA exhaustion by the app's
own thumbnailer, not DRM master / logind.** Every service restart empties
`/run/dji-hdmi` (`RuntimeDirectory=`), so the first video listing starts a
thumbnail pass over every clip (~20 s each), each one a hardware
`v4l2slh265dec` decode holding ~440 MiB of CMA. A play request during that
pass started mpv while one was still running; mpv's own log (only visible
with `--log-file`, because `--no-terminal` silenced it) showed
`vo/drm: Cannot create dumb buffer: Cannot allocate memory` →
`Error opening/initializing the selected video_out (--vo) device` →
`Video: no video`, after which mpv played the audio track only and then
went idle — exactly the reported symptom (no DRM fd, `vo-configured=false`,
`idle-active=true`, low CPU). Manual mpv runs over SSH worked because by
then the thumbnailer had stood aside (playback was "active"). The app's
`Display` was never the problem: it drops DRM master right after its
modeset. Fixes (`src/storage.rs`, `src/video.rs`): thumbnails are decoded in
software (`GST_PLUGIN_FEATURE_RANK` demotes `v4l2slh265dec`, `v4l2h264dec`,
`v4l2jpegdec` for that subprocess only; same thumbnail, ~2.5–4.5 s vs.
~3 s, zero CMA) at `nice 10`; and mpv no longer runs with `--no-terminal`,
so its errors reach the journal.

**Choppy picture with "0 dropped frames" — software rendering at 4K,
not a decode problem.** `--vo=drm` can only scale in software, so
`--hwdec=auto` meant `drm-copy` (every 11 MP 10-bit frame copied back to
RAM) plus zimg scaling, at the monitor's preferred mode (here 3840x2160
@30). Measured on screen via `drm:drm_vblank_event_delivered` tracepoints:
6.2 frames/s at 0.14x speed for a 50 fps clip (mpv slowed its whole clock
instead of dropping, hence the clean drop counters). Fix (`src/video.rs`):
zero-copy DRM PRIME frames shown on an HVS plane, which also scales them:
`--vo=gpu --gpu-context=drm --gpu-hwdec-interop=drmprime-overlay
--drm-drmprime-video-plane=primary --drm-draw-plane=overlay
--hwdec=drm,v4l2m2m --vd-lavc-o=threads=4 --framedrop=no --profile=fast
--video-rotate=no`, plus `--drm-mode` set to the mode the app's own display
was using (`Display::mpv_mode`). Each part was needed:

- The overlay interop's default plane roles fail every atomic commit with
  `EINVAL` (it sets the video overlay plane's zpos to 0, out of vc4's range);
  swapping the roles fixes it.
- `threads=4` (frame threading) roughly doubles the hardware decoder's
  throughput through FFmpeg; output is bit-exact with software decoding
  (61/61 frames, showinfo checksums).
- With the default `--framedrop=vo`, a clip the decoder can't quite sustain
  made mpv drop 2 of every 3 frames (15 frames/s on screen); `no` shows them
  all, slightly slower.
- The HVS can't rotate by 90/270°, so honouring a rotation flag squeezed the
  picture into a portrait rectangle; such clips now play as recorded.
- `--drm-mode`: mpv otherwise switches to the monitor's preferred mode (4K
  at 30 Hz here). It matches the refresh to 0.01 Hz and fails the video
  output on no match, hence the exact `1920x1080@59.94`-style string.

Result through the service (1080p59.94): 29.97 fps clips play in real
time, every frame shown for exactly 2 vblanks, A/V in sync, mpv ~45% CPU;
50 fps clips show ~42 frames/s with nothing dropped, i.e. ~0.85x speed
(the audio, which goes to the 3.5 mm jack, drifts ahead) — the decoder
tops out at ~45 frames/s at this size through FFmpeg (~54 through
GStreamer); H.264 1080p60 (bcm2835-codec, zero-copy) plays at a perfect
1-vblank cadence. Peak CMA during playback ~360 MiB of 640.

**The `bcm2835-codec: dma alloc of size 1843200 failed` seen during
playback is not contention or fragmentation.** The kernel's own dump says
`ret: -4` (`-EINTR`) with `=> 154940 free of 163840 total pages` and a
single 605 MiB free run: `alloc_contig_range()` gives up with `-EINTR` when
the allocating task has a fatal signal pending, i.e. the app was SIGKILLing
that `gst-launch` pipeline (`Player::drop`) while it was in `REQBUFS`.
Reproduced on demand (identical messages, incl. the 1843200-byte size, 3
in 40 start-then-SIGKILL cycles of a `v4l2h264dec` pipeline) with >600 MiB
contiguous free each time. Harmless (the process is dying anyway), and
independent of the CMA configuration.

**Fix B (patched `rpi-hevc-dec` + 128 MiB `hevc_cma` region) evaluated on
the hardware, not deployed.** Same tests, fresh boots, Fix A (640 MiB)
alone vs. Fix A + B: zero-copy mpv HEVC playback plus a concurrent
bcm2835-codec H.264 decode ran without any CMA failure on both (lowest
free 271.8 vs. 318.6 MiB); with the old 440 MiB hardware thumbnail
pipeline added, *something* failed on both — mpv's decoder on A (software
fallback), the thumbnailer on A+B — because the total demand exceeds the
pool either way; the EINTR reproduction was unaffected. Fix B buys ~47 MiB
of headroom the app no longer needs (nothing in it takes the 440 MiB
pipeline anymore), at the cost of an out-of-tree module tied to one exact
kernel build. Keep Fix A as is; the driver patch is still worth
upstreaming, after which the overlay alone would be a free option.

## USB playback of DJI Action 2 4K60 clips, 2026-11-01..06: established vs. retracted

Reported in the field as 4K60 Action 2 footage stuttering. A long live session
on 2026-11-01/02 reached two wrong conclusions; a parallel investigation by
another agent (Gemini "Argon", handoff at
`H:\Gemini Argon\Claude Code\HANDOFF-ACTION2-4K60.md`, which measured with the
DRM vblank tracepoint and mpv's IPC `playback-time` instead of logs and
eyeballing) falsified them on 2026-11-03. Anything marked RETRACTED is not true.

**RETRACTED - "CMA leak of ~115 MiB per stop".** `CmaFree` read ~460 MB before
playing `DJI_0023.MP4` (3.4 GB), swung between 0 and ~100 MB while playing and
settled near 340 MB after stopping. That is the Linux *page cache*: reading a
3.4 GB file over USB fills free CMA pages, and the kernel gives them back only
under pressure. `echo 1 > /proc/sys/vm/drop_caches` took `CmaFree` from 44 kB
to 561584 kB straight away. The earlier HEVC investigation had already said
`CmaFree` is misleading here (tracepoints showed zero outstanding allocations
after every run). Never read `CmaFree` without dropping caches or using the
`cma:cma_alloc_*` tracepoints.
Consequence: the change to `UsbPlayer::drop` in `src/video.rs` (2026-11-02:
send IPC `quit`, wait up to 800 ms, then `kill()`) rests on this wrong
diagnosis. After it, stopping playback showed the Linux console for 20-30 s
before the idle screen returned, which did not happen before. To be reverted
to a plain `kill()` + `wait()` and re-tested. Mechanism unconfirmed (guess: mpv's
DRM context restores the original console CRTC on a clean exit, which SIGKILL
never allowed).

**RETRACTED as a cause - "DJI_0023.MP4 declares HEVC Level 5.0 but needs 6.0".**
The fact is real (`general_level_idc = 150` in the `hvcC` box; 4096x3072 =
12,582,912 samples exceeds the Level 5.x cap of 8,912,896), and it is the same
class of quirk as the Osmo Nano above. But it does not explain slow playback:
GStreamer's `v4l2slh265dec` decodes this exact file at 50.8-53.2 fps in
hardware, and FFmpeg/mpv size their buffers from width x height, not the
level. It also fits the user's report that the clip played fine on ~0.3.40.

**"8 fps" was an eyeball estimate**, not a measurement. Measured: mpv with the
app's flags plays this clip at ~0.68x (34.6 flips/s on the vblank tracepoint,
`playback-time` advancing 11.68 s per 17.0 s). On the Acer (HDMI-A-2) at
3840x2160@30 a 60 fps clip with `--framedrop=no` is 0.5x by construction.
`--audio=no` (added 2026-11-01) does not speed anything up: with no audio
clock mpv simply slows its own clock (hence `frame-drop-count: 0`); with the
audio track on, the audio clock runs in real time, the decoder falls behind
and the demuxer queue overflows (`Too many packets in the demuxer packet
queues`). The all-P-frame GOP means the decoder cannot skip frames before
decoding.

**Established - black screens every few seconds on oversized clips.** (The
`HDMI_VID_CTL` readings in this paragraph are RETRACTED, see the 2026-11-06
section below; the clock cause is confirmed there by other means.) Cause
(measured in `hdmi0_regs`/`hdmi1_regs`/`hvs_regs` and `vcgencmd measure_clock
core`): with hardware decode the ARM cores are mostly idle, so the firmware's
`ondemand` governor drops the VideoCore/HVS core clock from 500 MHz to
199-250 MHz mid-video. `vc4` computes only ~148.5 MHz of core clock need at
the initial modeset (`HVS State: Core Clock Rate: 148500000`) and, per Argon's
reading, does not raise it on later plane updates. Scaling 4096x3072 to
1440x1080 (2.84x vertical) makes the HVS switch from the PPF scaler to the TPZ
scaler, which needs far more line-buffer bandwidth; the HDMI pixel FIFO then
underflows (`HDMI_VID_CTL` = `0xc0080000`, bit 19; `0xd8000000` = underflow +
`BLANKPIX` = black picture). Fix: `core_freq_min=500` in `/boot/firmware/
config.txt` keeps the clock at 500 MHz (`HDMI_VID_CTL` stays `0xc0000000`).
Only applied by hand on the test Pi; not yet in the image build. The proper
fix would be in `vc4` (candidate upstream bug; needs a minimal reproduction
without this app first).

**Do not retest:** `hevc_freq=650` without `over_voltage` freezes the HEVC
decoder (`feb00000.codec`) a few frames in.

**Still open:** why mpv gets ~35-41 fps from this clip where GStreamer gets
~51; how to get 1.00x playback by dropping frames at the VO instead of
slow motion (ideas from Argon, unmeasured: a dummy audio master clock via
`--lavfi-complex=anullsrc` with `--framedrop=vo`; a larger demuxer buffer); what
in *this app's* code changed since ~0.3.40 (old release bundles saved in
`H:\Claude Code\pi-backup-2026-11-06\pi-old-releases.tgz`); whether to force a
fixed 1920x1080@60 output for every input; the recurring `[drm] *ERROR*
flip_done timed out` at boot and late in a long session (unexplained; possibly
the same clock/underflow problem, unverified).

**Test hygiene (cost hours):** `systemctl stop dji-hdmi` unmounts
`/run/dji-hdmi/usb-media/*` (device letters change afterwards, paths go stale)
and blanks the display. The service drops DRM master right after its modeset,
so mpv can simply be started alongside it. The test Pi has two monitors
attached (HDMI-A-1 BenQ 1080p60, HDMI-A-2 Acer 4K; the user watches the Acer):
pass `--drm-connector` or the picture may appear on the other one.

## Re-measured on the clean 0.3.42 image, 2026-11-06

Fresh image, Acer XV275K V on HDMI-A-1 (`pixelvalve-2`), BenQ GL2460 on
HDMI-A-2 (fbcon clone at 1080i). mpv started over SSH with the app's exact
flags (`--drm-mode=1920x1080@59.94 --drm-connector=HDMI-A-1`, `--start=20`),
alongside the running service. Per run: speed = delta IPC `playback-time` /
delta monotonic wall time; flips/s from `drm:drm_vblank_event_delivered`;
`SCALER_DISPSTAT` EUFLOW bits (HVS FIFO underflow status, per channel),
`HDMI_VID_CTL` and `vcgencmd measure_clock core` sampled at 10 Hz. The vc4
debugfs directory number changes between boots (`dri/0` or `dri/1`); use
`/sys/kernel/debug/dri/gpu/`. Scripts: `/root/bench/` on the test Pi.

**Baseline.** `DJI_0023.MP4` (4096x3072 HEVC 8-bit, 59.94 fps): 0.567x,
34.0 flips/s in every run, stock and with `core_freq_min=500`.
`DJI_..._O4.MP4` (3840x2880 HEVC Main10, 59.94 fps): 0.75-0.77x, 45-46 flips/s.

**Black screens / core clock: cause confirmed, register evidence corrected.**
Stock (`ondemand`): core clock 274-500 MHz during playback; EUFLOW was set in
6 samples, all at a core clock <= 324 MHz, none at 500 MHz (about 550 samples).
`performance` governor: core pinned at 500 MHz, 0 EUFLOW. `core_freq_min=500`:
core 499-500 MHz even while the ARM ran at 900 MHz, 0 EUFLOW in 645 samples.
Playback speed is the same in all three, so the clock only affects the
display, not decoding. Why vc4 does not ask for more (read in `rpi-6.18.y`
`vc4_plane.c` / `vc4_kms.c`): a plane's `hvs_load` is `crtc_w * crtc_h *
vrefresh` - the source size of a downscaled plane is ignored - and the core
clock request is `max(per-channel FIFO load ~ pixel clock, 40-60 % of the
summed plane loads)` = 148.5 MHz here; the 500 MHz floor is applied only
during a modeset. vc4's HVS underrun interrupt reporting is compiled out on
this branch (`if (0 && ...)` in `vc4_crtc.c`), so debugfs `hvs_underrun` is
always 0 and proves nothing.
RETRACTED (from the section above): `HDMI_VID_CTL` bit 19 (`0xc0080000`) is not
an underflow flag - it toggles in ~8 % of samples on both outputs while idle,
with no scaling at all. `0xd8000000` is ENABLE | UNDERFLOW_ENABLE | VSYNC_LOW |
HSYNC_LOW (`vc4_regs.h`), i.e. a mode with negative sync polarity, not
BLANKPIX (bit 18). A black picture itself was not watched in this session; the
conclusion rests on EUFLOW vs. clock plus the user's earlier visual report.
`core_freq_min=500` is now written by `ungoggled system-fixes` (image build
and every update, see "System fixes through the update" below).

**mpv ~34 fps vs. GStreamer ~51: the scanout costs decode throughput, not
mpv.** GStreamer's 51 fps is with `fakesink` (nothing scanned out); re-measured
50-51 fps on this boot. The same mpv command reaches 50.6 flips/s (0.843x) when
`--video-crop=1920x1080+1000+1000` makes the HVS read only a 1920x1080 part of
each frame, 42.7 with `--video-crop=4096x1536+0+0`, 34.0 with the full frame;
a 1920x1080@50 output gives 37.4 (0.623x). Decode speed falls with the amount of
frame data the HVS reads per second - consistent with SDRAM bandwidth
contention between scanout and the HEVC decoder (inferred, not measured on the
bus). No mpv option changed it: `threads=1/2/4/8` 21/33/34/34 flips/s;
`--hwdec-extra-frames=16/32`, `--vd-queue-enable`, `--swapchain-depth=1`,
`--video-sync=display-desync`, `--untimed`, `--vd-lavc-skipframe=nonref` all
34.0-34.2. One GStreamer run gave only ~35 fps right after a killed paused mpv;
the next runs were 51 again, so single runs need care.
**1.00x is not reachable for `DJI_0023.MP4` on a Pi 4:** the decoder alone
tops out at ~51 fps (< 59.94) and every frame is a reference
(`skipframe=nonref` has no effect), so frames cannot be dropped before
decoding. Measured attempts: real audio track + `--ao=null`: video still 0.56x,
A/V offset grows to ~10 s within 20 s; plus `--framedrop=vo`: same speed but
only 15-17 flips/s (choppier); `--lavfi-complex=...anullsrc...` as master clock:
mpv loses hardware decoding (software fallback, 5 fps); EGL/GPU scaling
(`--gpu-hwdec-interop=drmprime`): 0.62x for `DJI_0023`, but 0.43x for the
10-bit clip. Kept: `--framedrop=no --audio=no` (smooth slow motion).

**Output mode.** `--drm-mode=3840x2160@30.00`: both clips exactly 0.50x
(30 flips/s); 1920x1080@59.94: 0.57x / 0.75x. The app in automatic mode already
uses 1080p (max 60 Hz) for the idle picture and hands that mode to mpv, so no
code change; 4K30 only happens if the user picks it in the settings.

**"Played fine on ~0.3.40": not reproduced.** The mpv arguments in the 0.3.38,
0.3.40 and 0.3.41 binaries are identical to 0.3.42 except `--audio=no` (new in
0.3.42). The 0.3.40 binary, run as a throwaway instance on this Pi and driven
through its HTTP API: 0.53x video (from the reported position) with mpv's
"Audio/Video desynchronisation detected" (audio at 1x on the jack). The old
Pi's kernel/firmware state can no longer be reproduced.

**Console after stopping a USB video: 20-30 s not reproduced.** Throwaway app
instance (own data/runtime dir, 127.0.0.1:8099; the real service stopped
meanwhile), play/stop through the HTTP API, 3 cycles each, scanout read from
debugfs state at 20 Hz: 0.3.42 as imaged (IPC quit, up to 800 ms): fbcon
visible from 0.32-0.40 s, app picture back at 0.65-0.83 s; with plain
`kill()`: fbcon from 0.28-0.31 s, back at 0.57-0.59 s. No `flip_done` timeout
in any boot of this session. The short console flash exists because the app
closes its DRM fd before mpv starts, so mpv's exit is the device's last close
and the kernel restores fbcon (at 1920x1080@60.00 while the app uses @59.94:
two mode changes, each may make a monitor resync).

**Display hand-over (two healthy monitors).** `chvt 2`: fbcon on *both*
outputs within 80 ms, including the app's; after `chvt 1` the console stays on
the app's output until the app rebuilds its display (it does not notice).
Emulated unplug of HDMI-A-1 (`echo off > .../status`): the other output is
unaffected; after `echo detect` fbcon takes HDMI-A-1 within 0.2 s and the app
(connector 35 chosen in the settings) never takes it back. Physical unplug not
tested.

**Kernel patch (`rpi-hevc-dec` memory-region), kept apart from the above.** The
installed module matches `rpi-6.18.y` commit `f2bc178ed43d` (identical function
sizes and `.text` vs. the shipped `.ko`). The patch applies to it (offset -43)
and cleanly to the branch head `a1a601d48579` (2026-10-15); both build without
compiler warnings against 6.18.50+rpt. Fresh boots, GStreamer
`v4l2slh265dec` on `DJI_0023.MP4`, `cma_alloc_finish` tracepoints: stock: every
allocation from `linux,cma`; stock + 128 MiB `memory-region` overlay: region
reserved and present in the DT, still every allocation from `linux,cma` (the
bug); patched + overlay: the decoder's internal buffers (16-, 1024-, 4096-page
allocations) from `hevc_cma@0`, frame buffers still from `linux,cma`, 48.9 vs
48.8 fps; patched without overlay: identical to stock. Log fix, CMA hogged
until STREAMON fails: stock prints "Failed to alloc 0x400000 Coeff0 buffer" for
a failed 4096-page (0x1000000) request; patched prints "Failed to alloc
0x1000000 Coeff2 buffer". raspberrypi/linux PR #6666 is the merged (2025-03)
backport of this very driver into `rpi-6.12.y`; mainline has no `hevc_dec` yet.
The test Pi runs the stock module again (overlay and `updates/` module removed).

## USB vs. SD card, output size and 4K60, 2026-11-07

Question from the user: is the camera's USB port the bottleneck? Measured on
the test Pi (0.3.43 candidate, one monitor: Acer XV275K V on HDMI-A-1).
`DJI_0023.MP4` is 3,633,995,773 B / 276 s = 13.2 MB/s on average (the O4 clip:
2,062,365,602 B / 122 s = 16.9 MB/s). mpv with the app's exact flags and
`--start=20`, started next to the service, 5 s warm-up then a 12 s window,
page cache dropped before every run; speed = delta IPC `playback-time` / delta
wall time, flips/s from `drm:drm_vblank_event_delivered`. Core clock 500 MHz in
every run, 0 dropped frames. Scripts (gone after the next reflash):
`/root/bench.py`, `/root/runbench.sh`.

| source | output mode | speed | flips/s |
|---|---|---|---|
| camera USB | 1920x1080@59.94 | 0.592x | 35.5 |
| copy on the SD card | 1920x1080@59.94 | 0.591x | 35.4 |
| camera USB | 640x480@60.00 | 0.804x | 48.3 |
| copy on the SD card | 640x480@60.00 | 0.803x | 48.1 |

- **USB is not the limit** for either clip today: the SD copy plays exactly as
  fast. Raw read speed of the camera (USB High-Speed, 480 Mbit/s, `dd
  iflag=direct`) is 18-23 MB/s; copying the whole file took 170 s = 21.4 MB/s;
  the SD card reads 45.5 MB/s. Headroom: `DJI_0023` needs 13.2 MB/s at 1.0x, but
  the O4 clip needs 16.9 MB/s - if decoding ever became fast enough for it,
  the camera's USB would be the next limit.
- **The output mode matters:** 640x480@60 is 36 % faster than 1920x1080@59.94
  with the same source (0.80x vs 0.59x). That extends the 2026-11-06 finding
  (decode speed falls with what the display reads): the output size counts too.
  Mechanism not verified. The decoder alone does ~51 fps (GStreamer, fakesink),
  so ~0.85x is the ceiling for this clip (4096x3072 at 60 fps is ~755 Mpixel/s,
  the Pi 4's nominal HEVC rating is 4Kp60 = ~498 Mpixel/s).
- **mpv's own speed matches the screen:** 0.592 x 59.94 fps = 35.5 flips/s. So
  `container-fps x (delta playback-time / delta wall time)` from the IPC socket
  is a trustworthy "fps that reaches the screen" figure for an in-app readout.
  The web UI shows nothing during USB playback today: mpv runs as its own
  process and does not feed `input_fps`/`output_fps` (`/api/status` stayed at
  `hdmi: fallback`, 0/0 while a clip played).

**4K60 output is switched off in our config.** The kernel logs at boot `[drm]
The core clock cannot reach frequencies high enough to support 4k @ 60Hz.
Please change your config.txt file to add hdmi_enable_4kp60.`; `vcgencmd
get_config hdmi_enable_4kp60` is 0; the monitor's mode list on HDMI-A-1
(`mpv --drm-mode=help`) has 3840x2160 only at 30/29.97/25/24/23.98 Hz although
the monitor can do more. Nobody decided this: the image uses the stock
Raspberry Pi OS `config.txt` plus our marked block, and the app's automatic
mode picks 1080p.

**Tested the same day with `hdmi_enable_4kp60=1` (test Pi, one monitor, SD copy
of `DJI_0023.MP4`, same method).** The firmware then reports `core_freq=550`
(`measure_clock core` 549.99 MHz) and `hdmi_pixel_freq_limit` 600 MHz for HDMI0
but only 162 MHz for HDMI1, i.e. 4K60 is HDMI0 (= HDMI-A-1) only, as the user
had read. The kernel warning is gone and the Acer's list gains 3840x2160 at
60/59.94/50 Hz.
- 1920x1080@59.94: 0.623x, 37.4 flips/s (was 0.592x / 35.5 at 500 MHz), core
  516-550 MHz, 55 C, `throttled=0x0`. About 5 % better.
- **3840x2160@60 with the app's mpv flags is unusable:** 0.048x, 2.7 flips/s,
  `[drm] *ERROR* flip_done timed out` / `commit wait timed out` on pixelvalve-2
  and the planes every 10 s, the monitor then showed "resolution not supported"
  and no picture. Killing mpv did not recover it (defunct mpv, IPC timeouts in the
  next runs; `systemctl restart dji-hdmi` did not help either, flip_done kept
  timing out); only a reboot did. Cause not separated: either the link (HDMI
  2.0 needs an 18 Gbit/s cable and the monitor input to accept it) or the HVS
  bandwidth of two full-screen planes (video + mpv's ARGB overlay) at 4K60.
  Not tried again on purpose - it blanks the user's monitor.
- Consequence: do not ship `hdmi_enable_4kp60=1` just for the 5 %. It would also
  put 4K60 modes into the mode list of the web UI, and picking one blanks the
  screen. The test Pi keeps the line for now (backup: `config.txt.bench-orig`).

## Display bandwidth and HEVC clock, 2026-11-07 (evening)

Goal: real time (1.00x), smooth, for `DJI_0023.MP4` (DJI Action 2, 4096x3072
HEVC Main 8-bit, 59.94 fps). Test Pi 4B 2 GB, 0.3.44 as shipped (kernel
6.18.50+rpt pinned, patched `rpi-hevc-dec`, `core_freq_min=500`), one monitor
(Acer XV275K V, HDMI-A-1). Copy of the clip on the SD card (`/root`), page
cache dropped before every run, the service's own (paused) mpv quit first.
Method (`bench2.py`): mpv with the app's exact flags plus the listed extras,
`--start=20`, 5 s warm-up, then a **60 s** window: speed = delta IPC
`playback-time` / delta wall time, also per 10 s segment (no drift in any run:
segments within +-0.01x, except the first segment of the first run after a
boot), page flips from `drm:drm_vblank_event_delivered` with a histogram of
vblanks per flip, `vcgencmd measure_clock core/hevc/v3d/arm`, temperature,
`get_throttled` (0x0 in every run) and new kernel messages (none in any mpv
run). Results also in `/root/b2/results.jsonl` on the test Pi (until reflash).

### Bitstream structure (parsed from the file, `gop.py`)

Tracks: `hvc1` video (16,566 samples, 60000/1001 fps, no `ctts` = no
reordering), `mp4a` audio, two DJI metadata tracks (`djmd`, `dbgi`); there is
no MJPEG stream in this file. Sync samples every **30 frames (0.5 s)**, all
gaps 30. Each picture is two slice segments. NAL types in the first 400
samples: `IDR_N_LP` (I) at every sync sample, everything else `TRAIL_R` (P),
`nuh_temporal_id` 0 everywhere, no TSA/STSA/RASL/RADL/CRA. So: a strict
I-P-P-...-P chain of 30 frames, no droppable pictures, but a fresh IDR every
0.5 s. Average sample 208 KB, IDR ~450 KB, largest 1.2 MB.

### What the display costs, at the stock 500 MHz HEVC clock (1920x1080@59.94 unless noted)

| mpv extras | speed | flips/s | vblanks per flip (1/2/3) |
|---|---|---|---|
| none (app today) | 0.593 | 35.6 | 668/1461/3 |
| `--drm-draw-surface-size=960x540` | 0.624 | 37.4 | 915/1308/19 |
| `--drm-draw-surface-size=320x180` | 0.648 | 38.9 | 1063/1267/0 |
| `--video-zoom=-0.08` | 0.721 | 43.2 | 1590/1001/2 |
| zoom -0.08 + surface 960x540 | 0.755 | 45.3 | 1847/861/9 |
| zoom -0.08 + surface 320x180 | 0.772 | 46.3 | 1965/805/7 |
| same, 1920x1080@50.00 output, surface 960x540 | 0.777 | 46.6 | 2586/206 (at 50 Hz) |
| same, 1920x1080@50.00 output, surface 320x180 | 0.792 | 47.5 | 2697/152 (at 50 Hz) |
| 640x480@60, surface 320x180, `--video-crop=256x256+0+0` (almost nothing scanned out) | 0.884 | 53.0 | 2800/344/36 |

- **Why 640x480 was faster (open question of the morning, now answered from
  the source):** `vc4_plane.c` (`rpi-6.18.y`) reads only every Nth source line
  when a plane is scaled down vertically by more than 3x
  (`vdownsample = src_h / (crtc_h * 3) + 1`, "Reduce the number of lines read
  to avoid exceeding SDRAM bandwidth"), and the SAND128 path programs it
  (`SCALER_TILE_SKIP_0`). 3072 -> 1080 is 2.84x, so every one of the 3072 lines
  is read 60 times a second; 3072 -> 480 is 6.4x (every 3rd line). Shown 1020
  lines high instead of 1080 (`--video-zoom=-0.08`, picture 5.6 % smaller,
  debugfs `crtc-pos=1362x1020+276+30`) the ratio passes 3 and the display
  reads every 2nd line: 0.593x -> 0.721x. Vertical quality: the scaler sees
  1536 lines for 1020 output lines, not visibly different on the Acer (not
  judged by the user yet).
- **mpv's draw plane** (`--drm-draw-plane=overlay`, carries only the OSD,
  which the app never uses) is a full-screen ARGB buffer at the output size;
  the GPU redraws it per frame and the HVS reads it every refresh. A smaller
  surface (`--drm-draw-surface-size`, mpv upscales it on the plane) gives
  +5 % (960x540) to +9 % (320x180), and +5 %/+7 % on top of the zoom.
  Not smaller than 960x540 in the app: a clip mpv decodes in software is
  rendered into this plane at that size (mpv `vo.rst`).
- The video-zoom can be set after start over IPC (`set_property video-zoom`):
  same 0.755x as the command-line flag, plane at 1358x1018. mpv 0.40 reports
  `video-params/w|h` (4096x3072) and `display-width|height` (the DRM mode,
  1920x1080; `osd-width` is the draw surface size instead).
- 50 Hz output: +3 % and a more even cadence (mostly 1 vblank per frame, but
  still a 2-vblank repeat every ~0.2 s), not real time either.
- **The O4 clip (3840x2880 Main10, from the camera's USB):** app flags 0.794x,
  surface 960x540 0.824x, plus `--video-zoom=-0.18` (956 lines, 11.5 %
  smaller) 0.829x - the line skip does not help this clip (its limit is
  elsewhere; at 0.83x it already reads 14 MB/s from the camera, whose USB does
  18-23 MB/s; not separated). The app therefore only zooms when the cost is
  <= 10 % (`src/lineskip.rs`), which excludes this clip.

**RETRACTED (from the 2026-11-06/07 sections): "the decoder alone tops out at
~51 fps" as the hard ceiling.** That was GStreamer `v4l2slh265dec` ->
`fakesink`. mpv (FFmpeg v4l2request, frame threading) with almost nothing
scanned out reaches 53.0 fps at 500 MHz. A GStreamer run tonight on the
*first* 1800 frames of the file gave only 38.5 fps (42.0 at 600 MHz), i.e.
that number depends on the passage (and maybe start-up) and is not a
comparable ceiling; the mpv figures are all from `--start=20`.

### The HEVC clock is the real limit (boot-level tests, test Pi only, all reverted)

`rpi-hevc-dec` asks the firmware for its maximum HEVC clock while it decodes
(`hevc_d_hw.c`: `clk_set_min_rate(dev->clock, max_clock_rate)`, the max read
from the firmware at probe), so `hevc_freq` sets the decode clock directly;
`measure_clock hevc` read 500 MHz throughout every stock run. Raised with
`over_voltage_delta` (DVFS offset; per the Raspberry Pi documentation the
permanent overclock/warranty bit is only set by `force_turbo=1` together with
`over_voltage*` > 0 - `force_turbo` was never used; `/proc/cpuinfo` Revision
stayed `b03115` before and after):

| config.txt (besides the shipped block) | clocks under load | "ceiling" run (640x480, crop) | zoom -0.08 + surface 320x180 at 1080p59.94 | app flags at 1080p59.94 |
|---|---|---|---|---|
| none (shipped) | core 500, hevc 500, v3d 500, 0.926 V | 0.884x / 53.0 fps | 0.772x / 46.3 | 0.593x |
| `hevc_freq=600 over_voltage_delta=25000` | **core 300**, hevc 600, v3d 300, 0.941 V | **1.000x / 60.0 (vsync-capped)** | 0.888x / 53.3 | 0.602x |
| `hevc_freq=550 hdmi_enable_4kp60=1 over_voltage_delta=25000` | core 512-550, hevc 550, v3d 550, 0.941 V | 0.971x / 58.2 | 0.827x / 49.6 | - |
| `hevc_freq=650 hdmi_enable_4kp60=1 over_voltage_delta=50000` | **firmware dead** | - | - | - |
| `hevc_freq=700 hdmi_enable_4kp60=1 over_voltage_delta=50000` | **firmware dead** | - | - | - |
| `hevc_freq=600 hdmi_enable_4kp60=1 over_voltage_delta=25000` | **firmware dead** | - | - | - |

- Decode throughput scales with the HEVC clock (~0.106 fps per MHz at almost
  zero scanout; 53.0 / 58.2 / >= 60 at 500 / 550 / 600 MHz). With the display
  at 1080p (zoom + small surface) about 87 % of that remains, so real time at
  1080p59.94 would need roughly 650-700 MHz with the core clock still >= 500.
- **`hevc_freq=600` alone dropped the core (and V3D) clock to 300 MHz**,
  ignoring `core_freq_min=500` (`vcgencmd get_config` still said core_freq=500
  / core_freq_min=500). Presumably the firmware derives core/V3D/HEVC from one
  PLL and can only reach 600 by giving the core a 300 divider (inferred, not
  documented anywhere I found). A 300 MHz core is exactly the condition under
  which the HVS FIFO underflowed and the screen went black on 2026-11-06
  (EUFLOW only at <= 324 MHz), so this setting is unusable even though it
  decodes faster. Not run long enough to see a black frame.
- **"Firmware dead" (the -110 state):** with `hdmi_enable_4kp60=1` (core 550)
  and `hevc_freq` 600, 650 or 700 the VideoCore firmware stops answering the
  mailbox already during boot: dmesg at ~10 s `vc4-drm gpu: [drm] Couldn't
  stop firmware display driver: -110`, then `raspberrypi-clk ... Failed to
  change fw-clk-arm frequency: -110`, `hwmon ... Failed to get throttled
  (-110)`; every `vcgencmd` blocks in `rpi_firmware_property_list` (D state,
  load average climbs), no HDMI output was detected on either connector (the
  user checked at the Pi), the 650/700 runs never got to a decode. Linux
  itself and SSH kept working, so `config.txt` could be reverted and the Pi
  rebooted; the reboot out of this state took 4-30 minutes (shutdown waits on
  the stuck mailbox calls; the last one ~30 min, until the user was at the
  Pi). Whether 650/700 work *without* `hdmi_enable_4kp60` (but then the core
  drops below 500, see above) or with other clock combinations was not tested
  - stopped on purpose, see below.
- Temperatures 31-61 C over all runs (case as used by the user), no
  throttling; voltage +15 mV with `over_voltage_delta=25000`.

**Decision (coordinator, 2026-11-07): no more boot-level clock experiments on
the test Pi;** a bad combination can leave a Pi without picture, and on a
user's Pi without SSH it would need the SD card edited on a PC. Overclocking
is not proposed for shipping: the one board we have needs >= 650 MHz HEVC with
core >= 500 MHz for real time, and no combination that gives that booted. See
`TODO.md` for what would be needed if the user wants to pursue it anyway.

### Compromise options, quantified

- **Smooth slow motion (shipped approach, improved):** 0.593x -> 0.755x
  (45 frames/s on screen, every frame shown, no drift) with the two mpv
  changes now in `src/video.rs` (`--drm-draw-surface-size=960x540`) and
  `src/lineskip.rs` (IPC `video-zoom` once the clip size and output mode are
  known; only when it shrinks the picture by <= 10 %).
- **Real time by dropping the tail of every GOP (not built):** IDR every 30
  frames, so a player could decode the first frames of each 0.5 s GOP at the
  decoder's pace and jump to the next IDR on time. At 45 decoded frames/s that
  is ~22 of 30 frames per GOP, then the last picture held for ~170 ms, twice
  a second - a visible 2 Hz stutter. mpv cannot do this by itself
  (`--vd-lavc-framedrop`/`skipframe=nonkey` would decode the following P
  frames against missing references, i.e. garbage until the next IDR); it
  would need either keyframe seeks driven by the app (slow motion inside the
  GOP plus jumps, worse) or a custom player. Not recommended.
- **Lower output refresh:** 50 Hz is +3 %; a 24/25/30 Hz mode cannot make a
  60 fps clip faster than the decoder (every frame must still be decoded,
  all P), it only adds frame drops at the output.
- **1080p/30 "compromise":** the clip is 4096x3072 on the card; nothing on the
  Pi can decode it at a lower resolution or rate (all-P chain, no temporal
  layers), so a 30 fps result needs a 30 fps (or 2.7K/1080p) recording on the
  camera, or a transcode on a PC. A 4K 30 fps HEVC clip plays in real time
  here (2026-10-31 section).

### Soak run of the shipped combination

Zoom -0.0825 (the value `lineskip.rs` computes) + surface 960x540, 1920x1080@59.94,
`--loop-file=inf`, 600 s: 45.1 flips/s, vblanks per flip 18259x1 / 8649x2 /
134x3 / 1x4, every 10 s segment 0.747-0.767x (one segment is the loop's jump
back to the start), 0 drops, core/hevc 500 MHz, 54-60 C, `throttled=0x0`, no
kernel messages. Smooth slow motion at 0.75x, not real time.

Sources: Raspberry Pi documentation, config.txt / overclocking
(https://www.raspberrypi.com/documentation/computers/config_txt.html:
hevc_freq default 500 on Pi 4, "SDRAM frequency is not configurable on
Raspberry Pi 4", over_voltage_delta "adds the given offset in microvolts to the
number calculated by the DVFS algorithm", overclock bit only with force_turbo +
over_voltage, core_freq 550 with hdmi_enable_4kp60); popcornmix in the
LibreELEC forum (https://forum.libreelec.tv/thread/22076-rpi4-hevc-playback-max-bitrate:
SDRAM bandwidth is usually the limit for hard HEVC, OSD/GUI overlays cost
bandwidth, hdmi_enable_4kp60 runs the AXI bus at 550 MHz, HEVC clock can be
overclocked); mpv 0.40 `DOCS/man/vo.rst` (`--drm-draw-surface-size`, intended
exactly for drmprime-overlay at high resolutions); `vc4_plane.c` and the
`rpi-hevc-dec` sources of `rpi-6.18.y` (local copies). Not found anywhere:
measured HEVC decode rates vs. `hevc_freq` on a Pi 4, and the firmware's rule
for core/HEVC PLL sharing.

## System fixes through the update (2026-11-06, not released)

Implementation: `src/system_fixes.rs`, called by `prepare-pi.sh` (every
service start) and `build-image.sh` (`--offline --strict`). Design and rules:
`docs/UPDATES.md`, "System fixes".

**Module build is reproducible.** `scripts/build-hevc-module.sh build` on the
test Pi (headers 1:6.18.50-1+rpt1, driver source of `f2bc178ed43d` fetched and
checked against its git blob hashes, patch applied) gave
`rpi-hevc-dec.ko` sha256 `0f1b0f50...0966340`, srcversion
`248BA44E93962CBDB77A832`, vermagic `6.18.50+rpt-rpi-v8 SMP preempt mod_unload
modversions aarch64` - byte-identical to the earlier build in another
directory (`/root/hevc/patched`). Overlay `.dtbo` also byte-identical
(`0b9cd509...`). The stock module's vermagic is the same string.

**First hop through the installed 0.3.42 updater.** Test Pi reset by hand to
the 0.3.42 image state (stock `config.txt` from before the hand edit, no
`updates/` module, no overlay, no hold, rebooted; `vcgencmd get_config
core_freq_min` = 200, decoder without `memory-region`, stock module srcversion
`A8E8382C2538332363EC202`). A 0.3.90 test package (scratch copy, version
bumped) passed the 0.3.42 binary's `check-update`, then `update-helper
update-install --source` (0.3.42 code: same update-worker transaction as the
web UI's Install): "Updated to 0.3.90" after 15 s, health check passed. In
the journal, `prepare-pi.sh` installed module, overlay, config block and the
hold of 10 packages; `/api/status` `reboot_required: true` with reasons
`core_freq_min=500 not active (core_freq_min=200)`, `decoder memory pool
appears after the next boot`, `stock module loaded`. `config.txt` diff vs.
stock: only the 6-line block at the end; `config.txt.ungoggled-orig` equals
the stock file.

**After the reboot.** `core_freq_min=500`; dmesg `created CMA memory pool at
0x78000000, size 128 MiB` + `feb00000.codec:mem: assigned reserved memory
node hevc_cma@0`; loaded module srcversion `248BA44...` from
`updates/ungoggled/`; kernel taint gains `O`; `reboot_required: false`.
GStreamer `v4l2slh265dec` on `/root/hevc-test/test.mp4` (3840x2880 HEVC
Main10, from the old Pi's backup; no USB camera attached) with CMA
tracepoints: decoder-internal allocations (16, 169, 768, 3072 pages) all
from `hevc_cma@0`, frame buffers (1800/3600/5063 pages) from `linux,cma`, 0
failures, 49.7 fps. mpv with the app's HDMI flags: `hevc-drm` hardware decode
(`drm_prime[rpi4_10]`) to the end of the clip.

**Idempotence, rollback.** Same build again: refused by the updater ("This
exact build is already installed"). A 0.3.91 package: installed, the run at
service start changed nothing (sha256 of `config.txt`, its backup,
`modules.dep`, module, overlay and state file identical); steady-state run
0.11 s, a run that installs (depmod + apt-mark) 3.4 s. Switch back to the
factory 0.3.42 through the updater: succeeded; the factory release keeps the
current `prepare-pi.sh`, whose call of the old binary logs "unrecognized
subcommand 'system-fixes'" and continues; module, overlay, config and hold
stay in place and active. Every install/switch also warned
`hdmi_connector: not compatible with this release; default used` - a
pre-existing settings-migration gap (TODO.md), unrelated.

**Failure paths on the Pi.** Kernel mismatch (`--kernel-release
6.18.99+rpt-rpi-v8` on the real root): overlay line removed, module and
`.dtbo` removed, depmod, hold released, `core_freq_min` kept;
`reboot_required` with "pool goes away" / "patched module loaded"; a normal run
restored the identical `config.txt` (same sha256). Module that does not load
(unloaded, then `install rpi_hevc_dec /bin/false` in modprobe.d): switched
off and recorded in `/var/lib/dji-hdmi/system-fixes.json`, files and hold
removed, stays off on the next run; after removing the rule the stock module
loaded; `enable-module` reinstalled everything. Overlay file missing while its
line is present (file moved away, rebooted): the firmware log (`vclog --msg`)
shows only the attempted read, the Pi booted normally without the pool, and
the start-time run reinstalled the file and flagged a reboot. Against a
temporary root on the Pi (copy of `config.txt`, hard-linked `/lib/modules`,
`dpkg` status): the image-style `--offline --strict` run, a second run, a
duplicated block start (file left untouched), a `vc4-kms-v3d` line under
`[pi3]` (CMA left alone), a stock `dtoverlay=vc4-kms-v3d` line (got
`,cma-size=0x28000000`), a missing `config.txt`, and `--strict` with a kernel
mismatch (exit 1); the real `modules.dep` stayed unchanged.

Not tested: the browser upload/Install buttons, a real `build-image.sh` run,
playback from the USB action camera, a 1 GiB Pi 4.


## Line-skip zoom falsifies the colours: dropped, 2026-11-07 (late evening)

The 5.6 % picture shrink from "Display bandwidth and HEVC clock" (video-zoom -0.08 so that the
HVS reads every 2nd source line, `src/lineskip.rs`, built into the 0.3.45 test candidate and
installed on the test Pi through the real updater) is **not usable**:

- Both Action 2 clips (`DJI_0023.MP4` 4096x3072 @59.94 and the 30 fps clip of the same camera)
  showed wrong colours on the Acer: coloured patches in the sky (cyan/magenta/yellow), too hard,
  too dark (user's phone photo next to a screenshot of the same clip on a PC). The O4 clip
  (3840x2880), where the shrink would have been > 10 % and so was NOT applied, was fine.
- A/B on the same clip with a script that toggled mpv's `video-zoom` at runtime (user watching):
  shrunk (plane 1360x1020+280+30) = wrong colours, `video-zoom 0` (plane 1440x1080+240+0) =
  right colours, shrunk again = wrong again. User's verdict: clearly the shrink.
- Not a colour-property problem: in all three phases mpv reports pixelformat drm_prime / rpi4_8,
  bt.709, limited, and the video plane in debugfs is NV12 with `color-encoding=BT.709` and
  `color-range=limited`, identical before and after. So the HVS's vertical line skipping
  (`vdownsample`, source/destination > 3x) damages the chroma plane of the 4:2:0 frame (the
  chroma plane has half the lines of luma). Kernel-side behaviour, nothing the app can set.
- Speed, same clip, 960x540 draw plane in all phases: shrunk 0.747x / 44.8 fps (flips 43.2/s),
  zoom off 0.647x / 38.8 fps (37.2/s), shrunk again 0.743x / 44.5 fps. The gain and the colour
  error come from the same mechanism and cannot be separated.
- **Consequence for the notes above:** the "ship the shrink" recommendation is withdrawn. What
  stays: the small mpv draw plane (`--drm-draw-surface-size=960x540`, 0.59x -> ~0.65x on the DJI
  clip, O4 45 -> 49-51 fps as read by the user), no colour side effect seen.
- **Open side effect for any setup where a clip is scaled down by more than 3x vertically**
  (e.g. a 720p or 640x480 output with a 4:3 4K clip): the HVS skips lines there on its own and
  the colours will probably be wrong. The earlier 640x480 "super smooth" observation was made
  without looking at the colours. Not checked, no such output mode is offered as a default.

Result with the line-skip zoom removed (0.3.46 test candidate, installed through the real
updater, user watching the colours, logger on the Pi, 2026-11-07 23:09-23:12; draw plane
960x540 only, 1080p59.94 output, camera on USB):

| Clip | Source | Speed (median) | On screen |
|---|---|---|---|
| `DJI_0023.MP4` | Action 2 4096x3072 @59.94 | 0.638x (0.575-0.655) | 38.2 fps |
| `DJI_0010.MP4` | Action 2 4096x3072 @29.97 | **1.000x** (0.986-1.023 after the first 3 s) | **30.0 fps** |
| `DJI_..._O4.MP4` | O4 3840x2880 @59.94 | 0.829x (0.700-0.876) | 49.7 fps |

Colours correct on all three. The 30 fps clip needs ~377 Mpix/s against ~667 available, as
estimated before measuring it. Throttled 0x0, 58 C, no kernel errors.


## Stretching a picture to the screen, 2026-11-07 (late)

Asked for as two options (live goggles picture, USB videos), off by default; implemented as
`stretch_live` / `stretch_usb` in the settings, two switches in Settings -> HDMI.

- **No speed effect** (A/B on `DJI_0023.MP4`, mpv `keepaspect` toggled at runtime, user watching):
  fit 1440x1080+240+0 = 0.632x / 37.9 fps, stretched 1920x1080+0+0 = 0.638x / 38.2 fps, colours
  correct. The decoder works on the full frame and the vertical scale (3072 -> 1080 lines) is the
  same, so nothing changes for the display's line reads.
- **USB (mpv):** `--keepaspect=no` at start and `set_property keepaspect` over IPC while playing
  (the setting is applied within a second; it does not rebuild the output). Tested by hand with
  the app's flags on H.264 clips of 1440x1080, 1280x960 and 960x720: all fill 1920x1080.
- **Live (GStreamer): kmssink has no stretch switch** (1.26.2: no `force-aspect-ratio`; it always
  fits the picture into the render rectangle). It does take the picture's pixel aspect ratio into
  account, so the stretch is a `capssetter caps=video/x-raw,pixel-aspect-ratio=N/D` in front of the
  sink, with N/D = (screen w x picture h) / (screen h x picture w) (4/3 for 4:3 on 16:9). The
  picture size comes from the SPS that starts the decoder (`h264::sps_size`, new; tested against an
  x264 stream, the built-in loop and synthetic SPSs); the screen's from the mode. Checked first
  with a test pattern (640x480 plus PAR 4/3 -> plane 1920x1080 instead of 1440x1080).
- **The display chip refuses some stretches.** On the live decoder's overlay plane (I420/YU12),
  `drmModeSetPlane` fails with ENOSPC ("No space left on device") for 1440x1080 (horizontal-only
  1.33x), 960x720 (x2, y1.5) and every PAR tried on 1440x1080 (also via videoconvert to NV12). It
  works for 1280x960 (x1.5, y1.125), 1280x720 -> 1080p without PAR, and 640x480 NV12 -> 1080p.
  Cause not pinned down (HVS scaling memory, per plane); the same stretch on mpv's primary plane
  works for all three sizes. The failing pipeline never prerolls, the decoder restarts for every
  keyframe and there is no picture at all, so the worker remembers: a stretched decoder that never
  showed a frame marks that picture size as refused, and the next start is unstretched (one lost
  keyframe interval; cleared on settings change and when the input goes away).
- **Tested end to end** through the installed 0.3.48 with a stand-in for the goggles (an H.264 file
  sent to the app's video socket, 8 byte sequence number + payload per datagram, paced): with
  `stretch_live` on, 1280x960 -> plane 1920x1080+0+0 at the first start; 1440x1080 and 960x720 ->
  one failed start, then 1440x1080+240+0 unstretched and playing; 1280x720 unchanged. With the
  setting off 1440x1080 stays 1440x1080+240+0. Not tested: real goggles, which sizes a 4:3 goggles
  mode really sends, USB stretch through the web UI.

**Display hand-over, follow-up 2026-11-08: a quick hotplug is not seen by the app.** An emulated
monitor power cycle (`echo off`, 0.2 s, `echo detect` on the connector's `status`) puts fbcon
(RG16) on the output within 0.2 s and, on 0.3.48, nothing took it back: the probe thread polls the
connector every 3 s, so an `off` shorter than that is invisible (with `off` for 2 s it caught it
about every other time and re-prepared the display). 0.3.49's probe also reads the CRTC's
framebuffer (`get_crtc`) and rebuilds the display when it is not the app's own: console visible for
2-4 s, then the app's picture, 3 of 3 runs. While a live stream plays, the same cycle left the
primary plane on the app's framebuffer (the overlay plane is in use) and caused no decoder restart.
