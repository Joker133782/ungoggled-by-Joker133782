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

## Switching back and history

A switch to an installed release is a job containing `rollback.json` instead of
a package. `update-worker` runs it through the same stop, snapshot, journal,
migration, health check and restore steps; the target's executable migrates the
settings. The factory release reports no build ID, so its health check expects
`build_id: null`. Releases without stored service files keep the current ones.
Each install, switch and boot recovery appends to `history.json` (last 50).

Wi-Fi profiles, login credentials, uploaded images, OS packages, boot configuration
and `/etc/default/dji-hdmi` are outside the application migration. Rootless local
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
after success. Release notices query the public GitHub API in the browser, cache
for one hour and compare stable semantic versions. Release notes render as text.
