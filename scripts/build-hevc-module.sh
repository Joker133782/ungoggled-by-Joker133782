#!/bin/bash
# Reproducible build of the patched rpi-hevc-dec module and its overlay, which
# src/system_fixes.rs embeds (include_bytes!) and installs on the Pi.
#
#   scripts/build-hevc-module.sh fetch
#       Any machine with internet: download the driver source of the pinned
#       raspberrypi/linux commit into build/hevc-module/src and check every
#       file against its git blob hash below.
#   scripts/build-hevc-module.sh build [SRC_DIR]
#       On a Pi 4 (aarch64) running 64-bit Raspberry Pi OS with
#       linux-headers-$KERNEL_RELEASE at exactly $KERNEL_PACKAGE installed (the
#       headers' build tools are aarch64 binaries). Applies the patch, builds
#       against those headers, compiles the overlay and writes
#       assets/kernel/$KERNEL_RELEASE/{rpi-hevc-dec.ko,BUILD-INFO} and
#       assets/kernel/ungoggled-hevc-cma.dtbo. Copy those back into the repo.
#
# Kernel bump: change the four pins below (commit = the last raspberrypi/linux
# commit touching drivers/media/platform/raspberrypi/hevc_dec in the new
# kernel; blob hashes from `git ls-tree COMMIT drivers/media/.../hevc_dec/`),
# rebuild on a Pi running the new kernel, update KERNEL_RELEASE /
# KERNEL_PACKAGE and the include_bytes! path in src/system_fixes.rs. The unit
# tests there fail until the embedded module, BUILD-INFO and the constants
# agree. Stop shipping the module (delete the constants' module) once an
# official kernel contains the fix.
set -euo pipefail
cd "$(dirname "$0")/.."
KERNEL_RELEASE=6.18.50+rpt-rpi-v8
KERNEL_PACKAGE=1:6.18.50-1+rpt1
DRIVER_COMMIT=f2bc178ed43d8f737c10e2ca80598e2a812eab7b
DRIVER_PATH=drivers/media/platform/raspberrypi/hevc_dec
PATCH=docs/upstream-patches/rpi-hevc-memory-region/0001-rpi-hevc-dec-memory-region.patch
OVERLAY_SOURCE=assets/kernel/ungoggled-hevc-cma.dts
# git blob SHA-1 of every file in $DRIVER_PATH at $DRIVER_COMMIT.
BLOBS='Kconfig fc4d0baf5c0f83a0399cf157cfbef632b35d1ff7
Makefile b6506bf2e00d153b71ab982bae2948a771b00449
hevc_d.c 392fd40119fcfb99a0113d919f1443d90e38afa3
hevc_d.h 6d40d91229a6b5f50c3d113f5f7e8795b60522e1
hevc_d_h265.c 83ce62afd158b7a7f1b8a0a829b35b34231a75a8
hevc_d_h265.h 9961917bf204013fa5c36285d4d9e3699c67fc9d
hevc_d_hw.c 8564b332f8d2afa893e840ca2a4fabb065ef4c6b
hevc_d_hw.h 85fe96fc13d4c59c8315ddc1e0cadabab4e7dba2
hevc_d_video.c 27e92ccad24dfac084ff26ba0b523d9374239b68
hevc_d_video.h 5a1ca7e4f06d461c0b7d9b2a29b0a9d5bb4fa6c4'

blob_sha1() { python3 -c 'import hashlib,sys;d=open(sys.argv[1],"rb").read();print(hashlib.sha1(b"blob %d\0"%len(d)+d).hexdigest())' "$1"; }
verify_source() {
    local dir=$1 name sha
    while read -r name sha; do
        [[ -f "$dir/$name" ]] || { echo "missing $dir/$name" >&2; exit 1; }
        [[ $(blob_sha1 "$dir/$name") == "$sha" ]] || { echo "$name does not match $DRIVER_COMMIT" >&2; exit 1; }
    done <<< "$BLOBS"
    [[ $(find "$dir" -maxdepth 1 -type f | wc -l) == $(wc -l <<< "$BLOBS") ]] || { echo "unexpected files in $dir" >&2; exit 1; }
}

