# Prepared upstream patch: `rpi-hevc-dec` memory-region support

**Status: ready to submit, not yet sent.** Blocked only on the submitter's
GitHub account (pending an email change) — this folder holds everything
needed to open the PR the moment that's unblocked. Since 2026-11-06 (user
decision) this fork also ships the patched module itself, built from this
patch for the one pinned kernel (`scripts/build-hevc-module.sh`,
`assets/kernel/`), installed through the normal update and the image build by
`src/system_fixes.rs`, with the kernel packages held until an official kernel
carries the fix. The CMA pool size stays in place as well.

## What this is

`0001-rpi-hevc-dec-memory-region.patch` is a `git format-patch`-formatted
commit against [`raspberrypi/linux`](https://github.com/raspberrypi/linux),
branch `rpi-6.18.y` (confirmed the repo's current default branch as of
2026-10-31). It fixes a real bug in the stock BCM2711 HEVC decoder driver:
a `memory-region` device-tree property on the decoder's node is silently
ignored (the driver never calls `of_reserved_mem_device_init*()`), so
there's no way to give the decoder's own internal working buffers
(~45 MiB: PU/coeff/cmd-FIFO/colmv) a dedicated reserved-memory pool
separate from the shared default CMA pool that GStreamer's
`v4l2slh265dec` can fill most of on footage that undersells its real
resolution (exactly what DJI Osmo Nano 3840x2880 10-bit footage does —
see `FINDINGS.MD`, "HEVC (DJI Osmo Nano) USB-playback decode").

Built, loaded, and verified on this project's own Raspberry Pi 4
hardware: decode works, bit-exact output against software decode,
clean module load/unload, no behaviour change when no `memory-region` is
present. A later, separate investigation (also documented in
`FINDINGS.MD`) compared it directly against the simpler config-only
workaround this fork actually ships and found the extra 128 MiB
`memory-region` region gives only marginal headroom over just raising the
shared pool — not enough to justify maintaining an out-of-tree kernel
module for one specific kernel build. That was the reason this fork did not
run the patch day to day; the user decided otherwise on 2026-11-06 (see
above). It's still a genuine upstream fix worth sending:
the driver bug is real regardless of which workaround this fork prefers.

**Re-validated 2026-11-06** (details in `FINDINGS.MD`, "Re-measured on the
clean 0.3.42 image"): the running 6.18.50+rpt module matches `rpi-6.18.y`
commit `f2bc178ed43d`; the patch applies there (offset -43) and cleanly to the
branch head `a1a601d48579`, builds without warnings, and the fresh-boot A/B
(stock vs. patched, with and without a 128 MiB `memory-region` overlay, CMA
tracepoints) plus a forced Coeff allocation failure (stock logs 0x400000 for
a 0x1000000 request, patched logs 0x1000000) confirm it. raspberrypi/linux
PR #6666 is the already merged backport of this same driver into
`rpi-6.12.y`; the patch targets that driver. Name/e-mail are still
placeholders.

## Files

- `0001-rpi-hevc-dec-memory-region.patch` — the patch itself. Applies
  cleanly to `rpi-6.18.y` as of 2026-10-31 (verified via `git apply
  --check` against a fresh clone). Has a placeholder `Signed-off-by`
  line — see below.
- `PR-DESCRIPTION.md` — ready to paste as the GitHub PR's title and body.

## To actually submit it, once the GitHub account is ready

1. Replace the two `PLACEHOLDER_NAME <PLACEHOLDER_EMAIL>` occurrences in
   the patch (the `From:` header and the `Signed-off-by:` trailer — they
   must match) with the real name and email to attribute the contribution
   to. The Linux kernel requires a `Signed-off-by` as a Developer's
   Certificate of Origin; don't submit without it.
2. Fork `raspberrypi/linux` on GitHub, check out `rpi-6.18.y`.
3. `git am docs/upstream-patches/rpi-hevc-memory-region/0001-rpi-hevc-dec-memory-region.patch`
   (re-verify it still applies cleanly first — re-pull `rpi-6.18.y`, since
   time will have passed; `git apply --check` the patch against a fresh
   clone before `am`-ing, and re-test on real hardware if it needed
   adjusting).
4. Push the branch, open the PR against `raspberrypi/linux:rpi-6.18.y`
   using `PR-DESCRIPTION.md`'s title and body.
5. This is an independent contribution to a third-party project, unrelated
   to this fork's own (unresolved) license situation — see `CLAUDE.md`'s
   license note. No conflict either way.
