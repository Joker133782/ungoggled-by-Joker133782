#!/bin/bash
# Install/update a release bundle. Keeps settings, uploaded images and Wi-Fi.
set -euo pipefail
[[ $(id -u) == 0 ]] || { echo 'Run with sudo.' >&2; exit 1; }
cd "$(dirname "$(readlink -f "$0")")"
sha256sum --check --quiet SHA256SUMS
./bin/ungoggled --version
arch=$(uname -m)
expected=$(cat ARCH)
[[ "$arch" == "$expected" ]] || { echo "Bundle is for $expected; this Pi is $arch." >&2; exit 1; }
version=$(cat VERSION)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[a-f0-9]+$ ]] || exit 1
release=/opt/dji-hdmi/releases/$version
mkdir -p /opt/dji-hdmi/releases /var/lib/dji-hdmi /etc/default /usr/local/lib/dji-hdmi
stage=
previous_unit=$(mktemp)
trap 'test -z "${stage:-}" || rm -rf -- "$stage"; rm -f -- "$previous_unit"' EXIT
if [[ -f /etc/systemd/system/dji-hdmi.service ]]; then
    cp /etc/systemd/system/dji-hdmi.service "$previous_unit"
fi
if [[ ! -d "$release" ]]; then
    stage=$(mktemp -d /opt/dji-hdmi/releases/.install-XXXXXX)
    cp -a bin web VERSION ARCH SHA256SUMS "$stage/"
    chmod -R a+rX "$stage"
    mv "$stage" "$release"
    stage=
fi
previous=$(readlink -f /opt/dji-hdmi/current || true)
install -m755 prepare-pi.sh /usr/local/lib/dji-hdmi/prepare-pi.sh
install -m644 dji-hdmi.service /etc/systemd/system/dji-hdmi.service
if [[ ! -e /etc/default/dji-hdmi ]]; then
    printf 'DJI_HDMI_DECODER=v4l2h264dec\n' > /etc/default/dji-hdmi
fi
ln -sfn "$release" /opt/dji-hdmi/current.next
mv -Tf /opt/dji-hdmi/current.next /opt/dji-hdmi/current
install -m755 bin/ungoggled /usr/local/lib/dji-hdmi/update-helper
install -m644 ungoggled-update-recovery.service /etc/systemd/system/ungoggled-update-recovery.service
systemctl enable ungoggled-update-recovery.service
systemctl daemon-reload
systemctl enable dji-hdmi.service
healthy=0
if systemctl restart dji-hdmi.service; then
for _ in {1..30}; do
    # /api/status is the one route open to a request from this machine (the rest
    # needs the control password). -L -k: with TLS on, plain HTTP only redirects
    # to the self-signed HTTPS port, and curl does not fail on a 3xx by itself.
    if curl -fsSLk --max-time 2 http://127.0.0.1/api/status 2>/dev/null | grep -q '"uptime_seconds"'; then healthy=1;break;fi
    sleep 1
done
fi
if [[ $healthy == 0 ]]; then
    if [[ -n "$previous" && -d "$previous" ]]; then
        ln -sfn "$previous" /opt/dji-hdmi/current.next
        mv -Tf /opt/dji-hdmi/current.next /opt/dji-hdmi/current
        # The previous release may still call its executable dji-hdmi.
        if [[ -s "$previous_unit" ]]; then
            cp "$previous_unit" /etc/systemd/system/dji-hdmi.service
            systemctl daemon-reload
        fi
        systemctl restart dji-hdmi.service
        echo 'Update failed its HTTP health check; previous application restored.' >&2
    else
        echo 'Service did not become healthy; inspect journalctl -u dji-hdmi.' >&2
    fi
    exit 1
fi
printf 'Installed %s. Open http://192.168.50.1\n' "$version"
