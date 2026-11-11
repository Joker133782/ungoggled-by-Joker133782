# Protocol research and attribution

This implementation was written in Rust from the public AOA/Linux specifications,
public protocol facts and local hardware observations. Research downloads under
`research/upstream/` are ignored by version control and are not shipped with the
application.

- DJI mobile framing and N3 registration observations: Samuel Sadok,
  https://github.com/samuelsadok/dji_protocol . Exact revision and corrections are
  in FINDINGS.MD. No Python implementation is included in the application.
- AOA control-channel and app-identity response observations: the
  dji-o3-usb-decoder contributors, MIT licensed,
  https://github.com/dr00min/o3-usb-decoder .

MIT notice for the latter reference:

Copyright (c) 2026 the dji-o3-usb-decoder contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

Cargo and npm dependency licenses remain with their respective authors.

## Bundled mpv packages

An update may carry Debian's unmodified `mpv` `.deb` package and its new
dependencies (also unmodified Debian builds), for offline-installing mpv on
a Pi that predates it (used for USB-drive video playback). No mpv or ffmpeg
source or binary is embedded in the Rust executable; these are the exact
packages Debian trixie publishes, under their own licenses (mpv is GPL-2.0-
or-later). See `scripts/bundle-mpv.sh` for how they are obtained.

## Spectator-page player (mpegts.js, Apache-2.0)

`web/public/mpegts.js` is the unmodified `dist/mpegts.js` of the npm package
`mpegts.js` 1.8.2 (https://github.com/xqq/mpegts.js), copyright its authors,
Apache License 2.0. It is served next to `watch.html` so the spectator page
needs no internet. Package integrity (npm `dist.integrity`):
`sha512-cZYMa5muASH55wrS6JK5IGkUqnCA96c0f+WdnW91jIK9s/N6NzjmTQDJNYg9A5u/G2Q5nj2b3w1Zwzze/9guxA==`;
SHA-256 of the file: `bda31748736a69cb610c2edf4623e633f1f4f47b5bda83668c8d287e51b0c3a8`.
The upstream license is shipped beside it as `mpegts.js.Apache-2.0.txt`, and
the bundled `es6-promise` notice (MIT) as `mpegts.js.LICENSE.txt`, which the
file's header refers to.

## Optional legacy decoder plugin

`scripts/build-legacy-omx.sh` downloads and builds unmodified gst-omx 1.14.4,
under its GNU LGPL 2.1 license. It is loaded into the external GStreamer process.
No GStreamer source or binary is embedded in the Rust executable. The archive,
checksum and build configuration are documented in FINDINGS.MD and the script.
Retain the upstream license and source when redistributing a built plugin.

Source: https://gstreamer.freedesktop.org/src/gst-omx/gst-omx-1.14.4.tar.xz

## Patched Raspberry Pi HEVC decoder driver (GPL-2.0)

The executable embeds a kernel module, `assets/kernel/6.18.50+rpt-rpi-v8/
rpi-hevc-dec.ko`, and a device-tree overlay, `assets/kernel/
ungoggled-hevc-cma.dtbo`, which it installs on the Pi. The module is the
Raspberry Pi kernel's `rpi-hevc-dec` driver, GNU GPL version 2, built from
`drivers/media/platform/raspberrypi/hevc_dec/` of
https://github.com/raspberrypi/linux at commit
`f2bc178ed43d8f737c10e2ca80598e2a812eab7b` (branch `rpi-6.18.y`), with our
change `docs/upstream-patches/rpi-hevc-memory-region/0001-rpi-hevc-dec-memory-region.patch`
applied, against the headers of the Raspberry Pi OS kernel package
`linux-headers-6.18.50+rpt-rpi-v8` 1:6.18.50-1+rpt1. The complete
corresponding source is that upstream directory plus the patch;
`scripts/build-hevc-module.sh` fetches and verifies it and rebuilds the module
bit for bit (hashes in `assets/kernel/6.18.50+rpt-rpi-v8/BUILD-INFO`). The
overlay source is `assets/kernel/ungoggled-hevc-cma.dts` (GPL-2.0). Whoever
redistributes the executable or an image must keep this source available
with it.

## Built-in waiting-screen videos

`assets/default-loop.h264`, `assets/default-loop-3d.h264` and
`assets/default-comic.h264`, with their poster images (`*.jpg`), were created
by the project owner together with an AI assistant (Claude Opus). They are
original to this project, contain no third-party footage, and are released
under this project's MIT license.
