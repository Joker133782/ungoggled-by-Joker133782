# Changelog

## 0.3.52

- **New: the log is kept for 30 days, across reboots.** It used to live in the Pi's memory and was gone at every power cut, so afterwards nobody could see what had gone wrong. It now goes to the SD card and rolls over by itself: lines older than 30 days are dropped, and so is the oldest part as soon as the log reaches 100 MB, so it can never fill the card. **Download diagnostics** (System tab) contains it, together with the Pi's kernel messages (USB, under-voltage, SD card), the remembered goggles and the USB state.
- **New: much more in the log about the goggles.** When the goggles connected, which protocol was tried and which one produced the first picture, USB suspend/resume, every kind of control message the goggles send, every change of the picture state, a one-line summary every minute while goggles are attached (picture rate, decoder starts, temperature), and warnings when the Pi reports under-voltage or overheating.
- **Fixed: Goggles 3 could be sent the Goggles 2 start sequence while the drone was still off.** Goggles that have shown a picture once are now kept with the protocol that worked and never probed with the other one. Goggles seen for the first time try the other protocol less and less often (the Goggles 3 phase doubles each round, up to two minutes) instead of every twenty seconds.
- **New: a new help center** (the **?** next to every panel): searchable, with a Raspberry Pi 4B pin diagram (which GPIO sits on which pin, and what ungoggled uses), wiring pictures for the beeper, LEDs, buttons and fan, drawings of the settings, a troubleshooting section and a glossary. It works without internet, on the Pi's own Wi-Fi. The hint texts in the web interface are now a single short line; the explanations moved into the help center.
- **New: choose the goggles type in the web interface** (Settings, first panel): Automatic, Goggles 3, Goggles 2 or FPV Goggles V1/V2. A fixed choice never tries any other protocol; changing it restarts the receiver. `UNGOGGLED_GOGGLES` in `/etc/default/dji-hdmi` still works while the choice says Automatic.
- **New: up to four more buttons on GPIO pins** (Settings, Beeper, LEDs & buttons). A momentary switch between a pin and ground can shut the Pi down, reboot it, save the last 30 seconds, start/stop a recording, restart the receiver, or run a script of your own (`/var/lib/dji-hdmi/button-scripts/`). Shutdown and reboot default to a two-second hold so a knock in the bag does nothing; a button on GPIO 3 (physical pins 5 and 6; "Add power button" sets it up) also starts a Pi that was shut down.
- **Fixed: original FPV Goggles (V1/V2) that stop sending without any USB error stayed silent forever.** The start packet is now sent again after 10 seconds of silence, and the connection is rebuilt after 90 seconds. A picture decoder that had to restart no longer waits for the next parameter set: it starts at the next full picture.
- **Fixed: a changed control port (`--control-port`) was used for sending but not for the goggles' answers.**
- **Changed: one password for everything.** The control password is now also the Pi's SSH and root password (and the one the web terminal asks for). Until you set a control password nothing changes: both are still `ungoggled`. Setting or changing the control password sets the other as well, and a Pi that already has a control password takes it over the first time you log in after this update. Forgot it? Put an empty file named `reset-password` on the SD card's small boot drive (the one with `config.txt`, it opens on Windows and Mac) and start the Pi: the control password is removed (the next visit asks for a new one) and SSH/root is back to `ungoggled`. The control password must now be at least 8 characters on the Pi's side too.
- **New: change the password in the web interface** (Settings, Password): current password, new password twice. It changes the Pi's SSH/root password with it, and every other device has to log in again.
- **Fixed: the spectator page needed the internet.** Its video player was loaded from a public server, so on the Pi's own Wi-Fi (no internet) the page stayed on "Connecting…". The player now comes from the Pi itself.
- **Safer: USB drives are only opened as FAT, exFAT or NTFS** (what cameras, memory cards and Windows use). A stick with another filesystem, ext4 for example, is no longer mounted.
- **Safer login:** the session cookie is only sent over HTTPS; many wrong passwords from different addresses slow the login down for everybody; one typo weeks ago no longer counts towards a lockout; two people opening a brand-new Pi at the same moment can no longer both set the first password; the password file is written safely (a power cut can no longer leave a half-written one that stops the Pi from starting) and is readable by root only.
- **Fixed: the console said "Disconnected" on older iPhones and Android phones** although the Pi was fine. The status check used a browser feature that those phones do not have.
- **Fixed: two devices could silently undo each other's settings.** Every change is now merged into the current settings on the Pi instead of sending a whole old copy back.
- **Deleting a recording now asks first.**
- **Fixed: long flights.** With "Auto-record every flight" a flight is kept in the Pi's memory until it lands, and a long one could fill that memory. It is now saved in parts (`flight-…-2.h264`, …) when the memory gets full; the five newest flights are kept, all parts of a flight count as one. Saving no longer freezes the picture or the web page, a copy that is cut short never shows up as a finished recording, and switching off the Pi waits for the copy to finish.
- **Fixed: downloading a big recording could run the Pi out of memory** and take the live picture down with it. Recordings are now streamed from the card, and a download that broke can continue.
- **Fixed: a Wi-Fi scan that the phone gave up on blocked every later Wi-Fi change** (and updates) until the Pi's service was restarted.
- **Fixed: a wrong power-button pin switched the Pi off right after every start.** The button only counts once it has been seen released.
- **Fixed: the picture could freeze for seconds when a USB stick was plugged in or pulled** with "Record to USB" on. A drive that is slow to open no longer holds up the live picture; a stick that cannot be opened is retried every 30 seconds instead of every 2.
- **Fixed: the screen could stay dead while the web interface still worked** after a rare internal error. The service now restarts itself.
- **Fixed: "stretch to fill" could switch itself off for the whole flight** when the picture decoder was only slow to start the very first time.
- **Fixed: a clip tapped in the moment the previous one ended was dropped.**
- **Fixed: 4K at 60 Hz is never switched on next to a manual `hevc_freq` or `over_voltage` setting** in `config.txt`: that combination switches the Pi's HDMI output off.
- The spectator stream at "Low quality" can no longer slow the live HDMI picture down when the Pi cannot keep up with re-encoding; and if it cannot start, it is retried every 15 seconds instead of constantly.
- The three built-in waiting-screen videos are no longer rewritten (15 MB) on every start, which protects the SD card.
- The service gets 30 seconds to stop (it was 5) so a flight can be saved, and a higher open-files limit.

