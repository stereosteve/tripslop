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
| 8 | ✅ [UI redesign](08-ui-redesign.md) | large | It looks like a debug panel; hard to see what's live |

The order is a suggestion. 4 and 5 are the fun ones, and nothing stops you doing them first.

### Ideas from Arkestra

[Arkestra](https://www.arkestra.app/) is the closest app to tripslop. [arkestra.md](arkestra.md)
covers what it has, what we already have, and what we're leaving out.

| # | What | Size | Why |
| --- | --- | --- | --- |
| 9 | ✅ [Dry/wet on every effect](09-dry-wet.md) | small | Effects are all or nothing |
| 10 | [Step sequencer and response shaping](10-sequencer.md) | small–medium | No rhythmic patterns of your own; audio mappings jitter |
| 11 | [Macros and shared modulators](11-macros.md) | medium | One knob can't drive a whole look; LFOs can't be shared |
| 12 | [Snapshots that morph](12-snapshots.md) | medium | A good look is lost as soon as you touch a knob |
| 13 | [Time and glitch effects](13-time-fx.md) | medium | Stutter, buffer loop, slit scan, datamosh, pixel sort |
| 14 | [LUTs](14-lut.md) | small | No colour grades |
| 15 | [Cue](15-cue.md) | small–medium | New looks get built in front of the audience |
| 16 | [Effect-bus layers and masks](16-bus-masks.md) | medium | Effects are per layer or master, nothing between |
| 17 | [OSC and Ableton Link](17-osc-link.md) | medium | Phones (TDLiDAR), TouchOSC, and band sync |
| 18 | [Point clouds](18-point-clouds.md) | large | Pictures lifted into 3D and blown apart on the kick |
| 19 | [Saving sets](19-sets.md) | medium–large | Nothing survives a restart; 12 makes that hurt |
| 20 | [Claude in the Code tab](20-claude-code-tab.md) | medium | Describe a shader, get one that compiles |

9 to 12 are what changes how it plays; do those first. 19 is needed once 12 lands.


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
