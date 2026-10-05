# 3. Output resolution

## Problem

`renderer::WIDTH`/`HEIGHT` are 1280×720 constants, used for every render target, shader
uniforms, the recorder and snapshots. Going to 1080p also multiplies history memory: a
32-frame ring is about 112 MiB at 720p and about 253 MiB at 1080p, for every
feedback/echo/time-split effect.

## Plan

**Resolution setting**
- Replace the constants with a `size: (u32, u32)` on the renderer, with presets 720p, 1080p
  and Custom.
- `--size 1920x1080` on the command line and a `size` script command.
- Changing it rebuilds the render targets and clears feedback history. That's fine; it's not
  something you do mid-set.
- Recorder: `bytes_per_row` must be a multiple of 256. 1280 and 1920 are both fine, but
  pad and strip for custom widths.
- Video import stays at 1280×720 for now; clips get scaled up. Import size can be a
  separate setting later if it looks soft.

**Cheaper history**
- A per-effect *history quality* option: Full / Half (half-res ring). Feedback is blurry
  by nature, so Half probably looks the same at a quarter of the memory.
- Use the real history size in the texel-size uniforms.

**Memory readout**
- A rough "GPU memory ≈ N MB" readout in the Composition tab: render targets, history
  rings, layer pairs, plus clip RAM (`VideoMedia::memory_bytes` already exists).

No budgets, no automatic downgrades, just the number.

## Done when

- The demo, a custom shader, snapshot and recording all work at 720p, 1080p and an odd
  width like 1366×768.
- The output window keeps the right aspect ratio.
- `scripts/smoke.tripslop` still passes at the default size.