case "${1:-}" in
fetch)
    src=build/hevc-module/src
    rm -rf "$src"; mkdir -p "$src"
    while read -r name _; do
        curl -fsSL --retry 2 -o "$src/$name" \
            "https://raw.githubusercontent.com/raspberrypi/linux/$DRIVER_COMMIT/$DRIVER_PATH/$name"
    done <<< "$BLOBS"
    verify_source "$src"
    echo "Source of $DRIVER_COMMIT in $src"
    ;;
build)
    src=${2:-build/hevc-module/src}
    verify_source "$src"
    [[ $(uname -m) == aarch64 ]] || { echo 'Build on a 64-bit Pi (aarch64).' >&2; exit 1; }
    headers=/lib/modules/$KERNEL_RELEASE/build
    [[ -f "$headers/Makefile" ]] || { echo "Install linux-headers-$KERNEL_RELEASE first." >&2; exit 1; }
    installed=$(dpkg-query -W -f='${Version}' "linux-headers-$KERNEL_RELEASE")
    [[ "$installed" == "$KERNEL_PACKAGE" ]] || { echo "linux-headers-$KERNEL_RELEASE is $installed, need $KERNEL_PACKAGE" >&2; exit 1; }
    work=build/hevc-module/work
    rm -rf "$work"; mkdir -p "$work/$DRIVER_PATH"
    cp "$src"/* "$work/$DRIVER_PATH/"
    (cd "$work" && patch -p1 --no-backup-if-mismatch -s) < "$PATCH"
    # Fixed build directory and a clean environment: the module carries no
    # timestamps, so the same inputs give the same bytes (checked below).
    make -s -C "$headers" M="$PWD/$work/$DRIVER_PATH" modules KBUILD_BUILD_TIMESTAMP= > "$work/build.log" 2>&1 \
        || { cat "$work/build.log" >&2; exit 1; }
    if grep -q 'warning:' "$work/build.log"; then cat "$work/build.log" >&2; echo 'compiler warnings' >&2; exit 1; fi
    ko=$work/$DRIVER_PATH/rpi-hevc-dec.ko
    vermagic=$(modinfo -F vermagic "$ko")
    [[ ${vermagic%% *} == "$KERNEL_RELEASE" ]] || { echo "vermagic $vermagic" >&2; exit 1; }
    stock=$(modinfo -F vermagic -k "$KERNEL_RELEASE" "/lib/modules/$KERNEL_RELEASE/kernel/$DRIVER_PATH/rpi-hevc-dec.ko.xz")
    [[ "$vermagic" == "$stock" ]] || { echo "vermagic differs from the stock module: $stock" >&2; exit 1; }
    out=assets/kernel/$KERNEL_RELEASE
    mkdir -p "$out"
    install -m644 "$ko" "$out/rpi-hevc-dec.ko"
    dtc -q -@ -I dts -O dtb -o assets/kernel/ungoggled-hevc-cma.dtbo "$OVERLAY_SOURCE"
    {
        echo "kernel_release=$KERNEL_RELEASE"
        echo "kernel_package=$KERNEL_PACKAGE"
        echo "driver_commit=$DRIVER_COMMIT"
        echo "patch_sha256=$(sha256sum "$PATCH" | cut -d' ' -f1)"
        echo "vermagic=$vermagic"
        echo "srcversion=$(modinfo -F srcversion "$ko")"
        echo "module_sha256=$(sha256sum "$out/rpi-hevc-dec.ko" | cut -d' ' -f1)"
        echo "overlay_sha256=$(sha256sum assets/kernel/ungoggled-hevc-cma.dtbo | cut -d' ' -f1)"
    } > "$out/BUILD-INFO"
    cat "$out/BUILD-INFO"
    ;;
*)
    sed -n '2,24p' "$0" >&2
    exit 2
    ;;
esac
