#!/bin/sh
# Runs as root before every start of dji-hdmi.service (ExecStartPre); every
# updater installs this file, so it also carries system fixes to old installs.
[ "$(id -u)" -eq 0 ] || { echo 'Run as root.' >&2; exit 1; }
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
