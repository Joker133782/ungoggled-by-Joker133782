#!/bin/bash
# Download mpv and its new dependencies as ARM64 .deb files, for an update to
# offline-install on a Pi that predates mpv (added for USB-drive video
# playback). Requires build/image/rootfs from a prior build-image.sh run (run
# it right after one, so the image's package versions are still in the
# archive), and kernel.apparmor_restrict_unprivileged_userns=0 for the
# rootless chroot (restore it to 1 afterwards; see HANDOFF notes on
# build-image.sh).
#
# "New dependencies" means: what removing mpv from the provisioned image would
# also remove, i.e. what an image provisioned with the same packages minus mpv
# lacks. Asking apt to download mpv instead would fetch nothing (it is already
# installed there) or only upgrades. The exact versions installed in the image
# are bundled, because those are known to work together. Pis built from an
# older base image may still run older library versions; this bundle is
# resolved against the current base image only.
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
[[ $(dpkg-query -W -f "\${Status}" mpv 2>/dev/null) == "install ok installed" ]] || {
    echo "mpv is not installed in build/image/rootfs; run scripts/build-image.sh first." >&2
    exit 1
}
apt-get update
# Recommends/Suggests do not keep a package: the image is provisioned with
# --no-install-recommends, so they are no reason to expect it on an old Pi.
apt-get -s -o APT::AutoRemove::RecommendsImportant=false \
    -o APT::AutoRemove::SuggestsImportant=false purge --auto-remove mpv \
    | awk "\$1 == \"Purg\" {print \$2}" > /tmp/mpv-packages
grep -qx "mpv\(:arm64\)\?" /tmp/mpv-packages
rm -rf /tmp/mpv-debs
mkdir -p /tmp/mpv-debs
cd /tmp/mpv-debs
while read -r package; do
    version=$(dpkg-query -W -f "\${Version}" "$package")
    apt-get download "$package=$version"
done < /tmp/mpv-packages
# One .deb per package, or a download went missing.
[[ $(ls -- *.deb | wc -l) == $(wc -l < /tmp/mpv-packages) ]]
'
cp build/image/rootfs/tmp/mpv-debs/*.deb build/mpv-debs/
rm -rf build/image/rootfs/tmp/mpv-debs build/image/rootfs/tmp/mpv-packages
ls build/mpv-debs
du -sh build/mpv-debs
