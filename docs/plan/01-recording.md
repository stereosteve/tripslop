# 1. Recording that never stalls

## Problem

`Recorder::capture` (`src/recorder.rs`) chooses "never drop a frame" over "never hitch":

- If all 4 staging buffers are still in flight, it calls `drain(device, true)`, which does
  `PollType::Wait` and blocks the render thread on the GPU.
- `send` uses a `sync_channel(16)`, so if ffmpeg falls behind, the render thread blocks
  until it catches up.

That's the right trade for offline export (see [6](06-export.md)), but the wrong one live,
where a hitch is visible on the projector.

## Plan

Give the recorder a mode: `Live` (the default for Cmd+R) and `Exact` (scripts and export).

In `Live` mode:
- No free staging buffer → skip this frame's capture.
- Writer queue full → `try_send`; if it fails, drop the frame.
- Count skipped frames, and have the writer thread repeat the last frame once for each
  skipped one, so the file's duration still matches real time at constant 60 fps. Send a
  small `Frame(Vec<u8>)` / `Repeat(n)` message so the repeats don't copy 3.7 MB each on the
  render thread.
- Show "dropped N" next to the ⏺ indicator when N > 0.

`Exact` keeps today's behavior.

## Done when

- Recording a heavy feedback set doesn't hitch the output more than not recording does.
- An artificially slowed writer (e.g. a `sleep` in the writer thread) produces a file with
  the right duration (check with `ffprobe`) and repeated frames, not a frozen UI.
- Script-driven `record start/stop` still captures every frame.
