# 7. Frame-time overlay

Enough diagnostics to answer "am I dropping frames, and roughly why?"

## Plan

- A toggleable overlay on the output monitor (not the output window), maybe on
  Cmd/Ctrl+I, showing:
  - ticks per second vs. 60, and how many ticks were caught up last second
  - CPU time per frame (prepare + submit), average and worst over the last second
  - video frames waiting on decode (a clip that's asked for a frame it doesn't have yet)
  - recorder: queued / dropped (from [1](01-recording.md))
  - GPU memory estimate (from [3](03-resolution.md))
- `print fps` / `print frametime` script commands, so performance runs can log numbers.

GPU timestamp queries, percentile tracking and saved reports can wait until there's a
specific slowdown to investigate.