## 0.3.51

- **New: 4K at 60 Hz on the first HDMI port.** The Pi's boot setting `hdmi_enable_4kp60` is now switched on by the update, and in new flash images. The video chip then runs a little faster (550 instead of 500 MHz), which plays big action-camera clips a few pictures per second smoother, and a monitor that can do it now lists 3840x2160 at 50 and 60 Hz in Settings, HDMI. This needs one restart of the Pi after the update; the interface tells you when. 4K at 60 Hz only works on the HDMI port next to the USB-C port and needs a high-speed HDMI cable; "Automatic" still picks 1080p. A value set by hand in `config.txt` wins.
- **New: the fan switches on from 60 C.** A fan on GPIO 14 (what raspi-config sets up under Performance Options, Fan) is now controlled by the Pi's own temperature from 60 C by default, also after the one restart. A fan that was already set up by hand (another pin or temperature) is left as it is. A Pi without a fan is not affected.

## 0.3.50

- **Fixed: pulling the action camera out of the Pi while a clip was playing left the screen on the Pi's text console, with the player and the list of clips still shown in the web interface.** Playback now stops by itself, exactly as if Stop had been pressed, and the idle screen comes back within a few seconds. The same happens whenever the player stops for another reason (the clip ends, or the file can no longer be read). The list of clips is hidden while no USB drive is detected, and a clip from a drive that is gone can no longer be started.
- A drive pulled out during playback no longer leaves a dead mount behind on the Pi.

## 0.3.49

