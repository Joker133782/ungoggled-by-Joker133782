#!/bin/bash
# Rootless Linux image build. Requires subordinate UID/GID mappings, binfmt
# qemu-aarch64, e2fsprogs, util-linux, curl, xz, Rust and Node/npm.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(python3 scripts/version.py --check)
source_url=https://downloads.raspberrypi.com/raspios_lite_arm64/images/raspios_lite_arm64-2026-09-15/2026-09-15-raspios-trixie-arm64-lite.img.xz
source_sha=cdf4f3bfac35ae947b46e4e767f935453810549779ac3290e05a6754aee627e5
mkdir -p build/cache build/image dist
base=build/cache/raspios-lite-arm64.img
# Every cached artifact below is written under a temporary name and renamed
# only once complete, and is checked against its expected size before reuse:
# an interrupted build (or one that ran out of disk) must not leave a partial
# file that the next build then trusts as its base.
if [[ ! -f "$base.xz" ]]; then
    curl -fL --retry 2 "$source_url" -o "$base.xz.part"
    mv "$base.xz.part" "$base.xz"
fi
printf '%s  %s\n' "$source_sha" "$base.xz" | sha256sum -c -
# Uncompressed size as recorded in the verified .xz index.
base_size=$(xz --robot --list "$base.xz" | awk -F'\t' '$1 == "totals" {print $5}')
[[ $base_size =~ ^[0-9]+$ ]] || { echo "Cannot read the uncompressed size of $base.xz" >&2; exit 1; }
if [[ ! -f "$base" || $(stat -c %s "$base") != "$base_size" ]]; then
    rm -f "$base"
    xz -dc "$base.xz" > "$base.part"
    mv "$base.part" "$base"
fi
bash scripts/prepare-image-release.sh
# Pinned image layout, verified against its MBR before reading partitions.
python3 - <<'PY'
import pathlib,struct
b=pathlib.Path('build/cache/raspios-lite-arm64.img').open('rb').read(512)
assert b[510:]==b'\x55\xaa'
assert struct.unpack_from('<II',b,446+8)==(16384,1048576)
assert struct.unpack_from('<II',b,462+8)==(1064960,4915200)
PY
root_ext4=build/image/root.ext4
# Outside the tree on purpose: everything inside rootfs ends up in the image.
extracted=build/image/rootfs.extracted
if [[ ! -f "$root_ext4" || $(stat -c %s "$root_ext4") != $((4915200*512)) ]]; then
    rm -f "$root_ext4" "$root_ext4.part" "$extracted" build/image/root.special-modes.json
    dd if="$base" of="$root_ext4.part" bs=512 skip=1064960 count=4915200 status=none
    [[ $(stat -c %s "$root_ext4.part") == $((4915200*512)) ]]
    mv "$root_ext4.part" "$root_ext4"
