# Update implementation

`Cargo.toml` defines the release version. `scripts/version.py` synchronizes npm
metadata and checks the Cargo lock entry and changelog. Stable `vX.Y.Z` tags
trigger `.github/workflows/release.yml`.

## Package contract (format 1)

`ungoggled-VERSION.update.tar.gz` contains regular files at the archive root:

- `manifest.json`: product `ungoggled`, format `1`, target version, minimum
  updater version, target settings schema and every payload file's size/SHA-256.
- `bin/aarch64/ungoggled` (Pi 4 with 64-bit Raspberry Pi OS only).
- `web/`, service definitions, `prepare-pi.sh`, release notes and attribution.
- `install.sh`: manual bootstrap, checking that the Pi runs a 64-bit OS.

No previous application files are required. Keep format 1 and the migration
command compatible with updater 0.3.0 so users can skip releases. A target
release's `migrate-settings --data-dir DIR --output FILE` command writes the
compatible settings and prints a JSON array of warnings. It must not replace the
source settings. A newer settings schema is handled by the target executable,
not the installed updater. Breaking this contract requires an explicitly
communicated migration path; do not silently increase `minimum_updater`.

Archive extraction rejects links, special files, path traversal, duplicates,
unlisted files and mismatched checksums. Uploads are limited to 64 MiB compressed
and 256 MiB expanded. Checksums provide integrity, not signing/authentication.
Only upload trusted release packages.

## Transaction

1. Stream the upload to `/var/lib/ungoggled-update/ID/`, with a process-wide file
   lock. Validate and persist a `ready` state before presenting Install.
2. Launch `update-worker` as a separate transient systemd unit so stopping the
   receiver does not stop the installer.
3. Validate again, install to `/opt/dji-hdmi/releases/VERSION-HASH`, and ensure the
   boot recovery helper is available outside the application releases.
4. Stop the receiver, snapshot its settings and service/helper files, and fsync
   a recovery journal before changing them.
5. Run the candidate's settings migration, atomically switch `current`, then
   reload/start the service. Check HTTP on port 80 for the exact target version
   and build ID, with multiple successful observations after startup.
6. Persist `succeeded` as the commit point. Cleanup is best-effort afterward.
   On failure, restore the previous release, settings and service files.

`ungoggled-update-recovery.service` runs before the receiver at boot and restores
an uncommitted transaction. It never starts the receiver synchronously during
boot, avoiding an ordering deadlock. The image and installers enable both units;
the receiver must not acquire a new `Wants=` dependency on recovery when restarted
mid-update. Retained releases: the active and preceding ones, the newest other
web-installed release and the factory release from the SD-card image (it has
`VERSION` but no `manifest.json`). Directories outside the `VERSION-HASH` scheme
are never touched. Web-installed releases keep copies of their
`dji-hdmi.service` and `prepare-pi.sh` for later switches.

## System fixes (boot configuration, HEVC decoder module)

Fixes below the application (`config.txt`, a kernel module) cannot travel as
new archive entries: the first hop is processed by the updater already on the
Pi, whose whitelist (`package.rs::allowed`) rejects unknown files. What every
updater does is install the package's `prepare-pi.sh`, switch `current` and
restart the service. So `prepare-pi.sh` runs `ungoggled system-fixes apply`
as root before every service start, and the data is embedded in the binary
(`src/system_fixes.rs`, `include_bytes!` from `assets/kernel/`). The image
build runs the same command inside the image (`--offline --strict
--kernel-release ...`), so a fresh image and an updated Pi end up identical.

- `config.txt` (`/boot/firmware/config.txt`): a marked block at the end
  (`[all]`, `core_freq_min=500`, `hdmi_enable_4kp60=1`,
  `dtoverlay=gpio-fan,gpiopin=14,temp=60000`, `dtoverlay=ungoggled-hevc-cma`) and
  `cma-size=0x28000000` on the single `vc4-kms-v3d` line if it is missing or
  smaller. Idempotent, written atomically (temp file, fsync, rename, directory
  fsync); `config.txt.ungoggled-orig` (before the first change) and
  `config.txt.ungoggled-prev` (before the latest one) are kept. A file with an
  unexpected shape, or that cannot be read or written, is left alone and
  logged. A value someone else set (`core_freq_min`, `hdmi_enable_4kp60`, a
  `gpio-fan` overlay of any pin or temperature, a bigger CMA pool) wins.
