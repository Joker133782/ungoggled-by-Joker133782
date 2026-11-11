# TODO

- [x] **2026-11-11 (evening): UI hints shortened + new wiki.** User: "nicht so extrem viele Erklaerungstexte in der WebUI ... kurze knappe praegnante Erklaerungen", wiki with a Pi 4B pin picture, "das Ungoggled CI nicht im Wiki", Opus subagent for "Wiki UI, Aufbau, Suche ... Beispiele, Grafiken". Done: all UI help texts >200 chars shortened (App HDMI, AuthGate, Goggles, Hardware x3 incl. links to `#gpio-pinout` / `#extra-buttons`, IdleScreen, Password, SoftwareUpdate x2, Versions, Wifi; originals saved for the wiki). Opus rebuilt `web/public/wiki.html` (146 KB) + `wiki/wiki.css` + `wiki/wiki.js` (about 206 KB, no bitmaps, offline): 24 chapters, search with typo tolerance, drawer nav, pin diagram + 40-pin table, wiring sketches, panel drawings, 17 troubleshooting questions, glossary. I verified: no external URLs/requests, 40/40 pins vs the canonical header map, all UI-linked anchors exist, no horizontal scroll at 360 px, anchors land under the sticky header, search finds "gpoi" -> GPIO. Fixes by me: the "real mix-up" story (the colleague's pins) was my guess, now a hypothetical "typical mix-up"; label overlap in the power-button drawing; search snippet spacing; two "What's new" bullets. Open: owner should check the fan drawing (block diagram: 5 V pin 4, switch/transistor, GND pin 6, control pin 8); links from the console to `#log` / `#troubleshooting` / `#hdmi-4k` are not added yet. SSH to the test Pi no longer works with `ungoggled` (the owner logged in, password sync), so the package has to be installed through the web UI (System -> software update).
- [x] **2026-11-11: Part 4 findings + logging + GPIO buttons implemented in 0.3.52 (not released, not committed).** User: "OK, dann setz mal um. Vor allem auch das logging ... Umlaufprotokollierung auf 30 Tage ... Scripte wie shutdown etc. fuer GPIO momentary switches" (a friend's wish).
      Done: (G1) `session.rs`: goggles with exactly one remembered protocol are pinned (never probed with the other one, state `remembered`), unknown ones back off (Goggles 3 phase 8 s doubling to 120 s; `remembered_bonus` removed), 3 new tests; (G4) `gadget.rs` compares with `args.control_port`; (G5) `v1v2.rs` read timeout 1 s, start packet again after 10 s of silence, reconnect after 90 s (`Stall`); (G6) `video.rs` caches SPS/PPS, a decoder may start at an IDR picture (first slice) with them; (G9/logging) new `src/diag.rs` (Watcher = status changes + messages rate-limited, Heartbeat = usb_state/hdmi/clock/throttled + a summary every minute while goggles are attached, ControlLog = first DUML packet of every kind instead of the first 30), worker start/exit lines, FunctionFS disable/suspend/resume, banner at start; (FINDINGS.MD) garbled paragraph restored from git history (commit 019ba59).
      **Log persistence (overrules the image's old `Storage=volatile`)**: `scripts/prepare-pi.sh` writes `/etc/systemd/journald.conf.d/dji-hdmi.conf` (Storage=persistent, SystemMaxUse=100M, SystemMaxFileSize=8M, MaxFileSec=1day, MaxRetentionSec=30day) if it differs, restarts journald and flushes; `provision-image.sh` writes the same; a unit test keeps both scripts equal. Support bundle: 30000 lines of the service log, kernel log across boots, boot list, `goggles.txt` (memory file + UDC state), throttled/date/uptime. Measured before: ~116 service lines in 18 h idle, journal 5.4 MB volatile.
      **GPIO buttons**: `Settings.buttons` (max 4: pin, action shutdown|reboot|save_clip|toggle_recording|restart_receiver|script, hold_ms 0..10000, script file name), validated in `save()` and `migrate()`, pins distinct from LEDs/beeper/power button; `hardware.rs` `PressTracker::holding`, `ExtraButton`, `run_script` (`/var/lib/dji-hdmi/button-scripts/<name>`, must be executable, `timeout 60`, cwd = data dir); `App::run_button` in main.rs (ensure_idle for power/restart); web: HardwarePanel rows (pin, action, hold, script name), CSS, types, mock_api, wiki + USER-GUIDE + GOGGLES.md + CHANGELOG. GPIO 3 wakes a halted Pi 4 (documented). Tests: 207 passed, clippy/fmt clean.
      **Tested on the test Pi 2026-11-11** (final package `flash-images/ungoggled-0.3.52.update.tar.gz`, build 0.3.52-6b4a8f649ffb, sha256 6ee23059...d209, installed with `update-install` = the real worker, "succeeded"): journald switched itself to persistent at the update (stdout stream survived the restart inside ExecStartPre), second install does not repeat it, log survived a reboot (`--list-boots` shows both boots); simulated button presses with `pinctrl` 10/10 (tap, hold 2 s, no repeat, blip ignored, non-executable script, record toggle); throwaway-instance API test 21/21 (buttons saved/refused, support bundle with all new entries); UI checked in the mock (rows, defaults, validation).
      **Later the same day (user: "mach noch ein Auswahlfeld fuer die Goggles Wahl")**: `Settings.goggles` (auto|goggles3|goggles2|v1v2, default auto), first panel of the Settings tab (`GogglesPanel.tsx`), wiki article `goggles`; a change bumps `app.restart`; `goggles_mode(setting, system)` in main.rs: a fixed UI choice wins, `auto` leaves `UNGOGGLED_GOGGLES` from /etc/default/dji-hdmi in charge (status field `goggles_mode`). Also "Add power button (GPIO 3)" preset (shutdown, hold 2 s). Pi 4 wake from halt: bootloader `WAKE_ON_GPIO=1`, `POWER_OFF_ON_HALT=0` verified on the test Pi (firmware does the starting, no software); the colleague's "GPIO 5 & 6" is most likely physical pins 5 (GPIO3) and 6 (GND). Final package sha256 dad0d187...1919, build 0.3.52-6b2bd85e5ebd, installed on the test Pi; selector end-to-end test with a throwaway instance 11/11 (harness lesson: `pkill -f tt-data` kills its own shell and left the real service running; use a pid file).
      Still to check on hardware: Goggles tests from `review-part4-goggles-fable.md`, a real switch on a free GPIO, the real GPIO 3 wake. Known nit: `diag::Watcher` drops an identical message within 10 s, so the second "FunctionFS ready for goggles" after a quick worker restart is not logged (the worker-started line is). Not done: UI selector for the goggles protocol (env var `UNGOGGLED_GOGGLES` only), G2/G3/G7/G8.
- [ ] **Code review part 2 (video/display, system_fixes, USB protocol; Fable subagent, 2026-11-10): report `H:/Claude Code/flash-images/review-part2-core-fable.md`; no critical/high, 5 medium, 14 low; verified in the code, nothing changed yet.**
      Confirmed: V1 (video.rs ~915: a stretched live decoder that dies before its first frame for ANY reason latches `stretch_rejected` until reconnect),
      V2 (recording.rs `finalize_flight` copies the whole flight on the render thread under the recording lock; cut by TimeoutStopSec=5), V3 = R6 (unbounded
      tmpfs flight file, ignored write errors), V4 (storage.rs `reconcile` holds the `mounted` lock across `mount`/`umount` without timeout while the
      render loop takes it every tick when `record_to_usb` is on), S1 (my 4kp60 line is added without checking for a foreign `hevc_freq` /
      `over_voltage_delta`: the combination killed the firmware on 2026-11-07 -> add a guard), V7 (receive thread ends on EINTR -> dead screen while the service
      runs; one-line fix + make the end of the render thread fatal). Low: V5, V6, V8-V10, S2-S7, P1 (Pi clock sent to the air unit even if never
      synced), P2. Parts 3-4 (web UI, remaining files, build scripts) not reviewed. Fix plan = the 0.3.52 bundle together with part 1.
- [ ] **Code review part 1 (security, by a Fable subagent, 2026-11-10/11): report in `H:/Claude Code/flash-images/review-part1-security-fable.md`; I verified the findings against the code,
      nothing is changed yet.** Confirmed: S1 sshd root login with the published default password and no firewall (biggest, needs a product decision),
      S2 session cookie without `Secure` (TLS on), S3 update install/rollback need only the session (no password step-up like the terminal),
      S4 lockout per IP only + 8-char minimum only in the frontend, S5 first-visit setup claimable + non-atomic, S6 no fd limit / spectator cap,
      S7 USB mount without `-t`, R1 power-button pin that reads low powers the Pi off at every start, R2 `auth.json` written non-atomically (a bad file
      = restart loop), R3 recording download reads the whole file into RAM, R4 `wifi_busy` stuck if a scan request is cancelled, R5 blocking nmcli /
      mutex on async threads, R6 auto-record unbounded in tmpfs + ignored write errors, R7 lockout counter never decays. Low relevance: S8, S9 (no
      other users on the Pi), S10. Plan: quick safe fixes as 0.3.52 (R2, R4, R3, R5, R1, S2, S4, S5, R7), decisions needed for S1, S3, S7;
      parts 2-4 of the review (video/storage, system_fixes, web UI) not started.
- [x] **0.3.51 RELEASED 2026-11-10 (update only): 4Kp60 + fan in the managed config block (user: fan for updates too).** Nextcloud: update + .sha256 (public download verified), `latest.json` (3 notes: 4K60, fan, the 0.3.50 unplug fix; previous file `flash-images/latest.json.0.3.50-backup`); committed `6c523ee` and pushed to the fork. Still 0.3.49: the flash image and the GitHub release/README buttons. Details of the work: 2026-11-10: HDMI hint added (Automatic = 1080p, 4K60 needs HDMI0 + high-speed cable, black picture -> choose Automatic / restart), package rebuilt (sha256 c91fb3d6...e80d) and re-installed on the test Pi.**
      (1) Ship `hdmi_enable_4kp60=1`: the user re-tested it on the test Pi, a few more fps when playing and the
      Acer is now offered at 60 Hz; this overrules the 2026-11-07 "do not ship" below. Still open: the
      2026-11-07 finding that 3840x2160@60 through mpv blanked the monitor (only a reboot recovered), so ask what
      the user actually selected before deciding whether 4K60 modes stay in the web UI's mode list. Add it to the
      managed block in `src/system_fixes.rs::edit_config` (respect an existing line), reboot check in `pending()`,
      tests, release notes say "restart the Pi once". Never with `hevc_freq` >= 600 (firmware dead).
      (2) Fan: the app has no fan code; the user wants it in the boot config the raspi-config way (Performance
      Options -> P4 Fan -> GPIO 14 -> temperature), i.e. `dtoverlay=gpio-fan,gpiopin=14,temp=60000` (60 C) as the
      default. Verify the param names in `/boot/firmware/overlays/README` and any existing line in the test Pi's
      config.txt first; respect a line the user already has; confirm whether updates or only new images get it.
      Memory: ungoggled-open-work-2026-11-09.
- [ ] **Unplugging the action cam during USB playback (field report from the user's colleague,
      2026-11-08), fix written, unit-tested AND confirmed on the test Pi with the real Action 2 (2026-11-08); RELEASED as 0.3.50 (update only, no flash image) on 2026-11-08.**
      Symptom: screen falls back to the Linux console, the web UI shows no drive but still the
      player and the clip list; Stop brings the idle screen back. Cause (from reading the code, not
      reproduced): mpv (`--idle=yes`) lets go of the display when its file can no longer be read,
      while the render loop keeps standing aside for `current_usb`. Fix: `video.rs::usb_end` ends the
      playback like Stop when the clip file is gone, mpv exits, reaches EOF or reports `idle-active`
      twice in a row (after a 3 s start-up grace); `UsbPlayer::drop` waits at most 3 s for mpv;
      `storage.rs`: `umount -l` fallback for a drive that is really gone (a busy plain umount left a
      dead mount behind), `resolve` refuses vanished files; `UsbVideos.tsx` hides the clip list while
      no drive is detected. Also fixed: the `embedded_module_matches_the_pinned_kernel_and_build_info`
      test had been failing since the patch text was edited (BUILD-INFO `patch_sha256` updated; code
      of the patch unchanged). Real test 2026-11-08 (test binary run via a /run drop-in, then removed):
      cable pulled while a 4096x3072 clip played -> status message "USB drive removed: video
      playback stopped", mpv gone, display back on the service's own XR24 framebuffer (fallback),
      no stale mount. Not measured: the exact delay (the logger had run out), by design <= ~2 s
      (drive poll) + 100 ms. Release 0.3.50: update installed on the test Pi through the real installer
      (state "succeeded", no reboot needed), then `ungoggled-0.3.50.update.tar.gz` (+ `.sha256`) and
      `latest.json` on the Nextcloud share (public download verified against the checksum; the
      previous file is `H:/Claude Code/flash-images/latest.json.0.3.49-backup`). Still on 0.3.49:
      the flash image and the GitHub release/README download buttons (the fork has no 0.3.50 commit yet).
- [x] **Fork published (2026-11-08):** `https://github.com/Joker133782/ungoggled-by-Joker133782`
      (`master` = this working tree at 0.3.49 plus the upstream MIT commit; upstream
      `TimoLehnertz/ungoggled` has been MIT since 2026-11-02). Commits carry the GitHub noreply
      identity only (owner's request: no private info on GitHub); the pre-rewrite history is in
      `H:/Claude Code/backup-ungoggled-before-publishing.bundle` (local, still has the old identity,
      never push it). `CLAUDE.md` is excluded from the repository. Open: (a) the GitHub release
      checker (`web/src/releases.ts`) stays off until the owner wants releases on the fork (then
      point it at the FORK, never at TimoLehnertz/ungoggled); (b) the kernel-driver report is DONE as an
      issue, no pull request (a PR needs a real name and e-mail in `Signed-off-by`): raspberrypi/linux#7743,
      posted 2026-11-08 as Joker133782, patch as plain text, no name/e-mail. Answered 2026-11-09 by 6by9 and jc-kynesim (Raspberry Pi): the logging bug (coeff_size printed as
      pu_size) is confirmed and will be fixed; for the memory-region property they are lukewarm (no obligation
      to support it, level 5.2 is beyond the block's 5.1 / 4k60 spec, would not be on by default, but the use
      case is real; 6by9 notes devm_of_reserved_mem_device_init would simplify the patch). Replied 2026-11-09 as Joker133782 (issuecomment-6753177021: the level-5.2 use case, opt-in
      property, why a child device instead of devm_of_reserved_mem_device_init, offer to test the devm_ variant
      on a Pi 4 if they prefer it; text in H:/Claude Code/flash-images/kernel-issue-reply-draft.md). Follow-up 2026-11-10: 6by9 (after talking to jc-kynesim) says the idea has merit but he does not want to
      complicate upstreaming the driver (device-tree binding review is open); left open, to be revisited after the driver is
      upstream. Logging bug fixed in their #7720. He also notes 3840x2880 is really level 6 (frame 24 % over the 5.2 luma
      limit); hardware probably decodes up to 4096x4096. Nothing to do or answer; our patch stays our own shipped module.
      **GitHub release v0.3.49 is published in the fork** (tag on `270829f`, marked Latest): the flash
      image, the update package and both `.sha256` files; GitHub's asset digests match the build's
      checksums. The in-app update check still reads the Nextcloud share, not GitHub.
- [x] **Release 0.3.49 (2026-11-08): update AND flash image published** (flash image added after the user's Test B passed; the user's colleague also asked to install it in the field).
      After the user's Test A (0.3.40 image, in-place update through the web UI) passed:
      `ungoggled-0.3.49.update.tar.gz` (+ `.sha256`) and `latest.json` (7 short notes) are on the
      Nextcloud share, public download verified against the checksum; the previous `latest.json`
      is saved as `H:/Claude Code/flash-images/latest.json.0.3.42-backup`. The CHANGELOG folds the
      never-published 0.3.43 - 0.3.49 into one `## 0.3.49`. Done after Test B
      (`ABNAHME-0.3.49.md`) passed: `ungoggled-0.3.49-pi4-arm64.img.xz` + `.sha256` are on the share
      (checksum verified inside the container; 0.3.40 + 0.3.49 = 2 images, nothing deleted). Not covered by
      any test: the phone's download from the share (`nextcloud.ts`, unchanged since 0.3.28;
      Test A used the manual upload), Walksnail / N3 / Goggles 2 (the user waits for feedback).
- [ ] **Stretch to fill the screen (user asked 2026-11-07), in the 0.3.48 test candidate,
      installed on the test Pi.** `stretch_live` / `stretch_usb` settings (off by default),
      two switches in Settings -> HDMI; details and the end-to-end test with a stand-in for the
      goggles in FINDINGS.MD "Stretching a picture to the screen". To do: the user tries the
      USB switch in the web UI with a real clip, and the live one with real goggles in a 4:3
      mode (which picture sizes does the goggles' 4:3 mode send? the display chip refuses
      1440x1080 and 960x720 stretches, then the picture stays unstretched after one lost
      keyframe interval). The stand-in feed (`scripts/fake-goggles-feed.py FILE SECONDS [REPEAT]`:
      datagrams to `/run/dji-hdmi/video.sock`) tests the live path without goggles.
- [x] **Monitor power-cycle leaves the Linux console on screen: FIXED in 0.3.49 (test
      candidate, installed on the test Pi; the user still has to power-cycle the real Acer once).**
      Observed 2026-11-07 with the Acer: after a hard power cycle the screen showed the kernel
      console (fbcon, plane format RG16 at 1920x1080@60.00) while the app reported
      `hdmi: fallback`; only `systemctl restart dji-hdmi` brought the picture back. Cause: a
      hotplug gives the output to fbcon, and the display's background probe only watched the
      connector state every 3 s, so a power cycle shorter than that (or a VT switch) was never
      seen. Reproduced on demand on the 0.3.48 Pi with an emulated cycle (`echo off >
      /sys/class/drm/card1-HDMI-A-1/status; sleep 0.2; echo detect > ...`): RG16 stays forever,
      2 of 2 runs (an `off` of 2 s is caught by the probe about every other time, which is why the
      problem looked intermittent). Fix in `src/display.rs`: the probe thread also compares the
      framebuffer on the CRTC (`get_crtc`) with the app's own and asks for a rebuild when it is not
      ours. Result on 0.3.49: the app's picture (XR24) is back 2-4 s after the console took over,
      3 of 3 runs. During a live (stand-in goggles) stream the same cycle did not take the screen
      at all and the decoder was not restarted (`decoder_starts` stayed 1), so no false alarms
      there. Not covered: the first observation of the same day, the Acer staying black after the
      service restart with a healthy Pi output (no fbcon involved: the monitor did not resync;
      unexplained, nothing the app can see).
- [x] **`hdmi_enable_4kp60=1` tested 2026-11-07: do not ship (decided by the
      measurements).** Core runs at 550 MHz, 1080p playback gains ~5 %, but 4K@60
      output through mpv does not work here (0.048x, `flip_done timed out`, the
      monitor drops the signal, only a reboot recovers) and the option would add
      4K60 entries to the mode list. Details: FINDINGS.MD, "USB vs. SD card,
      output size and 4K60". Open only if someone wants to find out whether the
      4K60 failure is the cable or the HVS bandwidth (blanks the monitor).
- [x] **USB playback speed/fps readout (user asked 2026-11-07), in the 0.3.44
      candidate, VERIFIED on the test Pi 2026-11-07** (flash image 0.3.43 ->
      web update to 0.3.44, real clips): the user read 34-36 fps for
      `DJI_0023.MP4` and 45-47 fps for the O4 clip in the player; a logger
      compared the app's `usb_fps` once a second with the page flips counted
      from `drm:drm_vblank_event_delivered`: DJI 35.5 vs 34.6 (speed 0.592),
      O4 47.7 vs 46.0 (speed 0.797 on this passage); the logger reads 1-1.7
      fps lower because it reads and clears the trace buffer once a second
      and so drops an event or two (the lossless bench of the same day matched
      exactly, 35.5 vs 35.5). Pause shows 0 and "Paused", seeking and resuming
      show "Measuring speed..." for ~4 s. mpv is a
      separate process and fed neither `input_fps` nor `output_fps`. Now
      `src/playspeed.rs` turns mpv's IPC `time-pos` (polled once a second from
      the video worker while a clip plays) into speed (clip seconds per wall
      second, 4 s window, resets on pause/seek) and fps = speed x
      `container-fps`; published as `usb_speed`/`usb_fps`/`usb_clip_fps` in
      `/api/status` plus `output_fps`, shown in the USB player panel, the HDMI
      card and the "TV" graph line. The method matched the vblank flip count
      (0.592 x 59.94 = 35.5). To verify: play a clip on the test Pi with
      `/root/statuslog.sh`-style logging of loopback `/api/status` and compare
      with the vblank tracepoint (`/root/bench.py`). Idea left out: an on-screen
      overlay via mpv `show-text`.
- [ ] **Deliver every system-level fix through the update, then release
      update AND flash build together (user decision 2026-11-06).** The user
      wants no flash image and no release until all of this is done and
      tested; once it is, releasing the update (Nextcloud + `latest.json` +
      wiki Updates page) and a flash build is pre-approved.
      **Implemented 2026-11-06, not released:** `src/system_fixes.rs`
      (`ungoggled system-fixes apply|status|remove-module|enable-module`), run
      by `scripts/prepare-pi.sh` before every service start (failures only
      logged) and by `scripts/build-image.sh` inside the image (`--offline
      --strict`); module + overlay embedded from `assets/kernel/`, built by
      `scripts/build-hevc-module.sh` (bit-identical rebuild verified). Covers
      `core_freq_min=500`, `cma-size=0x28000000` if missing/smaller, the
      patched module + `ungoggled-hevc-cma` overlay (only on kernel
      6.18.50+rpt-rpi-v8 / package 1:6.18.50-1+rpt1), `apt-mark hold` of all
      installed `linux-image-*`/`linux-headers-*`/`linux-kbuild-*`, and a
      "Restart the Pi once" notice (`/api/status` `reboot_required`).
      **Verified on the test Pi** (reset by hand to the 0.3.42 image state, not
      reflashed): installed 0.3.90 test package with the installed 0.3.42
      updater's own CLI path (`check-update` + `update-install`, which runs the
      same update-worker transaction as the web UI); after the reboot the
      patched module is loaded, `hevc_cma@0` (128 MiB) is reserved and the
      decoder's internal buffers come from it (CMA tracepoints, GStreamer
      `v4l2slh265dec`, ~49 fps); `core_freq_min=500` active; second update
      (0.3.91) changed nothing; switch back to the factory 0.3.42 kept the
      fixes and the service started; kernel-mismatch, load-failure and
      missing-overlay-file paths tested on the Pi. Details: FINDINGS.MD,
      "System fixes through the update".
      **Still open:** (1) the browser upload + Install buttons were not
      exercised (no control password available to the agent) - the user does
      that step in the acceptance test; (2) a real `build-image.sh` run (the
      image-side command was only run against a temporary root on the Pi);
      (3) USB playback with the action camera plugged in again (only the
      14.9 MB `test.mp4` 3840x2880 Main10 clip was decoded). (4) CLOSED
      2026-11-07: 1 GiB boards are out of scope - the user set 2 GB RAM as the
      minimum for ungoggled, so 640 MiB CMA + 128 MiB decoder pool applies
      everywhere, no RAM gating needed.
      **Release candidate 0.3.43 built 2026-11-07 (update package + flash
      image, local only, NOT uploaded; acceptance Test A passed on the test
      Pi) - SUPERSEDED the same day by 0.3.44, which adds the USB playback
      speed readout; 0.3.43 was never released, its CHANGELOG section is now
      "0.3.44". The 0.3.43 image/update in `H:/Claude Code/flash-images/` are
      obsolete; the flash image still has to be rebuilt at the final
      version.** Still to do at release (needs
      the user's go after the hardware acceptance test): Nextcloud upload of
      the update + `latest.json`, the flash image on Nextcloud (keep 2 newest),
      the wiki Updates page.
- [x] **Settings migration dropped `hdmi_connector` on every update (found
      2026-11-06, fixed 2026-11-07).** `settings::migrate` had no arm for
      `hdmi_connector`, so every update (and every switch back) reset a
      manually chosen HDMI output to automatic. Added the arm plus
      `migration_keeps_every_field_of_the_current_schema`, which fails for any
      field that is added later without a migrate arm. The migration that runs
      is the target release's, so the fix is effective on the first hop.
- [x] **AP channel: decided 2026-11-06 - stay on channel 36 (5 GHz, no DFS).**
      Channel 100 (DFS) does not work on the Pi 4's onboard chip: a flash image
      built with `channel=100` booted without any Wi-Fi, and the user's A/B on
      the device showed `nmcli connection up` activating on channel 36 but
      failing on 100 ("802.1X supplicant took too long to authenticate").
      `scripts/provision-image.sh` is back on `channel=36` (comment rewritten).
      Known cost, accepted: 5170-5190 MHz overlaps the O4 Air Unit Pro's
      5.17-5.25 GHz band. No flash image with channel 100 may be published
      again (the 0.3.42 one was removed from Nextcloud). Making channel/band a
      web-UI setting stays an option, not planned.
- [ ] **Display hand-overs take 10 s per step; suspect the second HDMI output.**
      On the two-monitor test Pi (2026-11-06): Ctrl+Alt+F2 does reach a login on
      tty2 but only after "an eternity" with the idle picture still showing;
      the console also shows for 20-30 s after a video stops; unplugging the
      active monitor leaves the other without a signal for a long time. Every
      `[drm] *ERROR* ... flip_done timed out` captured so far names CRTC 114 =
      `pixelvalve-4` (and, in the photo of 2026-11-02, CONNECTOR 44 = HDMI-A-2,
      the Acer). The kernel waits 10 s per timed-out commit, so 20-30 s is 2-3
      of them. This is a lead from error text, not proven. Cheapest decisive
      test: attach only ONE monitor (try the BenQ on HDMI-A-1 first) and repeat
      VT switch, video stop and idle-picture return. The field setup has one
      display, but the app must not stall 10 s per commit either way; check the
      connector auto-select (`hdmi_connector: null`) and what the app commits on
      a connected-but-unused output.
      **Read on the fresh 0.3.42 image, 2026-11-06 (Acer ACR XV275K V on HDMI-A-1,
      BenQ GL2460 on HDMI-A-2, identified from the EDIDs):** the app (PID holding
      `/dev/dri/card1` as DRM master) drives only HDMI-A-1 = `pixelvalve-2` at
      1920x1080@59.94 progressive with its own 1080p framebuffer; `hdmi0_regs`
      `HDMI_VID_CTL` = `0xc0000000` (healthy, no underflow). `pixelvalve-4`
      (HDMI-A-2) is active at **1920x1080i**, which the app never selects
      (`select_mode` skips interlaced in auto mode), so that is the kernel's
      fbdev/fbcon clone. `src/display.rs` in auto mode takes the first connected
      HDMI connector and never touches the other; the kernel command line
      `video=HDMI-A-1:1920x1080@60D` forces HDMI-A-1 "connected" permanently, so
      "auto" can never move away from it (this would explain a dead second
      output and no switch-over when the active monitor is unplugged). Ctrl+Alt+F2
      can then only show the console on the output the app does NOT own.
      **Resolved later the same day:** the Acer itself had hung (it needed a hard
      power cycle); afterwards both monitors worked, the web UI's HDMI output
      switch worked, and the console login shows on whichever output the app is
      not driving (user: that is fine). So "the Acer never shows a picture" was
      not a Pi bug. Still worth re-testing now that the Acer is healthy: the 10 s
      flip_done delays, Ctrl+Alt+F2 latency and the console-after-video-stop
      delay - the misbehaving Acer (on HDMI-A-2 = pixelvalve-4 in the earlier
      captures) may well have caused them.
      **Re-measured 2026-11-06, both monitors healthy (FINDINGS.MD, "Re-measured on
      the clean 0.3.42 image"):** no `flip_done` timeout in any boot; `chvt 2`
      switches in 80 ms; video stop -> idle picture back in 0.6-0.8 s (not
      20-30 s); emulated unplug leaves the other output alone. The 10 s lead is
      dropped. Still open, needs the user's decision: after a VT switch and back,
      or a re-plug of the app's monitor, the console stays on the app's output
      because the app never notices it lost the screen; and with a connector
      chosen in the settings it never moves. Decide how multi-output should
      behave (field setup: one display) before changing `src/display.rs`.
      Physical unplug/replug not tested yet.
      **Decision 2026-11-06 (user): not now.** The field setup normally has ONE
      monitor, so multi-output behaviour (re-claiming the screen after a VT
      switch or re-plug, moving to another connector) is parked here on purpose
      - do not start on `src/display.rs` for this unless the user asks.
- [ ] Confirm on hardware that the root partition expanded to fill the card on
      first boot. Everything else about the shortened image is verified: it
      boots, the receiver links to the goggles at boot with the cable already
      attached, and an update installs on top of it. One command answers it:
      `ssh root@192.168.50.1 'df -h /'` — expect ~6.7 G on the 8 GB card, not
      2.5 G.
- [ ] Find why an update install leaves the gadget silent while the goggles
      still report the port configured. A plain `systemctl restart` recovers,
      so the difference is the updater's stop/swap/start gap in
      `src/update/install.rs`. The bounded recovery in `src/functionfs.rs`
      repairs the symptom; the cause is still open.
- [ ] Spectator stream "Event" (low) quality mode destabilizes the main HDMI
      display — confirmed on real hardware 2026-10-31, repeated picture
      dropouts on the pilot's own goggles view the whole time it ran. Cause:
      it runs its own independent software H.264 decode (`avdec_h264`) plus
      re-encode (`x264enc`) concurrently with the live display pipeline,
      which already owns the Pi's one hardware decoder — real CPU contention
      on a Pi 4, not a transient glitch (`src/stream.rs::spawn`, `Quality::
      Event`). "High" quality (just muxes the goggles' own H.264 straight
      through, see 0.3.41) is now fixed and doesn't have this problem, since
      it never decodes anything — likely good enough for a handful of
      spectators and should be preferred until Event mode is reworked. Best
      fix found so far: the live pipeline in `src/video.rs` already has a
      `tee name=decoded` branch feeding the web preview's JPEG encoder —
      Event mode should tap that already-decoded output instead of running
      its own independent `avdec_h264`, so it only needs its own `x264enc`
      stage, not a second decode.
- [ ] Still unconfirmed whether a colleague's reported brief black-screen
      flicker "every few seconds" during a live flight (DJI Goggles 3, direct
      USB-C) was the Event-mode CPU contention above (i.e. whether a spectator
      was on Low Quality at the time) -- he couldn't say. Ask again next time
      it's reproducible. A second candidate: the VideoCore/HVS core clock
      dropping under `ondemand` and the HDMI FIFO underflowing (FINDINGS.MD,
      "USB playback of DJI Action 2 4K60 clips"); `core_freq_min=500` fixed it
      on the test Pi for oversized clips that are downscaled on the display
      plane. A live 1080p goggles feed isn't downscaled, so whether it is
      affected is unknown, but the fix is cheap to try. The recurring `[drm]
      *ERROR* flip_done timed out` seen at boot / late in a long session is
      still unexplained (possibly the same clock problem, unverified).
- [ ] **USB Action 2 4K60 playback -- open work** (see FINDINGS.MD, "USB
      playback of DJI Action 2 4K60 clips", for what is established and what
      was retracted). In order:
      (1) DONE 2026-11-06: `UsbPlayer::drop` is a plain `kill()` + `wait()`
      again. The 20-30 s console was not reproducible with either version on
      the clean image (0.65-0.83 s vs. 0.57-0.59 s until the idle picture is
      back); the user should confirm by eye.
      (2) DONE for new images: `core_freq_min=500` in `scripts/build-image.sh`
      (measured: HVS underflows only at core <= 324 MHz, none at 500).
      Existing installs keep their `config.txt` unless the update changes it:
      DECIDED 2026-11-06, the update must deliver it (see the "Deliver every
      system-level fix through the update" item below). The vc4 load model ignores the source size of a
      downscaled plane (possible upstream report; needs a reproduction
      without this app).
      (3) ANSWERED (refined 2026-11-07 evening, see the next item): 1.00x is
      impossible for `DJI_0023.MP4` at the stock 500 MHz HEVC clock (mpv max
      ~53 fps even with nothing on screen, all frames are references); mpv's 34 vs.
      GStreamer's 51 is the cost of scanning the full 4096x3072 frame out.
      `--framedrop=no --audio=no` stay (every alternative measured worse).
      (4) ANSWERED, no change: the app already uses 1080p for idle and
      playback; 4K30 gives exactly 0.50x for 60 fps clips.
      (5) ANSWERED: 0.3.40 has the same mpv flags plus audio and plays the clip
      at 0.53x on this Pi.
- [ ] **Action 2 4K60 playback, 2026-11-07 evening (FINDINGS.MD, "Display
      bandwidth and HEVC clock, 2026-11-07").** In the repo, not released:
      `--drm-draw-surface-size=960x540` in `spawn_usb_player` (kept: DJI clip
      0.59x -> ~0.65x, O4 45 -> 49-51 fps, no colour side effect). The line-skip
      zoom (`src/lineskip.rs`, 0.59x -> 0.755x) was built, installed as the
      0.3.45 test candidate and DROPPED the same evening: it falsifies the
      colours of both Action 2 clips (user A/B, FINDINGS.MD "Line-skip zoom
      falsifies the colours"); removed from the code. Candidate now 0.3.46
      (CHANGELOG folded, 0.3.45 never released), installed on the test Pi through
      the real updater and MEASURED 2026-11-07 (logger + the user's eyes, colours
      right again): 4:3 30 fps Action 2 clip `DJI_0010.MP4` plays at **1.000x /
      30.0 fps** (after the first ~3 s while the speed window fills), the 60 fps
      clip `DJI_0023.MP4` at 0.64x / 38.2 fps, the O4 clip at 0.83x / 49.7 fps;
      throttled 0x0, no kernel errors. Real time for 4:3 60 fps stays out of
      reach at stock clocks. Logger output: `H:/Claude Code/flash-images/
      verify-0.3.46-pi.txt`. Old items, now mostly obsolete:
      (1) End-to-end test of the new binary through the app (play from the web
      UI, check `video-zoom` on mpv's socket, plane `crtc-pos` 1358x1018 in
      debugfs, the speed readout) - not done tonight because it needs the
      service restarted (the Acer needed a power cycle after the last one) and
      the user's login.
      (2) The user should look at the picture: 5.6 % smaller with black borders,
      vertical filtering from every 2nd line.
      (3) Real time needs ~650-700 MHz HEVC with core >= 500 MHz. Every tried
      combination that would give that killed the VideoCore firmware at boot
      (`hdmi_enable_4kp60=1` + `hevc_freq` 600/650/700: mailbox -110, no
      HDMI, 4-30 min reboot); `hevc_freq=600` alone drops the core to 300 MHz
      (black-screen risk). Boot-level tests are STOPPED by decision; only with
      the user's explicit go, and with a way to fix the SD card on a PC at
      hand. Untested candidates: `hevc_freq=650` + `core_freq=650` (+ v3d),
      `gpu_freq=600`. Not for shipping either way (unknown boards, cooling,
      no SSH in the field).
      (4) Not recommended, not built: real time by dropping the tail of each
      0.5 s GOP (IDR every 30 frames) - 2 Hz stutter.
- [ ] Kernel driver `rpi-hevc-dec` (separate from all of the above; none of
      the playback problems depend on it): patch for `memory-region` support
      plus the wrong-size log message is in `docs/upstream-patches/
      rpi-hevc-memory-region/`. Needs the user's real name and e-mail for the
      `Signed-off-by` (placeholders now), a fresh `git apply --check` against
      the current `rpi-6.18.y` (the driver is being replaced by the upstream
      submission, raspberrypi/linux PR #6666), and an A/B on hardware that is
      kept apart from the playback measurements so the two can't confound
      each other. The bug report is written last, together with the user.
      **Re-validated 2026-11-06** (FINDINGS.MD): applies to the running
      kernel's source (`f2bc178ed43d`, offset -43) and cleanly to the
      `rpi-6.18.y` head (`a1a601d48579`); A/B on fresh boots confirms the bug,
      the fix and the corrected log size. PR #6666 is the already merged
      backport of this driver into `rpi-6.12.y`, not a pending replacement.
      Remaining: name/e-mail, then report + PR with the user.
      **Decision 2026-11-06 (user):** the patched module ships in the update
      and stays on the pinned kernel until an official kernel contains the
      fix (see the delivery item at the top). Bug report/PR still last, with
      the user.
- [ ] `scripts/install-release.sh` still health-checks `http://127.0.0.1/api/settings`,
      which is behind the login since 0.3.41. It only "passes" because plain
      HTTP answers with a 308 redirect that `curl -f` (no `-L`) accepts, so the
      check is vacuous. The Rust updater (`update/install.rs::healthy`) uses
      `/api/status` with `-L -k`, which is the right pattern. Not changed yet
      because it needs a test on real hardware.
- [ ] A colleague connected a **DJI Goggles 2** (not 3) to the Pi's USB-C
      port via an OTG adapter and it was not recognized/no video. He sent a
      log bundle but it only has config/settings dumps, no journal/dmesg —
      not enough to diagnose. Two separate things to do: (1) get a real
      journalctl/dmesg capture from him for this attempt, (2) check whether
      Goggles 2 (a different, older device than the G3/O3/O4 this project
      targets) going through an OTG adapter is expected to work at all —
      this project's whole USB gadget/FunctionFS setup was built and tested
      against G3, not G2; G2 may use a different pairing/negotiation
      sequence entirely (see FINDINGS.MD's G2/O3 Pi endpoint reference,
      which explicitly excludes G3/O4 — the reverse compatibility question,
      whether *this* G3-built implementation handles G2, was never tested --
      NOTE: the tree already has Goggles 2 handling (`src/g2.rs`,
      `g2_sequence.rs`, `v1v2.rs`), so check what those cover before assuming
      it is untested).

- [ ] **Fable code review part 3 (web UI, remaining Rust modules, build scripts), done 2026-11-11:**
      report `H:\Claude Code\flash-images\review-part3-web-scripts-fable.md`. 0 critical/high, 6 medium, 12 low.
      Verified by me in the code: W1 (`web/public/watch.html:21` loads mpegts.js from cdn.jsdelivr.net, no SRI,
      so the spectator page cannot work on the Pi's own Wi-Fi without internet and a third-party script runs in
      the control origin), W2 (`AbortSignal.any` in `api.ts:29` / `Graphs.tsx:158`, `.at(-1)`: old iPhones show
      "Disconnected"), W3 (settings fetched once at mount `App.tsx:141`, `save` posts the whole object: a second
      device reverts changes), W4 (`Recording.tsx:385` deletes without ConfirmDialog), M1 (`stream.rs:66-73`
      blocking `write_all` under a mutex from the video loop, "Low quality" transcode can stall live HDMI),
      M3 (`settings.rs:120-147` rewrites 14.6 MiB of built-in videos with fsync on every start; restart loops wear
      the SD card), B2 (= the item above: `install-release.sh:44` `-fsS .../api/settings`). Not yet verified by
      me: W5-W8, M2, M4, B1, B3-B6. Nothing changed in the code yet; waits for the user's decision (0.3.52 bundle
      together with parts 1 + 2).

- [ ] **0.3.52 built and tested on the test Pi, NOT yet released (waits for the user's go)** (2026-11-11). Package
      `H:/Claude Code/flash-images/ungoggled-0.3.52.update.tar.gz`; installed through the real installer, then a
      throwaway second server on the Pi's loopback (24 checks: password rules, Secure cookie, 0600 files, root/SSH
      password follows the control password and change, download + Range, symlink refused, lockout) and the
      `reset-password` boot-partition file (owner's auth.json backed up and restored). Decisions of the user
      (2026-11-11): W1 ok; SSH = root = control password, default `ungoggled` everywhere, follows every change
      (first boot or later); USB only vfat/exfat/ntfs3. Contents: parts 1-3 fixes R1 R2 R3 R4 R5 (client_status +
      try_lock) R6 (parts) R7 S2 S4 S5 S7 S9(partly: 0600) V1 V2 V3 V4 V5 V7 V8 V10 4kp60 guard W1-W5 W7 W8 M1 M2 M3 B2
      B4. NOT done: S3 (password step-up for update install), S6 (spectator cap, only LimitNOFILE), S8, V6, V9, M4, B1,
      B3, B5, B6, R8-R11, a change-password screen in the web UI (only the API endpoint exists).
      Found while testing: `scripts/check-image.sh` logs in to a throwaway server inside the image being built, which
      would now set the IMAGE'S root password to the throwaway one -> fixed with `UNGOGGLED_KEEP_SYSTEM_PASSWORD`
      (set in check-image.sh, plus a re-check of the root password at the end). The first real login on the test Pi
      after this update changes root's SSH password from `ungoggled` to the owner's web password (scratchpad
      `pi.py` etc. then stop working; `reset-password` file restores `ungoggled`).
      Decisions 2026-11-11 (later): NO password step-up for update install / downgrade (S3 declined by the user), NO limit on spectator
      numbers (S6 declined; only LimitNOFILE=8192 stays), YES a change-password panel in the web UI (building it).

- [ ] **Fable review part 4 (goggles implementations), done 2026-11-11:** report `H:/Claude Code/flash-images/review-part4-goggles-fable.md`.
      Verdicts: Goggles 3 + O3/O4 via FunctionFS = works (hardware-verified; AOA, descriptors, configfs order, DUML/CRC checked);
      legacy gadgetfs transport = known-bad after goggles reboot, not reachable from the service; Goggles 2 = unverified (g2_sequence.rs
      all 241 entries well-formed, but a blind replay of a third party's session incl. stale timestamps/identifiers; never seen on
      hardware); V1/V2 = should work to first picture, no hardware evidence, no stall recovery. Open decisions/ideas (nothing changed yet):
      G1 `auto` mode fires the unverified Goggles 2 burst at Goggles 3 whenever no video arrives within 8-13 s (drone not yet powered):
      verified in code (session.rs ~234-257, test `late_drone_power_on_is_found_by_alternating`); proposed fix: never alternate away from a
      remembered protocol (goggles whose AOA strings were already seen with video), or default `UNGOGGLED_GOGGLES=goggles3`; needs a real
      Goggles 3 test (questions list in the report). G4 control_port compare in gadget.rs:316; G5/G6 V1/V2 stall detection + cache SPS/PPS;
      G9 add goggles_protocol fields to the hardware test logger; FINDINGS.MD line 78 is garbled. Goggles 2 colleague debugging: check
      `cat /sys/class/udc/*/state` and the journal grep from section 2 of the report.