fi
# The extracted tree is reused (and provisioned on top of) by later builds;
# only one that was extracted completely counts. A tree from before this
# marker existed, or from an interrupted extraction, is extracted again.
if [[ ! -f "$extracted" ]]; then
    # debugfs reports read errors but still exits 0, so check the source first.
    e2fsck -fn "$root_ext4"
    if [[ -e build/image/rootfs ]]; then
        unshare --user --map-root-user --map-auto --mount rm -rf build/image/rootfs
    fi
    mkdir -p build/image/rootfs
    unshare --user --map-root-user --map-auto --mount debugfs -R 'rdump / build/image/rootfs' "$root_ext4"
    # Write errors (e.g. a full disk) do not fail debugfs either: every inode
    # in use must have arrived as at least one path (hard links give more).
    # Allowance: the root directory, reserved inodes 1-10, the orphan file,
    # and a few special files rdump does not recreate.
    inodes_used=$(dumpe2fs -h "$root_ext4" 2>/dev/null | awk -F': *' '
        $1 == "Inode count" {n = $2} $1 == "Free inodes" {f = $2} END {print n - f}')
    paths=$(unshare --user --map-root-user --map-auto --mount find build/image/rootfs -mindepth 1 | wc -l)
    if ((paths + 32 < inodes_used)); then
        echo "Incomplete extraction: $paths paths for $inodes_used inodes in use; check free disk space." >&2
        exit 1
    fi
    [[ -f build/image/rootfs/etc/os-release ]]
    touch "$extracted"
fi
unshare --user --map-root-user --map-auto --mount python3 scripts/restore-image-modes.py build/image/root.ext4 build/image/rootfs
bundle="build/releases/ungoggled-$version-aarch64"
rm -rf build/image/rootfs/tmp/dji-release
mkdir -p build/image/rootfs/tmp
cp -a "$bundle" build/image/rootfs/tmp/dji-release
unshare --user --map-root-user --map-auto --mount python3 scripts/configure-image.py build/image/rootfs
unshare --user --map-root-user --map-auto --mount --pid --fork --kill-child scripts/image-chroot.sh build/image/rootfs /bin/bash /tmp/dji-provision.sh
unshare --user --map-root-user --map-auto --mount --pid --fork --kill-child scripts/image-chroot.sh build/image/rootfs /bin/bash /tmp/dji-check-image.sh
unshare --user --map-root-user --map-auto --mount chroot build/image/rootfs dpkg-query -W > build/image/packages.txt
# Extract the original FAT boot partition using mtools from the ARM image.
dd if="$base" of=build/image/rootfs/tmp/dji-boot.fat bs=512 skip=16384 count=1048576 status=none
unshare --user --map-root-user --map-auto --mount chroot build/image/rootfs /bin/bash -c 'mkdir -p /boot/firmware; mcopy -s -i /tmp/dji-boot.fat "::*" /boot/firmware/'
unshare --user --map-root-user --map-auto --mount python3 - <<'PY'
from pathlib import Path
boot=Path('build/image/rootfs/boot/firmware')
p=boot/'config.txt';s=p.read_text()
# The CMA pool size, core_freq_min and the HEVC decoder module + overlay are
# not written here: `ungoggled system-fixes` below does that, the same code an
# update runs on an existing Pi (src/system_fixes.rs, where the reasons for
# each value are documented), so a fresh image and an updated Pi match.
s+='\n[all]\n# ungoggled USB accessory controller\ndtoverlay=dwc2,dr_mode=peripheral\n'
p.write_text(s)
p=boot/'cmdline.txt';parts=p.read_text().split();parts=[p for p in parts if not p.startswith(('init=','systemd.run=','systemd.run_success_action=','systemd.unit=','video=HDMI-A-1:', 'cfg80211.ieee80211_regdom='))];country=Path('build/image/rootfs/etc/dji-hdmi/radio-country').read_text().strip();parts+=['video=HDMI-A-1:1920x1080@60D', f'cfg80211.ieee80211_regdom={country}'];p.write_text(' '.join(parts)+'\n')
PY
# Kernel release of the image itself (uname -r inside the chroot is the host's).
kernel_release=$(cd build/image/rootfs/lib/modules && ls -d -- *-rpi-v8)
[[ $(wc -w <<< "$kernel_release") == 1 ]] || { echo "expected one rpi-v8 kernel, found: $kernel_release" >&2; exit 1; }
# --strict: the image build fails rather than ship without a fix (e.g. if the
# base image's kernel no longer matches the embedded module).
unshare --user --map-root-user --map-auto --mount --pid --fork --kill-child scripts/image-chroot.sh build/image/rootfs /opt/dji-hdmi/current/bin/ungoggled system-fixes apply --offline --strict --kernel-release "$kernel_release"
unshare --user --map-root-user --map-auto --mount chroot build/image/rootfs /bin/bash -c 'mcopy -o -i /tmp/dji-boot.fat /boot/firmware/config.txt /boot/firmware/cmdline.txt ::/ && mcopy -o -i /tmp/dji-boot.fat /boot/firmware/overlays/ungoggled-hevc-cma.dtbo ::/overlays/'

mv build/image/rootfs/tmp/dji-boot.fat build/image/boot.fat
# Remove build-only content and reset machine-specific state.
unshare --user --map-root-user --map-auto --mount python3 - <<'PY'
from pathlib import Path
import shutil
r=Path('build/image/rootfs')
for p in r.glob('qemu_*.core'):p.unlink()
for p in (r/'tmp').iterdir():
    if p.is_dir() and not p.is_symlink():shutil.rmtree(p)
    else:p.unlink()
# Boot files live on their FAT partition, not twice on the root filesystem.
for p in (r/'boot/firmware').iterdir():
    if p.is_dir():shutil.rmtree(p)
    else:p.unlink()
p=r/'etc/resolv.conf';p.unlink(missing_ok=True);p.symlink_to('/run/NetworkManager/resolv.conf')
PY
# Size the root filesystem around its contents rather than a fixed 6 GiB, so a
# card flashes and verifies in half the time. firstboot.sh still grows the
# partition and filesystem to the whole card on first boot.
headroom_mib=512
ceiling_mib=3584
root_image=build/image/final-root.ext4
fs_field() { dumpe2fs -h "$root_image" 2>/dev/null | sed -n "s/^$1: *//p"; }
# Reserve 1% rather than ext4's default 5%: root is the only writer, and the
# reservation grows with the card when firstboot.sh expands the filesystem.
mkfs_root() {
    rm -f "$root_image"
    truncate -s "$1" "$root_image"
    unshare --user --map-root-user --map-auto --mount mke2fs -q -t ext4 -F -m 1 -L rootfs -d build/image/rootfs "$root_image"
}
# resize2fs cannot estimate a minimum for this filesystem, so measure instead:
# format once with room to spare to learn what ext4 really needs for the tree,
# then format again at that size plus the headroom.
content_kib=$(unshare --user --map-root-user --map-auto --mount du -s --block-size=1024 build/image/rootfs | cut -f1)
mkfs_root $(((content_kib+content_kib/4+262144)*1024))
used_mib=$((($(fs_field 'Block count')-$(fs_field 'Free blocks'))*$(fs_field 'Block size')/1048576))
root_sectors=$(((used_mib+headroom_mib)*2048))
mkfs_root $((root_sectors*512))
e2fsck -fn "$root_image"
free_mib=$(($(fs_field 'Free blocks')*$(fs_field 'Block size')/1048576))
image_mib=$(((1064960+root_sectors)/2048))
printf 'Root filesystem: %s MiB, %s MiB free; expanded image %s MiB\n' \
    "$((root_sectors/2048))" "$free_mib" "$image_mib"
if ((free_mib < headroom_mib || image_mib > ceiling_mib)); then
    printf 'Wanted at least %s MiB free and at most %s MiB expanded\n' "$headroom_mib" "$ceiling_mib" >&2
    exit 1
fi
out=dist/ungoggled-$version-pi4-arm64.img
truncate -s $(((1064960+root_sectors)*512)) "$out"
dd if="$base" of="$out" bs=512 count=16384 conv=notrunc status=none
dd if=build/image/boot.fat of="$out" bs=512 seek=16384 conv=notrunc status=none
dd if="$root_image" of="$out" bs=512 seek=1064960 conv=notrunc status=none
# Record the shortened root partition, then read the image back: a card is
# flashed from these bytes, so the table must describe what they contain.
python3 - "$out" "$root_sectors" "$headroom_mib" <<'PY'
import struct,sys
path,root_sectors,headroom_mib=sys.argv[1],int(sys.argv[2]),int(sys.argv[3])
boot_start,boot_sectors,root_start=16384,1048576,1064960
with open(path,'r+b') as f:
    f.seek(462+12);f.write(struct.pack('<I',root_sectors))
    f.seek(0);mbr=f.read(512)
    assert mbr[510:]==b'\x55\xaa','missing MBR signature'
    assert struct.unpack_from('<II',mbr,446+8)==(boot_start,boot_sectors),'boot partition moved'
    assert struct.unpack_from('<II',mbr,462+8)==(root_start,root_sectors),'root partition mismatch'
    assert f.seek(0,2)==(root_start+root_sectors)*512,'image length does not match the table'
    f.seek(boot_start*512+510);assert f.read(2)==b'\x55\xaa','boot partition is not bootable'
    f.seek(root_start*512+1024);sb=f.read(1024)
    assert struct.unpack_from('<H',sb,56)==(0xEF53,),'root partition is not ext4'
    block_size=1024<<struct.unpack_from('<I',sb,24)[0]
    blocks,free=struct.unpack_from('<I',sb,4)[0],struct.unpack_from('<I',sb,12)[0]
    assert blocks*block_size<=root_sectors*512,'filesystem larger than its partition'
    assert free*block_size>=headroom_mib*1048576,'less free space than the intended headroom'
PY
xz -T2 -3 -f "$out"
(cd dist && sha256sum "$(basename "$out.xz")" > "$(basename "$out.xz.sha256")")
printf 'Image: %s.xz\nDefault Wi-Fi and root login: see README.md\n' "$out"
