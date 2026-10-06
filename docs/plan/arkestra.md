# Arkestra: what it has, what we take

[Arkestra](https://www.arkestra.app/) (Rhythmic Visions, macOS, Swift + Metal, v3.2.2 at the time of writing) is
the VJ app the [TDLiDAR](https://tdlidar.github.io/) iPhone sensor bridge lists next to
TouchDesigner. It's the closest thing to tripslop out there: ISF shaders, audio-reactive
modulation, feedback, a 3D renderer, and a live-instrument attitude. This page is what we found by
reading the whole manual (docs.arkestra.app, 47 pages) and looking through the app bundle in
`/Applications/Arkestra.app`, and what's worth taking.

## How Arkestra is built

```
Project (.vfx: a folder with a JSON "project", a screenshot and thumbnails)
└── Track (Layer, Effect Bus, Feedback Loop, 3D, Laser)       ← fader, blend, colour
    └── Chain (many per track, one active: instant-swap preset slots)
        ├── pre-feedback devices → feedback tap → post-feedback devices
        ├── up to 5 chain macros
        └── Device (ISF shader, video, camera, 3D scene, GPU effect)  ← dry/wet, blend, bypass
            └── Parameter (float, bool, colour, select, point2d, texture)
```

* **Scenes** pick the active chain on every track (plus fader and running state). **Chain
  snapshots** are 8 parameter presets per chain, recalled instantly or crossfaded over 0–5 s
  with an easing curve, with prev/next/random and beat-synced autoplay. **Project snapshots**
  (32) are full-state checkpoints, deliberately not for live use.
* **Every parameter** is the same type with a mapping and a response stage. In the project JSON
  each one carries `scale { min_value, max_value, offset, invert_value, smoothing_active,
  smooth_rise, smooth_fall, treshold_min, treshold_max }` next to its `map`. So any source
  (LFO, sequencer, audio band, MIDI, OSC, macro…) gets range, invert, smoothing with separate
  rise and fall times, and a threshold window. A *transfer curve* (stepped or linear) can remap
  the signal too. Mappings stack additively.
* **Sources:** LFOs (9 shapes; synced, free in Hz, or *triggered*: each trigger advances the
  phase by a step), 16-step **sequencers** with Elektron-style conditional trigs (probability,
  `2:4` cycles, FILL / !FILL, PRE, FIRST, LAST) and an optional ADSR, global LFOs and
  sequencers shared across the project, 3 audio detectors (kick / snare / hi-hat onsets with
  attack, release and threshold, each on its own input channel), MIDI (CC, note gate/toggle
  with velocity, keytrack and ADSR, PC, pitch bend), OSC, Art-Net, CLX (DJ deck state), Ableton
  Link, Apple Vision (face, hands, body), and a timeline with recordable automation lanes.
* **Every device has a Mix (dry/wet) slider and a blend mode** for how the wet signal lands
  on the dry one.
* **GPU effects** beyond single-pass ISF: Datamosh (real H.264 with dropped I-frames),
  Slit Scan (60-frame ring), Pixel Sort (rows, columns, rays, circles; several run modes),
  Buffer Loop (grab N beats and loop them), Frame Stutter (beat-synced freeze with probability),
  Quality Crusher, a JPEG crusher, LUT (`.cube`, ships 8 looks), Tonemap (switches the chain
  to RGBA16F), Fluid Solver, Wave Sim, Image Tracer (marching-squares contours), Reactive Poster
  (editorial grid layouts with type), Channel Reorder, Track Texture (route any track into any
  chain), Oscilloscope.
* **3D tracks:** point clouds (from PLY files, a live texture with luma or ML depth, a solid
  grid, text, iPhone LiDAR, audio Chladni / Lissajous / radial modes), Gaussian splats, SDF
  shapes (Mandelbox, fractal tunnel), with displacement by noise, a 3D fluid sim, a spring
  particle sim with up to 8 force generators (directional, radial, vortex, pulse on a trigger,
  noise, drag, excite, texture force), directional flow, or a "displacement track" whose RGB
  is read as XYZ offsets. Bloom, DOF, god rays, ACES.
* **Track options:** cue (pre-fader preview on the editor only; the output keeps playing),
  chain audition (preview a chain, then TAKE), a mask (shape, image, a live texture's luma or
  a drawing), a transform, momentary mode (hold a key to show the track, with attack and
  release), chain crossfades with transition styles (zoom, slide, rotate, darker, brighter),
  blackout, effect-bus conversion, Syphon out per track.
* **Output:** external windows, quad projection mapping, Syphon in/out, NDI out, recording to
  H.264, HEVC or ProRes with optional audio.
* **Studio:** an ISF editor with an AI chat that writes and edits shaders through a tool call
  and keeps an undo for each change. It runs on their servers, metered per month.
* **Controller profiles** (`.arksp` JSON in the bundle) for the APC Mini MK2, nanoKONTROL2,
  Launchpad Mini MK3 and Kontrol F1, mapping `track.N.fader`, `track.N.chain.M`, next/prev to
  notes and CCs, with LED feedback.
* The binary also has a built-in synth (`SynthEngine`, with JSON patches like FMKick and
  TapeDrone), an auto-BPM and rhythm analyser, and an AU/VST3 plugin ("Arkestra Echo") that
  sends audio analysis from inside Ableton.

## What tripslop already has

The clip grid with scenes and quantized launches; per-layer and master chains; 9 blend modes;
the A/B bank crossfader; clip loop modes, BPM sync and transitions; one LFO, envelope or audio
modulator per parameter; audio bands, kick and the spectrum; MIDI learn and MIDI clock; ISF,
Shadertoy and WGSL live coding; 355 bundled ISF shaders; the feedback fractal rig; 3D models in
the shape projector and projection mapping; punch-in pads; recording; scripts.

A lot of Arkestra's structure maps onto tripslop's: a *chain* is roughly a clip column, a
*scene* is a scene column, and a *device* is an effect card. The gaps are mostly in how
parameters are driven and recalled, and in a handful of effects.

## What we take

In rough order of how much they make tripslop nicer to play:

| # | Item | Size | Arkestra feature |
| --- | --- | --- | --- |
| 9 | ✅ [Dry/wet on every effect](09-dry-wet.md) | small | Device Mix + blend |
| 10 | [Step sequencer and response shaping](10-sequencer.md) | small–medium | Sequencers, conditional trigs, smoothing |
| 11 | [Macros and shared modulators](11-macros.md) | medium | Chain macros, global LFOs |
| 12 | [Snapshots that morph](12-snapshots.md) | medium | Chain snapshots, scene autoplay |
| 13 | [Time and glitch effects](13-time-fx.md) | medium | Frame stutter, buffer loop, slit scan, pixel sort, crushers |
| 14 | [LUTs](14-lut.md) | small | LUT effect |
| 15 | [Cue](15-cue.md) | small–medium | Track cue |
| 16 | [Effect-bus layers and masks](16-bus-masks.md) | medium | Effect Bus tracks, track masks |
| 17 | [OSC and Ableton Link](17-osc-link.md) | medium | OSC, Link (and TDLiDAR's OSC) |
| 18 | [Point clouds](18-point-clouds.md) | large | 3D tracks, displacement, particle sim |
| 19 | [Saving sets](19-sets.md) | medium–large | `.vfx` projects |
| 20 | [Claude in the Code tab](20-claude-code-tab.md) | medium | Studio AI assistant |

## What we leave

* **Apple-only frameworks:** Vision tracking, Depth Anything on the Neural Engine, VideoToolbox
  datamosh, AU hosting. tripslop runs everywhere wgpu does. (A fake datamosh from block motion
  and the history ring is in item 13.)
* **Online catalogues and accounts:** Pexels, Giphy and Unsplash browsers, community shaders,
  cloud sync, metered AI credits.
* **Multiple chains per track.** Clip columns plus scenes already give instant-swap looks, and
  snapshots (12) cover parameter recall within one chain. Revisit if a set needs two different
  effect stacks on the same clip.
* **Timeline automation lanes.** tripslop is a launcher, not an arranger. Scripts already cover
  timed shows.
* **Art-Net, CLX, lasers, the synth, the reactive poster, the image tracer, Gaussian splats.**
  Fine features, and nobody here needs them yet.
* **Syphon / NDI and projection mapping** stay in the README's *Not now* list. Arkestra keeps
  them behind its Pro tier, which says people pay for them. Start with Syphon when a real rig
  needs it.

Arkestra is a commercial, closed app. We take ideas, not files: the LUTs, shaders, SVGs and
profiles in its bundle stay there, and anything we ship is written or generated here.