- **Fixed: after switching the monitor off and on again, the Pi showed its text console instead of the picture until the service was restarted.** The Pi now notices that its picture is no longer on the output (within a few seconds) and puts it back, whether the monitor was away for a moment or for a while.
- **New: stretch to fill the screen.** Two new switches in Settings → HDMI, "Stretch the live picture to fill the screen" and "Stretch USB videos to fill the screen", pull a 4:3 picture over the whole 16:9 screen instead of showing black bars at the sides. Both are off by default, so nothing changes unless you switch them on; the USB one also works while a video is playing. It makes no difference to how smoothly a video plays. The Pi's display chip refuses to stretch some live picture sizes; those are shown as they arrive, after a short hiccup when the picture starts.
- **Slightly smoother USB playback of big action-cam clips.** The Pi no longer redraws a full-screen layer for on-screen text that this player never uses, which frees memory bandwidth for the video decoder: about 10 % more pictures per second on a DJI Action 2 4K 60 fps clip (35 to 38) and an Osmo/O4 clip (45 to 50). Real time at 60 fps is still out of reach for the Pi 4's decoder; the speed line under the time bar shows how close a clip gets. (An idea to gain more by showing the picture a little smaller was tried and dropped: it made the colours wrong.)
- **Fixed: large action-cam clips (e.g. DJI Action 2 4K) could turn the HDMI picture black during USB playback.** The Pi's video chip lowered its own clock while decoding, too far for scaling such a big picture down to the screen. It now keeps a minimum clock speed. This and the two items below are boot settings: they arrive with the update and switch on after the Pi's next restart — the interface shows a "Restart the Pi once" notice until then (no automatic restart, so never mid-flight).
- The Pi's video memory pool is raised to the size new flash images already use (640 MiB) on older installations that still have the smaller default, so HEVC clips that need it play there too.
- The hardware HEVC (H.265) decoder now gets its own reserved memory for its internal working buffers (a fixed version of the Raspberry Pi's decoder driver, built for exactly this Pi kernel), so video playback can no longer starve it of memory. Because that driver is built for one specific kernel, the Pi's kernel is now kept at that version (no kernel upgrades through apt) until an official kernel carries the fix. On any other kernel this part is skipped automatically and the stock driver is used; if the fixed driver ever fails to load, the Pi switches back to the stock one by itself.
- Flash images and updated Pis now share one mechanism for these boot settings, so a freshly flashed card and an updated one end up identical.
- **Fixed: every update and every switch back reset a manually chosen HDMI output to automatic.** The settings carry-over did not know that one setting; it is kept now.
- The Pi's own access point stays on channel 36. The move to channel 100 announced in 0.3.42 was withdrawn: that channel needs radar detection (DFS), and the Pi 4's Wi-Fi chip could not start the access point on it at all.
- **New: the USB video player now shows how smoothly a clip really plays.** A line under the time bar gives the pictures per second that reach the screen and the speed compared with real time (1.00× is real time). The same number appears in the HDMI output box and in the "TV" line of the frame-rate graph, which stayed at 0 during USB playback before. Very large action-camera clips are more than the Pi 4 can decode in real time; they play in smooth slow motion, and now you can see how slow.

## 0.3.42

- **Fixed: updating to 0.3.41 always failed and rolled back.** 0.3.41 added a password gate to the control interface, but the update process's own internal health check calls the status endpoint without logging in — so every single update to 0.3.41 failed its own check and was undone automatically. If you're reading this from a Software update panel that still says 0.3.40 (or earlier), this is why, and this release fixes it. Sorry — this should have been caught before 0.3.41 shipped.
- "Fallback image" renamed to "Idle Screen" throughout the interface and help pages — same feature, clearer name.
- A keyboard plugged into the Pi can now reach a login prompt (Ctrl+Alt+F2), independent of whatever ungoggled itself is showing on HDMI.
- The "last 30 minutes" history now also records CPU load, RAM use, SD card read/write and Wi-Fi/LAN throughput — not shown anywhere yet, just being recorded so the UI work can build on real data.
- The Pi's own access point moves from channel 36 to channel 100 in the next flash image, to stop overlapping either of the O4 Air Unit Pro's two video bands or 2.4GHz (where RC control often lives) — **not yet verified on real hardware that DFS works correctly in AP mode on this chip, see TODO.md before trusting this on a real flight.**
- Fixed: saving one setting (e.g. toggling Live preview) could silently revert a different setting changed moments earlier elsewhere on the page (e.g. Auto-record), because Recording, Spectator stream and the rest of the page each kept their own separate copy of settings and saved the whole thing. Now there's one shared copy.
- Fixed: confirmation dialogs (reboot, shut down) opened pinned to the top-left corner instead of centered.
- Fixed: the page could scroll sideways on a phone once a panel had enough content in it.
- New: the control-interface login now locks out an IP address for 5 minutes after 5 wrong passwords in a row — a prerequisite for the upcoming terminal feature, where a guessed password would mean a root shell, not just changed settings.

## 0.3.41

- New: if an update's automatic health check fails and rolls the update back, there's now a "Retry without the health check" option right there in the Software update panel — a safety valve for a health check that's wrong rather than the update actually being broken (this has happened before, see 0.3.35). Only offered for that specific failure, never for a corrupt download or another real problem.
- Fixed: the spectator stream page never showed any video, on any browser or device — the player was loading its video data through a URL that only resolves correctly on the page itself, not inside the background worker it actually ran in.
- Fixed: the spectator stream (High quality) could fail to decode on stricter devices because the goggles declare an H.264 level far higher than 1080p30 actually needs; it's now corrected before sending.
- The spectator stream page now also works on iPhone/iPad Safari (needed a newer build of the player library).
- New: the control interface (settings, recordings, Wi-Fi, everything except the spectator stream page) now needs a password — anyone on the Wi-Fi could previously reach and change anything, including through a shared spectator-stream link. The first visit asks you to set one; the spectator stream page itself never asks for it.
- Trimmed several panels' always-shown explanatory text that only duplicated the page's own "?" help link — less scrolling, especially on a phone. Nothing it explained changed; the full explanation is still one tap away in Help. Added the two entries that help page was missing (the skip-health-check retry option, and the new control password).

## 0.3.40

- Fixed: USB ActionCam HEVC (H.265) video — clips from cameras like the DJI Osmo Nano reliably failed to play at all (silently fell back to no picture) because the hardware decoder couldn't get enough memory for the stream's full resolution. The Pi's video memory pool is now sized to cover this.
- Fixed: USB ActionCam video playback was choppy even when it did play — every frame was being resized by the CPU instead of the display hardware. Now plays back smoothly in real time.
- USB ActionCam thumbnail generation is roughly 3x faster, and no longer competes with video playback for the Pi's video memory — previously, a thumbnail still being generated could make a video fail to start.
- Fixed: switching to a different clip while one was already playing left the old clip's progress bar position on screen instead of resetting for the new one.
- Fixed: unplugging a USB drive while browsing its videos didn't clear the list until manually rescanning.
- Fixed: the web interface could keep running an old cached copy of itself after installing an update, because the browser was never told a newer one was available.

## 0.3.39

- New: the Wi-Fi panel can now join an existing Wi-Fi network instead of just broadcasting its own hotspot — scan for nearby networks (or type a hidden one's name), enter its password, and join. The current network and IP show on the HDMI screen while connected, since the Pi's own hotspot goes down for as long as it's joined elsewhere. Falls back to its own hotspot on its own if the join doesn't succeed within 30 seconds — no reboot needed. A new "Always broadcast the access point after a reboot" switch controls whether a reboot resumes the joined network instead.
- The Pi's own Wi-Fi hotspot now uses 5 GHz (channel 36) instead of 2.4 GHz, for a faster, less congested spectator stream.
- Fixed: a status message meant for the HDMI screen (the "SD card full" warning, and now the new Wi-Fi status above) could stay hidden behind the looping waiting video, because the loop never stopped once a message wanted the screen instead.
- Fixed: the "record to USB" setting was silently reset to off on every update — a migration case for it was missing.

## 0.3.38

- USB ActionCam video list now appears almost instantly, even on a drive full of clips. Previously it generated every thumbnail (which can take up to ~20-30s per large 4K file) before showing the list at all; thumbnails now load in one at a time in the background after the list itself is already visible.
- The video list now shows when each clip was recorded, and sorts newest first — makes it much easier to tell whose footage is whose after a group flying session.
- Diagnostic-only: USB ActionCam playback now logs which video decoder/output path mpv actually used, to help track down choppy playback on high-resolution (e.g. 4K HEVC) clips — no behavior change yet, this is groundwork for actually fixing it once we can see what's happening on real hardware.

## 0.3.37

- Fixed: with HDMI output on "Automatic", moving the cable from one micro-HDMI port to the other left the screen black instead of switching to it. The port-watcher only ever checked whether its current port got a better signal, never whether it lost its signal entirely — so it never noticed the display had moved. Only matters in "Automatic" mode; a manually selected port still reports its own connection state as before.

## 0.3.36

- No functional change — a small test release to confirm the update mechanism itself works end to end again after the 0.3.35 health-check fix.

## 0.3.35

- Found and fixed the real reason every update since 0.3.30 failed its health check: the check asks the new release for its status over plain HTTP, but plain HTTP has redirected to HTTPS since 0.3.30 — and that redirect comes back empty, which doesn't parse as a status report. The check now follows that redirect instead of treating the empty response as a (failing) answer.

## 0.3.34

- Fixed the update health check's diagnostic info from 0.3.33 never actually showing up anywhere: it was only printed to a log that doesn't reliably survive a failed update's restart/rollback. The specific reason (unreachable, wrong version/build, or just not up long enough yet) is now saved directly into the update status instead, so it shows up wherever the "Change failed" message already does.

## 0.3.33

- The update health check now logs what it's actually seeing on each attempt (unreachable, wrong version/build, or just not up long enough yet), instead of only ever saying "did not pass its health check" with nothing else in the logs to explain why.

## 0.3.32

- HDMI output now works on either micro-HDMI port, not just the one closest to USB-C power — and when two displays are plugged in at once, a new "Output port" setting lets you pick which one shows the picture.
- USB drives now show up in the web interface as soon as they're detected (device name), even before scanning for videos, and a progress indicator shows while a scan for videos is running instead of looking stuck.
- New setting: "Record to USB drive instead of SD card" — recordings go to a plugged-in USB mass-storage drive when one is present, falling back to the SD card automatically when it's unplugged.
- New "Download diagnostics" button (System panel) — bundles recent logs, update history and hardware info into one file you can download, even from a phone, to attach when reporting a problem.

## 0.3.31

- Clearer HDMI error when a display is plugged into the wrong micro-HDMI port: "A display is connected, but to the wrong micro-HDMI port. Use the port closest to the USB-C power input." instead of a silent blank screen. Shown both as a banner on the HDMI panel and in the Goggles/HDMI status card's tooltip. Found after two real Pi 4s both turned out to have the cable in the second (non-default) port — the app only ever looked for the one closest to USB-C power.

## 0.3.30

- Plain HTTP now redirects to HTTPS instead of serving the app on both: whichever URL gets typed or bookmarked first always lands on the one secure origin, so a PWA install prompt (and anything else that cares about a secure context) isn't missed just because someone opened the HTTP link.
- Added a way to actually get Chrome's real "Install app" prompt instead of just a home-screen shortcut: download the self-signed certificate from `/api/tls-cert.crt` and trust it once per phone as a CA (Android: Settings → Security → Encryption & credentials → Install a certificate). Documented in the in-app wiki under "Installing as an app" — entirely optional, the shortcut already works fine for everyday use.

## 0.3.29

- The web interface is now a real installable PWA on Android: added `manifest.json` (name, theme color, fullscreen display) and 192/512px icons, wired into the page head. Chrome previously only offered "Create shortcut" instead of "Install app" because no manifest existed.

## 0.3.28 (repackaged)

- Fix the update file being rejected by an installer older than 0.3.7 with a confusing "Unexpected update file: packages/..." error: `scripts/package-update.py` only bundles mpv's offline-install `.deb` packages when `build/mpv-debs` actually has content (a one-time migration aid per `scripts/bundle-mpv.sh`'s own comment, not meant to ship forever), and now sets `minimum_updater` to `0.3.7` only when those files are actually present, so an installer too old to understand them gets the correct "install an intermediate version first" message instead of a cryptic rejection. Found by testing a real manual upload from a device running 0.3.2.

## 0.3.28

- Three selectable waiting-screen videos for the HDMI fallback (2D logo loop, 3D loop, FPV comic), demuxed to Annex-B H.264 with the same pipeline as a manual upload. The 2D logo loop is the default on a fresh install, replacing the previous placeholder loop. All three are treated as permanent built-in choices (refreshed on every start, not just first boot, so an in-place update brings them to an already-provisioned Pi too), leaving one library slot free for a custom upload.
- Favicon refined to match the finished brand kit (bordered tile, panel background color).

## 0.3.27

- Header bar (logo, connection status, version) is centered, and fades to much more transparent while scrolling so more of the background shows through.
- "Last 30 minutes" and HDMI are the same width now, matching the Wi-Fi/Spectator stream and Recording/USB row pairs below — all three side-by-side rows line up visually.
- Timeline hover tooltip shows a proper time ("20m 34s ago") instead of raw seconds.
- Fallback image: the preview box and the image/video picker tiles are noticeably wider and line up with the Upload/Delete buttons below instead of looking cramped.

## 0.3.26

- Every button throughout the web interface now has a matching icon (Apply, Upload, Download, Delete, Test buzzer, Play/Pause/Stop, Rescan, Check for updates, Install, Switch version, Cancel/Confirm, and more).
- All icons in the web interface — panel headings and buttons alike — render in the ungoggled accent (teal) color; icons inside filled teal (primary) buttons use the dark ink color instead so they stay visible.

## 0.3.25

- Every panel heading is centered now, not left-aligned.
- Fallback image: the preview box ("Video loop selected" etc.) is centered above its buttons instead of sticking to the left edge.
- Action buttons are noticeably wider; Start/Stop receiver and Reconnect goggles got their own icons (play/stop/refresh).

## 0.3.24

- Fix the recurring side-by-side row misalignment for good: Spectator stream and Wi-Fi access point, and now also Recording and USB - ActionCam Video Player (newly paired, same topic), sit at exactly equal height and position — each row's own leftover `margin-top` (from before they were paired) was fighting the row wrapper's margin every time.
- Per-toggle explanations ("— buffers in RAM while...") are real bullet points now, not dash-prefixed text.
- Help text throughout the web interface is justified and set in a bordered box; panel headings have a stronger accent-tinted underline; action buttons are centered.
- Fallback image panel: centered throughout.

## 0.3.23

- Software update: "Check for updates" now checks the owner's own Nextcloud share (not GitHub — unresolved with the upstream project, see project notes) for a newer version, shows its size and release notes, and downloads it over the viewing phone's own connection rather than the Pi's — the Pi itself never needs internet access. A splash after a successful download offers installing right away or leaving it staged to install later. Verified end to end: the version/size check, and a real multipart upload of a full-size package to the update endpoint, both confirmed working; the download step itself needs confirming on a real phone next (a sandboxed test browser couldn't complete a large upload to a LAN address, unrelated to the app).
- The in-app wiki's Updates page lists what's shipped so far in plain language, and the GitHub release check (unused for now) no longer points at the upstream project.

