# tripslop plan

What to work on next. tripslop is an instrument, so this list favors things that make it
nicer to play. Infrastructure work only goes in when something is actually broken or
blocking.

| # | What | Size | Why |
| --- | --- | --- | --- |
| 1 | ✅ [Recording that never stalls](01-recording.md) | small | Recording can hitch the live output |
| 2 | ✅ [Bank crossfader](02-crossfader.md) | small–medium | A/B fades feel like a delayed cut |
| 3 | ✅ [Output resolution](03-resolution.md) | medium | Stuck at 720p |
| 4 | ✅ [Audio reactivity](04-audio.md) | medium | Visuals that follow the music |
| 5 | ✅ [MIDI](05-midi.md) | small–medium | Play the punch pads on hardware |
| 6 | [Offline export](06-export.md) | small–medium | Frame-perfect renders from a script |
| 7 | [Frame-time overlay](07-perf-overlay.md) | small | Know when you're dropping frames |
| 8 | [UI redesign](08-ui-redesign.md) | large | It looks like a debug panel; hard to see what's live |

The order is a suggestion. 4 and 5 are the fun ones, and nothing stops you doing them first.

## Ground rules

- Keep it one binary, one wgpu device, one process.
- Each item should land as a few small commits, with a script in `scripts/` that exercises it
  where that makes sense.
- Don't change existing behavior without a toggle when sets or muscle memory depend on it
  (for example, the crossfader).
- Add a feature when you need it, not because it might come in handy someday.

## Not now

These come up in comparisons with bigger tools (Resolume, ghost-arcade). They're fine
ideas that aren't needed yet. Revisit one when a real problem shows up.

- **Splitting effects into "instruments" with host-owned resources.** `EffectDef` with
  `history: Option<History>` is already a clean enough boundary.
- **Memory budgets and resource accounting.** A memory number in the UI is enough (see 3).
- **Disk-backed frame cache or hardware video decode.** JPEG-in-RAM is what makes reverse,
  bounce and random playback cheap. Revisit if long clips start eating all the RAM.
- **Projection mapping:** corner pin, masks, multiple outputs, edge blending. If you start
  pointing a projector at real objects, corner pin on the output window is the first step.
- **Syphon/Spout/NDI.**
- **Plugins, node graphs, multi-process anything.**
