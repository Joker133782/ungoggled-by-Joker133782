# Development

[← Back to the README](../README.md)

- [Build from source](#build-from-source)
- [Versions and releases](#versions-and-releases)
- [Service and files](#service-and-files)
- [README screenshots](#readme-screenshots)

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
sudo apt-get install -y gcc-aarch64-linux-gnu
scripts/build-update.sh
```

The cross C compiler is only there so `ring` (the self-signed TLS
certificate's crypto backend) can compile its per-target assembly; the
final binary still links against musl exclusively, via `rust-lld`
(`.cargo/config.toml` points `ring`'s build at it already).

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
and `scripts/build-image.sh`. See [CHANGELOG.md](../CHANGELOG.md) for release notes
and [the updater contract](UPDATES.md) for maintenance details.

When publishing a new release, also update the version, file names, sizes and
SHA-256 checksums in [DOWNLOAD.md](../DOWNLOAD.md) and the download links,
badge and button images (`docs/images/download-*.svg`) in the
[README](../README.md).

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
| `/var/lib/dji-hdmi/` | Settings and idle-screen images |
| `/run/dji-hdmi/` | USB/video socket and GStreamer cache |
| `/etc/default/dji-hdmi` | Decoder/runtime configuration |
| `/etc/NetworkManager/system-connections/dji-hdmi.nmconnection` | Image's Wi-Fi AP profile |

```sh
sudo systemctl restart dji-hdmi
journalctl -u dji-hdmi -f
```

Rust handles USB, DJI framing, display management, settings and the Axum HTTP API.
GStreamer handles decoding and the small JPEG preview; mpv plays USB videos and
recordings on the HDMI output. The web application uses
React, TypeScript, Rsbuild and Tailwind. USB workers are supervised independently
of the persistent renderer.

Historical protocol research and
legacy setup details are retained in
[FINDINGS.MD](../FINDINGS.MD). Dependency attribution is in
[THIRD_PARTY.md](../THIRD_PARTY.md).

## README screenshots

The console screenshots in `docs/images/` are taken against a mock of the Pi's
HTTP API with made-up data, so no real device is involved. The tooling is in
`scripts/readme-screenshots/` (Python 3 with Pillow, websocket-client and PyAV,
plus Microsoft Edge or Chrome); the usage is at the top of `shoot.py`.