## 0.3.22

- System moved to the very bottom, below software updates; Spectator stream now sits beside Wi-Fi access point instead.
- System's three buttons (Sync Time, Reboot Pi, Shut down Pi) are now equal width and spread across the box, each with its own icon; "Sync time from this browser" shortened to "Sync Time".
- Spectator stream: the enable toggle and quality dropdown are now two independent toggles, "High quality" and "Low quality" — switching one on turns the other off, each with its own one-line explanation.
- Every panel's icon is now in the ungoggled accent color instead of grey.

## 0.3.21

- A second, smaller background glow left of center, so something is still visible on a phone's fullscreen crop (which cut off most of the original top-right one).
- Panels are more translucent again.
- Wi-Fi access point + System, and Software update + Installed versions, now sit side by side instead of each taking the full width — less scrolling, and the page doesn't look as left-heavy with everything stacked.

## 0.3.20

- Serve the web interface over HTTPS too (port 443, alongside the existing port 80), with a self-signed certificate generated once on first use. A real certificate needs a public domain name, which a private access point at a fixed local IP doesn't have; the one-time "connection isn't private" browser warning is the trade-off. Without this, Chrome and other browsers won't treat the page as an installable fullscreen app — "Add to Home Screen" falls back to a plain bookmark that still shows the address bar. Verified end to end: the certificate is generated and reused correctly, and a real browser completes the TLS handshake and loads the real interface over it. Disable with `--tls-listen ""`.

