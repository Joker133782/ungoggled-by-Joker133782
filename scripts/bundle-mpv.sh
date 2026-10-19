#!/bin/bash
# Download mpv and its new dependencies as ARM64 .deb files, for an update to
# offline-install on a Pi that predates mpv (added for USB-drive video
# playback). Requires build/image/rootfs from a prior build-image.sh run, and
# kernel.apparmor_restrict_unprivileged_userns=0 for the rootless chroot
# (restore it to 1 afterwards; see HANDOFF notes on build-image.sh).
#
# One-time: once most installations have picked up an update built with this
# bundle, stop running this script and remove build/mpv-debs so future
# updates stop carrying it.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f build/image/rootfs/etc/os-release ]] || {
    echo 'Run scripts/build-image.sh at least once first, for build/image/rootfs.' >&2
    exit 1
}
rm -rf build/mpv-debs
mkdir -p build/mpv-debs
unshare --user --map-root-user --map-auto --mount --pid --fork --kill-child \
    scripts/image-chroot.sh build/image/rootfs /bin/bash -c '
set -euo pipefail
mkdir -p /run/NetworkManager
echo "nameserver 1.1.1.1" > /run/NetworkManager/resolv.conf
apt-get update
apt-get install --no-install-recommends --download-only -y mpv
'
cp build/image/rootfs/var/cache/apt/archives/*.deb build/mpv-debs/
du -sh build/mpv-debs
