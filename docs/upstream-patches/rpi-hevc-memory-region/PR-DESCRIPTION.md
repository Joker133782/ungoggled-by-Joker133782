# PR title

media: rpi-hevc-dec: support a dedicated memory-region for internal buffers

# Target branch

`rpi-6.18.y` (the repo's current default branch).

# PR description (paste into GitHub)

## Problem

`rpi-hevc-dec` (the BCM2711 stateless HEVC decoder) allocates ~45 MiB of
private working buffers (PU/coeff/cmd-FIFO/colmv) from the default CMA pool
at `STREAMON` time. On BCM2711 that pool must stay below 1 GiB because the
HVS display controller can only scan out from the first 1 GiB of RAM.

A V4L2 stateless-API client (e.g. GStreamer's `v4l2slh265dec`) pre-sizes its
own capture-buffer pool off the stream's *declared* level, defensively,
before the decoder ever starts. On footage whose level undersells its real
resolution — e.g. 3840x2880 10-bit HEVC declaring level 5.2 — that pre-sized
pool alone can reach ~440 MiB against the default 512 MiB CMA pool, so
`STREAMON` reliably fails with `-ENOMEM` once userspace already holds its
buffers:

```
cma: __cma_alloc: linux,cma: alloc failed, req-size: 3072 pages, ret: -16
rpi-hevc-dec feb00000.codec: Failed to alloc 0x300000 Coeff0 buffer
rpi-hevc-dec feb00000.codec: hevc_d_start_streaming: qtype=10: FAIL
```
```
ERROR: from element .../v4l2slh265dec0: Could not enable the decoder driver.
VIDIOC_STREAMON(SINK) failed: Cannot allocate memory
```

A `memory-region` property on the decoder's device-tree node is the
standard way to give a device its own reserved-memory pool instead of
competing with everything else for the default one — but `hevc_d_probe()`
never calls `of_reserved_mem_device_init*()`, so the property is silently
ignored today. An overlay that defines one has no effect.

## Fix

Attach the `memory-region` (if present) to a dedicated child "memdev"
device, the same pattern already used by `s5p_mfc_alloc_memdev()`, and use
it in preference to the default pool **only** for the decoder's own
internal working buffers. `vb2` frame/bitstream buffers are left on the
platform device's default CMA, since those may need to be reachable by the
display (tighter addressing limits than the decoder, which has a 36-bit DMA
mask). Each internal buffer records which device it was actually allocated
from, so a too-small or absent region falls back per-buffer to the default
pool, and frees always go to the matching pool.

Also fixes the `Coeff%d` allocation-failure log line, which printed
`pu_size` (a quarter of the size actually requested) instead of
`coeff_size`.

No behaviour change when no `memory-region` is present — existing setups
are unaffected.

## Testing

Raspberry Pi 4 (BCM2711), `rpi-6.18.y`, reproduced with a real 3840x2880
10-bit HEVC clip (level 5.2) that reliably fails `STREAMON` with `-ENOMEM`
against the stock 512 MiB CMA pool:

- With a 128 MiB `memory-region` attached to the decoder node, default
  512 MiB CMA pool otherwise untouched: `STREAMON` succeeds, and the
  stream decodes via GStreamer's `v4l2slh265dec`.
- Decoded output is bit-exact against software decode (`avdec_h265`)
  across repeated runs.
- Clean module load/unload cycles; no behaviour change observed with the
  `memory-region` property absent (the existing/default configuration).
- Checked against `scripts/checkpatch.pl --strict`: the patch itself
  introduces no new style findings (five pre-existing findings elsewhere
  in `hevc_d_h265.c` are untouched by this change).

Two independent fixes for the underlying symptom exist; this PR is the
architecturally-correct one (a real dedicated pool for the decoder, frame
buffers stay where the display can reach them). The other is a config-only
workaround (raising the *default* CMA pool size so there's enough slack for
both GStreamer's worst-case pre-allocation and the decoder at once) that
doesn't need this driver change at all, for anyone who hits the same
symptom before this lands.

---

Signed-off-by: PLACEHOLDER_NAME <PLACEHOLDER_EMAIL>
