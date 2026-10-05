# 6. Offline export

A frame-perfect render of a scripted performance, for clips and loops you want to post or
reuse.

## Plan

Most of this exists already. `--script` runs in fixed-step mode, and `record start/stop`
works from scripts. What's missing:

1. **Exact video frames.** The video decoder is asynchronous: `VideoMedia::request` asks for
   a frame and `take_decoded` gets it whenever it's ready. In live mode, showing the
   previous frame is fine. For export, the tick should wait until each active clip's
   requested frame has arrived.
2. **Don't wait for the display.** In export mode, step ticks as fast as rendering and
   encoding allow, not on vsync, so a 30 s export can finish in less than 30 s (or take
   longer when it's heavy, and still be correct).
3. **Exact recorder mode** (see [1](01-recording.md)), so every tick becomes exactly one
   frame.
4. **A clean start:** clear histories, reset shader frame counters and seed the random
   modulators when export begins, so the same script gives the same video.

Usage:

```
cargo run --release -- --export out.mp4 --script scripts/my-set.tripslop
```

`--export` implies fixed-step mode and starts recording at tick 0. `quit` ends the export.
Optionally, show a progress bar in the window.

## Done when

- Exporting the same script twice gives the same frame count, and visually identical
  output (GPU float noise aside).
- Reverse/random video clips show the right frames in the export, with no repeated stale
  frames.