- Patched `rpi-hevc-dec` in `/lib/modules/RELEASE/updates/ungoggled/` plus
  `overlays/ungoggled-hevc-cma.dtbo`, only if the kernel release, the
  `linux-image-RELEASE` package version and the stock module's vermagic match
  what it was built for; otherwise it is removed (with its overlay line).
  Overlay file before the line; `depmod`, then `modinfo -n` must name our copy.
  The stock module is never touched. If after a boot no decoder module is
  loaded and `modprobe` fails, this module build is switched off for good and
  the stock one loaded. While installed, every installed `linux-image-*`,
  `linux-headers-*` and `linux-kbuild-*` package is on `apt-mark hold`
  (released again when the module goes).
- Nothing reboots. `/api/status` reports `reboot_required` (configured but not
  yet running: `vcgencmd get_config core_freq_min` and `hdmi_enable_4kp60`,
  the `gpio-fan` node in `/proc/device-tree`, the `linux,cma` size and
  the decoder's `memory-region` in `/proc/device-tree`, the loaded module's
  `srcversion`), and the web interface asks for a reboot.
- A failure here never stops the service: `prepare-pi.sh` only logs it. After
  a switch back to a release without the command (the factory release keeps
  the current `prepare-pi.sh`), the fixes stay as they are and the old binary
  just logs a usage error.
- Way back: `ungoggled system-fixes remove-module` (module, overlay, hold;
  stays off until `enable-module`). `ungoggled system-fixes status` prints the
  state as JSON. Module build and kernel bumps: `scripts/build-hevc-module.sh`.

## Switching back and history

A switch to an installed release is a job containing `rollback.json` instead of
a package. `update-worker` runs it through the same stop, snapshot, journal,
migration, health check and restore steps; the target's executable migrates the
settings. The factory release reports no build ID, so its health check expects
`build_id: null`. Releases without stored service files keep the current ones.
Each install, switch and boot recovery appends to `history.json` (last 50).

Wi-Fi profiles, login credentials, uploaded images, OS packages and
`/etc/default/dji-hdmi` are outside the application migration. Boot
configuration is handled separately, see "System fixes" below. Rootless local
tests use isolated directories and a fake service manager. `check-update --file
ARCHIVE --architecture aarch64` also verifies the actual built archive on a PC.

## HTTP and browser

- `GET /api/update`: current version/build, support status and durable progress.
- `POST /api/update/upload`: one multipart file; returns the validated candidate.
- `POST /api/update/install`: JSON `{ "id": "..." }`; queues that candidate.
- `GET /api/update/releases`: installed releases and the history, newest first.
- `POST /api/update/rollback`: JSON `{ "release": "VERSION-HASH" }`; queues a switch.

Mutating endpoints require the existing `X-DJI-Control: 1` header. Settings and
Wi-Fi changes are blocked during installation. Uploading alone does not interrupt
video. The browser tracks progress across receiver restarts and reloads its assets
after success.

## Release checking (Nextcloud, not GitHub)

`web/src/releases.ts`'s GitHub check is deliberately disabled (`endpoint = ""`)
and must stay that way until this fork's license status with the upstream
project is resolved — see the project memory on this. Checking or installing
from the upstream repository would silently overwrite every change made here.

The active mechanism is `web/src/nextcloud.ts`: the browser reads
`latest.json` and the update file directly from a public Nextcloud share via
WebDAV (cross-origin only because of a Cloudflare Worker added specifically
for that one path — see the project memory for the worker/zone details). The
Pi itself never needs internet access; the download and the version check
both happen in the browser (so, in practice, over whatever network the
viewing phone is on), and the downloaded file is fed into the existing
`/api/update/upload` endpoint exactly like a manual upload.

**Every time a new version is built**, update the Nextcloud share:
upload the new `ungoggled-VERSION.update.tar.gz`, and update `latest.json`
in the share root:

```json
{
  "version": "0.3.23",
  "file": "ungoggled-0.3.23.update.tar.gz",
  "notes": ["Short, non-technical bullet", "Another one"]
}
```

`notes` are shown verbatim in the update UI and should read like the wiki's
Updates page (`web/public/wiki.html#updates`) — plain, user-facing language,
no internals (e.g. "WebUI Rework", "Goggles 2 Support (experimental)"), not
a copy of `CHANGELOG.md`. Add the same bullets to that wiki page's history
list so it stays a complete, readable record for someone who wasn't there
for the development.
