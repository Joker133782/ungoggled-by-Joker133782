# Supported goggles

[← Back to the README](../README.md)

| Goggles | Air unit | Status | Connect to |
|---|---|---|---|
| **DJI Goggles 3** | O3 Air Unit, O4 Air Unit Pro | Tested with live HDMI output | Pi **USB-C** port |
| DJI Goggles 2 | — | Experimental; only the Goggles 3 path is hardware-verified | Pi **USB-C** port |
| DJI FPV Goggles V1/V2 | — | Experimental, unverified | Pi **USB-A** port |

Goggles 3 with both **O3 Air Unit** and **O4 Air Unit Pro** have been tested with
live HDMI output. Use the Pi's USB-C port for the goggles and power the Pi
separately (see [INSTALL.md](INSTALL.md#hardware-and-wiring)). The web interface
shows the protocol in use under **Diagnostics → Goggles protocol** (System tab).

## Goggles 2 (experimental)

Goggles 2 need a different start sequence than Goggles 3: DJI Fly's arming
requests and keepalives on control port `0x7530`, taken from the MIT-licensed
[dji-o3-usb-decoder](https://github.com/dr00min/o3-usb-decoder) capture. The
receiver remembers which protocol produced video for each pair of goggles,
identified by the AOA strings they send while connecting
(`/var/lib/dji-hdmi/goggles-protocol.json`). Known goggles start with their
protocol and **stay with it**: goggles that have shown a picture once are never
probed with the other generation's requests again, because a Goggles 3 whose
drone is simply not switched on yet must not be sent the Goggles 2 arming
burst. Unknown goggles (and entries written by older releases that saw both
generations) start with the Goggles 3 probe and switch after 8 s without video
(Goggles 2 after 12 s); each further round the Goggles 3 phase doubles, up to
two minutes, so goggles without a drone get the other protocol's requests less
and less often. The active protocol is shown under **Diagnostics → Goggles
protocol** and written to the log (`Goggles protocol: … first video N s after
the session started`). Only the Goggles 3 path is hardware-verified in this
project.

To make the receiver forget what it learned (new goggles with identical
identification, or a wrong first guess), delete the memory file and restart:

```
rm /var/lib/dji-hdmi/goggles-protocol.json && systemctl restart dji-hdmi
```

**Goggles 2 that show nothing?** The log tells where it stops (the Pi keeps it
for 30 days, also in the support bundle). With the goggles plugged in:

```
cat /sys/class/udc/*/state
journalctl -u dji-hdmi -b | grep -E "FFS setup|AOA string|Goggles protocol|DUML"
```

`not attached` means role or cable (the goggles must be the USB host; an OTG
adapter belongs on the goggles' end). `configured` but no `AOA string` lines
means the accessory handshake never started. AOA strings plus `Goggles 2:
arming` but no video means the goggles do not accept the replayed start
sequence; the `DUML` lines then show which messages they answer at all.

**Choosing the goggles type in the web interface (Settings, first panel):**
Automatic, Goggles 3, Goggles 2 or FPV Goggles V1/V2. A fixed choice never tries
the other protocol (it ignores the remembered goggles) and restarts the receiver
when changed. This is the easy way to force a protocol.

To force a protocol or the accessory PID from the system side instead, add to
`/etc/default/dji-hdmi` and restart `dji-hdmi` (the value is used while the web
interface says Automatic):

```
UNGOGGLED_GOGGLES=goggles2        # auto (default), goggles3 or goggles2
UNGOGGLED_ACCESSORY_PID=11521     # 0x2d01, the PID DJI Fly presents; default 11520 (0x2d00)
```

## Goggles V1/V2 (experimental, unverified)

The original DJI FPV Goggles (V1/V2) work the other way around from Goggles
2/3: the goggles expose a plain USB bulk interface and something else has to
be the USB *host* (Goggles 2/3 make the Pi the accessory instead). This is
based on the community project [voc-poc](https://github.com/fpv-wtf/voc-poc),
which itself calls the exact stream framing "currently largely unknown" and
only claims it "sort of just works" once piped into ffmpeg. **This path has
not run against real V1/V2 hardware and may not produce a usable picture.**

Plug the goggles into one of the Pi's regular USB-A ports (not the USB-C
gadget port used for Goggles 2/3) with a data-capable cable, then set in
`/etc/default/dji-hdmi` and restart `dji-hdmi`:

```
UNGOGGLED_GOGGLES=v1v2
```

If the goggles stop sending without any USB error (the air unit unlinked, for
example), the start packet is sent again after 10 seconds of silence and the
connection is rebuilt after 90 seconds; both are written to the log.