## 0.3.19

- Fix the HDMI panel and the "Last 30 minutes" chart not sitting at an equal distance from Preview: a leftover margin on the chart (from before it moved beside HDMI) was pushing it down independently of its row. Both now line up exactly.
- Panels are noticeably more translucent, so the background pattern actually shows through.

## 0.3.18

- The HDMI panel now stretches to match the height of the chart beside it, instead of leaving it visibly shorter.
- The header is a proper card matching the width of everything else below it (previously bled to the full window width via a now-removed trick), and a little smaller than the last pass.
- Panels are now translucent with a blur, so the page's background pattern shows through subtly without hurting readability.

## 0.3.17

- Layout: Preview now spans the full width; the HDMI panel moved beside the "Last 30 minutes" chart instead of leaving empty space below it. Software update and installed versions moved to the very bottom, next to the other rarely-used settings.
- The header (logo and connection status) is twice as large, to stand out more.

## 0.3.16

- Reorganized the web interface by how often each part is actually used: "Beeper, LEDs & power button" and "Fallback image" (split out of the HDMI panel) now sit at the very bottom, below the experimental features.
- Merged the header's Goggles/Video/HDMI status pills into the three stat cards above: each now carries its own red/green status icon (the ungoggled mark for goggles, a monitor for HDMI), and video streaming status is shown inside the goggles card, since it's a property of the goggles connection.

