#!/bin/bash
# Executed inside the extracted ARM64 image, not on the build host.
set -euo pipefail
[[ $(uname -m) == aarch64 && -f /tmp/dji-image.env ]]
. /tmp/dji-image.env
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav v4l-utils openssh-server network-manager dnsmasq-base curl ca-certificates sudo iw rfkill cloud-guest-utils mtools mpv
# A reused build tree keeps the release directories of earlier builds, which
# would ship as dead weight; only this build's release belongs in the image.
rm -rf /opt/dji-hdmi/releases
mkdir -p /etc/dji-hdmi /etc/default /var/lib/dji-hdmi /usr/local/lib/dji-hdmi /opt/dji-hdmi/releases
bundle=/tmp/dji-release
version=$(cat "$bundle/VERSION")
release=/opt/dji-hdmi/releases/$version
mkdir -p "$release"
cp -a "$bundle/bin" "$bundle/web" "$bundle/VERSION" "$bundle/ARCH" "$release/"
ln -sfn "$release" /opt/dji-hdmi/current
install -m755 "$bundle/prepare-pi.sh" /usr/local/lib/dji-hdmi/prepare-pi.sh
install -m644 "$bundle/dji-hdmi.service" /etc/systemd/system/dji-hdmi.service
install -m755 "$bundle/bin/ungoggled" /usr/local/lib/dji-hdmi/update-helper
install -m644 "$bundle/ungoggled-update-recovery.service" /etc/systemd/system/ungoggled-update-recovery.service
install -m755 /tmp/dji-firstboot.sh /usr/local/lib/dji-hdmi/firstboot.sh
install -m644 /tmp/dji-firstboot.service /etc/systemd/system/dji-hdmi-firstboot.service
printf 'DJI_HDMI_DECODER=v4l2h264dec\n' > /etc/default/dji-hdmi
printf '%s\n' "$RADIO_COUNTRY" > /etc/dji-hdmi/radio-country
printf 'ungoggled\n' > /etc/hostname
printf '127.0.0.1 localhost\n127.0.1.1 ungoggled\n::1 localhost ip6-localhost ip6-loopback\n' > /etc/hosts
# Reused build trees may still contain the old generated-login account.
if id dji >/dev/null 2>&1; then userdel --remove dji; fi
usermod --shell /bin/bash root
printf 'root:%s\n' "$SSH_PASSWORD" | chpasswd
rm -f /etc/ssh/sshd_config.d/rename_user.conf /etc/ssh/sshd_config.d/90-dji-hdmi.conf
mkdir -p /etc/ssh/sshd_config.d
printf 'PasswordAuthentication yes\nPermitRootLogin yes\n' > /etc/ssh/sshd_config.d/00-dji-hdmi.conf
# Existing stock-image keys must never be distributed to multiple devices.
rm -f /etc/ssh/ssh_host_* /etc/machine-id /var/lib/dbus/machine-id
: > /etc/machine-id
ln -s /etc/machine-id /var/lib/dbus/machine-id
# Stock Lite disables Wi-Fi in NetworkManager as well as rfkill. Unblocking
# before NM starts is insufficient unless its saved radio state also changes.
mkdir -p /var/lib/NetworkManager /etc/NetworkManager/system-connections /etc/netplan
printf '[main]\nNetworkingEnabled=true\nWirelessEnabled=true\n' > /var/lib/NetworkManager/NetworkManager.state
rm -f /var/lib/systemd/rfkill/*:wlan
printf 'options rfkill default_state=1\n' > /etc/modprobe.d/rfkill_default.conf
cat > /etc/NetworkManager/system-connections/dji-hdmi.nmconnection <<PROFILE
[connection]
id=dji-hdmi
uuid=$AP_UUID
type=wifi
interface-name=wlan0
autoconnect=true
autoconnect-priority=100

[wifi]
mode=ap
ssid=ungoggled
band=bg
channel=6
powersave=2

[wifi-security]
key-mgmt=wpa-psk
psk=$WIFI_PASSWORD

[ipv4]
method=shared
address1=192.168.50.1/24
never-default=true

[ipv6]
method=disabled
PROFILE
chmod 600 /etc/NetworkManager/system-connections/dji-hdmi.nmconnection
cat > /etc/netplan/90-dji-hdmi.yaml <<'NETPLAN'
network:
  version: 2
  renderer: NetworkManager
NETPLAN
chmod 600 /etc/netplan/90-dji-hdmi.yaml
# The image is already provisioned; avoid a second interactive/cloud setup.
mkdir -p /etc/cloud /etc/systemd/system.conf.d
: > /etc/cloud/cloud-init.disabled
systemctl mask userconfig.service
systemctl enable NetworkManager.service ssh.service ungoggled-update-recovery.service dji-hdmi-firstboot.service dji-hdmi.service
printf '[Manager]\nRuntimeWatchdogSec=10s\n' > /etc/systemd/system.conf.d/dji-hdmi-watchdog.conf
# Keep runtime logs off the SD card; configuration changes remain persistent.
mkdir -p /etc/systemd/journald.conf.d
printf '[Journal]\nStorage=volatile\nRuntimeMaxUse=32M\n' > /etc/systemd/journald.conf.d/dji-hdmi.conf
apt-get clean
# Drop content the headless appliance never reads so the distributed image
# stays small enough to flash and verify quickly. Licences stay; package
# lists come back from apt-get update, so only cached copies are removed.
rm -rf /var/lib/apt/lists/*
mkdir -p /var/lib/apt/lists/partial
find /usr/share/doc -type f ! -name copyright -delete
find /usr/share/man /usr/share/info -type f -delete
find /usr/share/doc /usr/share/man /usr/share/info -xtype l -delete
find /usr/share/locale -mindepth 1 -maxdepth 1 -type d ! -name 'en*' -exec rm -rf {} +
find /var/log -type f -delete
rm -f /tmp/dji-image.env /usr/sbin/policy-rc.d
