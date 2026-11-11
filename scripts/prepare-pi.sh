#!/bin/sh
# Runs as root before every start of dji-hdmi.service (ExecStartPre); every
# updater installs this file, so it also carries system fixes to old installs.
[ "$(id -u)" -eq 0 ] || { echo 'Run as root.' >&2; exit 1; }
# Forgotten control password. The same password also opens SSH, so there is no
# way back in over the network; the way back is physical access to the SD card:
# an empty file named "reset-password" on the boot partition (the small one
# with config.txt, which Windows and macOS can open). It removes the control
# password (the next visit asks for a new one, as on first start), puts the
# SSH/root password back to the default, and deletes the file. Never fatal.
reset_marker=/boot/firmware/reset-password
if [ -e "$reset_marker" ]; then
    rm -f /var/lib/dji-hdmi/auth.json
    if printf 'root:ungoggled\n' | chpasswd; then
        echo 'Control password removed and SSH/root password reset to the default (reset-password file found).' >&2
    else
        echo 'Control password removed, but the SSH/root password could not be reset.' >&2
    fi
    rm -f "$reset_marker"
fi
# The service log survives a reboot and is kept for 30 days: the Pi is usually
# powered from the flight battery, so a log in RAM is gone before anyone looks at
# what went wrong. Age and size are both limited (100 MB), so the log can never
# fill the SD card. Written once; the same settings are in provision-image.sh for
# fresh images. Never fatal.
journald_conf=/etc/systemd/journald.conf.d/dji-hdmi.conf
journald_want='[Journal]
Storage=persistent
SystemMaxUse=100M
SystemMaxFileSize=8M
MaxFileSec=1day
MaxRetentionSec=30day
RuntimeMaxUse=32M'
if [ "$(cat "$journald_conf" 2>/dev/null)" != "$journald_want" ]; then
    if mkdir -p /etc/systemd/journald.conf.d \
        && printf '%s\n' "$journald_want" > "$journald_conf" \
        && systemctl restart systemd-journald \
        && journalctl --flush; then
        echo 'Service log is now kept across reboots (30 days, 100 MB at most).' >&2
    else
        echo 'Could not switch the service log to persistent storage; continuing.' >&2
    fi
fi
# Boot configuration and the HEVC decoder module (src/system_fixes.rs). Takes
# effect after the next reboot; never reboots and must never keep the receiver
# from starting, so its result is only logged. A release without the command
# (a switch back to an older one) just prints a usage error here.
app=/opt/dji-hdmi/current/bin/ungoggled
if [ -x "$app" ]; then
    timeout 60 "$app" system-fixes apply || echo "system-fixes did not complete (exit $?); continuing" >&2
fi
# Configure the USB device side.
set -eu
modprobe dwc2
modprobe libcomposite
modprobe usb_f_fs
if ! ls /sys/class/udc/*/uevent >/dev/null 2>&1; then
    echo 'No USB device controller. Add dtoverlay=dwc2,dr_mode=peripheral to the Pi boot config and reboot.' >&2
    exit 1
fi
if ! mountpoint -q /sys/kernel/config; then
    mount -t configfs configfs /sys/kernel/config
fi
for element in h264parse capssetter kmssink fpsdisplaysink jpegenc videorate videoscale videoconvert; do
    gst-inspect-1.0 "$element" >/dev/null
done
echo 'Configfs, FunctionFS and video plugins ready.'