## 0.3.15

- New logo: a goggles-shaped mark with radio-wave arcs, replacing the previous crosshair icon in the header and as the browser favicon.
- A subtle, fully transparent radio-wave background behind the web interface, fading in from the top-right corner.
- The HDMI fallback now defaults to a short branded loop video ("ungoggled", waiting for the goggles' signal) instead of a plain dark background on a fresh install; the previous static image stays available to pick instead.

## 0.3.14

- Redesigned the web interface: a new dark color palette and typography, an icon plus a "?" help link on every panel, and shorter, plain-language in-page descriptions. The "?" links open a new in-app help wiki at `/wiki.html`, with a page per feature written for pilots rather than technicians.

## 0.3.13

- Recording: new "Crash buffer (last 30 seconds)" toggle, matching the DJI goggles' own instant-replay — the always-on ~30 s ring buffer now auto-saves itself the moment the feed drops, keeping the 3 most recent clips (a separate, smaller rotation from the 5 kept flights). The manual "save last 30 seconds" button keeps working independently, for a moment without a feed drop.
- Renamed "USB videos" to "USB - ActionCam Video Player" in the web interface.

## 0.3.12

- Fix the spectator stream's "Event" quality not playing in any browser: found by testing the real relay pipeline in a real browser after a prior fix regressed it. `x264enc` has no `profile` property on current GStreamer (it only exists as a caps field), so the earlier attempt to pin it to constrain the H.264 profile made the relay process fail to start at all, silently, rather than merely encode an incompatible profile as before. Fixed by constraining the profile with a capsfilter after the encoder instead of a property; verified end-to-end in a real browser against the exact relay pipeline, now decoding cleanly (Baseline profile, broadly compatible).

## 0.3.11

- Fix USB videos not appearing after an unclean restart: found by testing real restarts against a real mounted action cam over USB passthrough. A device already mounted from a previous run that did not unmount cleanly made the next mount attempt fail (already mounted) and silently left the device untracked, so it never showed up in the list even though it was reachable on disk. Now adopts an existing mount for a device instead of trying to mount over it.

## 0.3.10

- Recording: new "Auto-record every flight" toggle, buffering each live session in RAM (tmpfs) and saving it to the SD card as soon as the feed drops, keeping the 5 most recent flights. Manual start/stop recording stays available independently.
- Recording: saved files now get the same cached thumbnail treatment as USB videos, with download/delete/play-on-HDMI in one list.
- USB videos: list the camera's actual product name (e.g. "DJI DJIAction2"), not the raw Linux block device name (e.g. "sdc"), read from the USB device's own descriptor strings rather than the often-generic SCSI INQUIRY vendor/model.
- Checkboxes across the web interface (preview, beeper/LEDs, spectator stream, auto-record) now render as a green/red toggle switch instead of a native checkbox — one shared CSS rule, since every checkbox already used the same wrapper.
- Fixed USB-video thumbnails for real action-cam footage: found by testing against a DJI Action 2 over USB passthrough (see below) rather than only synthetic clips. A caps filter with only `width=` let `videoscale` leave height unchanged and rewrite the pixel-aspect-ratio to compensate instead of actually resizing, invisible at 16:9 but turning a real 4:3 4096x3072 recording into a 160x3072 thumbnail; fixed by forcing `pixel-aspect-ratio=1/1`. Also raised the generation timeout from 5s to 30s: a real 3.7 GB 4K60 HEVC clip took about 20s for GStreamer to read and index before decoding anything.

## 0.3.9

- Experimental spectator stream: phones/tablets on the Pi's own Wi-Fi can watch the live feed in a browser at `/watch.html`, separate from the HDMI output. MPEG-TS over a long-lived HTTP connection, decoded client-side with mpegts.js — real H.264, much lower latency than HLS. Toggle and quality ("High", relayed unmodified, ~1-2 viewers; "Event", transcoded lower-bitrate for more viewers) configurable in the web interface under **Spectator stream**, with the actual Wi-Fi bandwidth trade-off explained there. Verified end-to-end in a real browser against synthetic test video.

## 0.3.8

- Experimental: record the live goggles feed to the SD card (no re-encoding), started/stopped from the web interface, plus a "save last 30 seconds" shortcut backed by an always-on ring buffer. Stops itself, and won't start, below 2 GB free space; the HDMI output shows a "SD card full" notice once the goggles feed stops, without interrupting a live picture to show it. Recordings list in the web interface for download, deletion, or playback on the HDMI output.

## 0.3.7

- The update file now carries mpv's `.deb` packages and installs them offline after a successful update, so a Pi on an older release picks up USB-drive video playback automatically without needing internet on the Pi or a manual `apt install`. Best-effort: installing the packages never affects the rest of the update, succeeded or failed.

## 0.3.6

- Fix USB video seeking: mpv's IPC command array rejected the combined `absolute+exact` seek flag. Found by actually running the thumbnail GStreamer pipeline and the mpv IPC protocol for the first time, after getting sudo on the build host; both now verified to work as coded (except real DRM/KMS output, still unverified without a Pi and monitor).

## 0.3.5

- USB videos: play/pause and seek from the web interface (video stays on the HDMI output only), switching the playback backend from GStreamer to mpv. Needs `mpv` on the Pi, now included in the SD-card image; an already-flashed Pi needs a re-flash or `apt install mpv`.
- USB videos: thumbnails, cached in the tmpfs runtime directory so they never accumulate on the SD card and are gone after every reboot or power loss.

## 0.3.4

- Experimental: browse video files on a USB drive or action camera plugged into a regular USB-A host port and play one on the HDMI output, listed in the web interface. Not hardware-verified for every camera and codec; resolution and codec support depend on what the Pi's GStreamer install can decode.

## 0.3.3

- Experimental, hardware-unverified USB-host receiver for the original DJI FPV Goggles (V1/V2), based on the community-reverse-engineered `voc-poc` protocol. Opt in with `UNGOGGLED_GOGGLES=v1v2`; requires a USB-A host port, not the gadget USB-C port. The exact stream framing is not fully understood even upstream.

## 0.3.2

- Loop an uploaded MP4 (H.264, up to 1080p and 64 MiB) as the HDMI fallback while no video arrives. Uploads are demuxed without re-encoding and use the hardware decoder.
- Experimental DJI Goggles 2 support. The receiver tries the Goggles 3 and Goggles 2 start sequences, keeps the one that produces video and remembers it per pair of goggles by their USB identification. `UNGOGGLED_GOGGLES` and `UNGOGGLED_ACCESSORY_PID` in `/etc/default/dji-hdmi` override the choice.
- List installed versions in the web interface and switch back to an earlier one through the same safe transaction as an update, with automatic restore if it does not start.
- Record updates, switches and boot recoveries in an update history.
- Keep the SD-card image release, the active and preceding releases and one older update.

## 0.3.1

- Set O3/O4 recording timestamps from the Pi clock after the goggles USB session starts.
- Show the Pi clock in the web interface and copy the browser time (and timezone) when they differ by more than a second.
- Optional setting (off by default) to beep every 5 seconds until the Pi clock has been synced from a browser or NTP since boot.
- Default GPIO pins: piezo 6, shutdown button 19, goggles LED 17, video LED 23, HDMI LED 24.
- Offer every HDMI mode the display advertises, including interlaced modes and modes above 1080p60. Automatic mode still prefers progressive 1080p.

## 0.3.0

- Serve the web interface on standard HTTP port 80.
- Upload and install software updates from the web interface, with upload progress and automatic reconnection.
- One complete update file supports Pi 4 installations running 64-bit Raspberry Pi OS, without installing intermediate application releases.
- Preserve compatible HDMI, preview and fallback settings. Keep uploaded images, Wi-Fi settings and login credentials.
- Restore the previous application and settings if the new version fails its startup check. Recover interrupted installations at boot.
- Check GitHub releases from the browser and show newer stable versions, release notes and download links.
- Fix V4L2 hardware decoder negotiation for Goggles streams advertising H.264 level 5.2, without changing the encoded video.
- Fix fresh-image Wi-Fi being disabled by the stock NetworkManager radio state, and persist the configured radio country at boot.
- Add semantic version management and automated release builds containing an SD-card image and an ARM64 update file.
- Halve the SD-card image: the root filesystem is sized to its contents plus working headroom, and documentation, translations and cached package lists the appliance never reads are left out. The partition still expands to the whole card on first boot.
- Ship only the current application release in the SD-card image, instead of also carrying the release directories of earlier builds.
- Show goggles, video and HDMI state in a header that stays visible while scrolling. Goggles state comes from the USB controller, so a cable that is attached but has not finished the accessory handshake is no longer reported as disconnected.
- Report an attached HDMI display only when it answers with an EDID; the image forces an HDMI mode, which leaves the connector reporting "connected" with nothing plugged in.
- Recover on its own when the goggles keep the port configured but the gadget stops receiving events after an update restart, instead of needing a manual service restart.

## 0.2.0

- Live HDMI output for Goggles 3 with O3 Air Unit and O4 Air Unit Pro.
- Browser preview, HDMI controls, fallback images, Pi temperature and a 30-minute transmission history.
- Wi-Fi configuration and preconfigured Raspberry Pi OS Lite images.
- Rename the project to ungoggled. Default Wi-Fi SSID, Wi-Fi password and root password are `ungoggled`.
