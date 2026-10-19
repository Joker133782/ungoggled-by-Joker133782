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

## Optional legacy decoder plugin

`scripts/build-legacy-omx.sh` downloads and builds unmodified gst-omx 1.14.4,
under its GNU LGPL 2.1 license. It is loaded into the external GStreamer process.
No GStreamer source or binary is embedded in the Rust executable. The archive,
checksum and build configuration are documented in FINDINGS.MD and the script.
Retain the upstream license and source when redistributing a built plugin.

Source: https://gstreamer.freedesktop.org/src/gst-omx/gst-omx-1.14.4.tar.xz
