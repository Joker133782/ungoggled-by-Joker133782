#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Internal image staging; only the image and update belong in dist/.
target=aarch64-unknown-linux-musl
arch=aarch64
cargo build --release --target "$target"
(cd web && npm ci --cache /tmp/dji-npm-cache --fetch-retries=0 && npm run build)
binary=target/$target/release/ungoggled
version=$(python3 scripts/version.py --check)
# Include UI and installer changes in the release identity, not only the binary.
digest=$(tar -cf - "$binary" web/dist scripts/install-release.sh scripts/prepare-pi.sh deploy/dji-hdmi.service | sha256sum | cut -c1-12)
name=ungoggled-$version-$arch
bundle=build/releases/$name
mkdir -p build/releases
# Start fresh so obsolete hashed UI assets cannot enter the new release.
if [[ -d "$bundle" ]]; then rm -rf -- "$bundle"; fi
mkdir -p "$bundle/bin" "$bundle/web"
install -m755 "$binary" "$bundle/bin/ungoggled"
cp -a web/dist/. "$bundle/web/"
install -m755 scripts/install-release.sh "$bundle/install.sh"
install -m755 scripts/prepare-pi.sh "$bundle/prepare-pi.sh"
cp deploy/dji-hdmi.service deploy/ungoggled-update-recovery.service "$bundle/"
printf '%s-%s\n' "$version" "$digest" > "$bundle/VERSION"
printf '%s\n' "$arch" > "$bundle/ARCH"
cp README.md FINDINGS.MD THIRD_PARTY.md "$bundle/"
(cd "$bundle" && find bin web -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS && sha256sum install.sh prepare-pi.sh dji-hdmi.service ungoggled-update-recovery.service VERSION ARCH README.md FINDINGS.MD THIRD_PARTY.md >> SHA256SUMS)
printf 'Image application staged: %s\n' "$bundle"
