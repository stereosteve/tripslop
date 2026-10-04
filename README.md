# tripslop

A live video mixer (VJ tool) in Rust, in the spirit of Resolume Avenue: a clip grid of layers
× scenes, per-layer and master effect chains, blend modes, an A/B crossfader, clip loop modes,
beat-synced automation for every parameter, and recording. Its signature effect emulates an
analog **video-feedback fractal rig** (a camera pointed at monitors, with delay lines, keyers
and proc amps) on the GPU.

```
cargo run --release -- --demo
```

Requires `ffmpeg` and `ffprobe` on your PATH for video, cameras and recording. Images and
generators work without them.

## Demo

`--demo` builds a three-layer set from the media in `samples/` (run `samples/fetch.sh` to
re-download it). **Run it from the repo root**, since the sample paths are relative.

* **Footage** (bottom): jellyfish, Big Buck Bunny, a plasma generator, and a half-speed
  bouncing jellyfish
* **Fractal**: small images and a dot generator (the layer is scaled to 42%) going through the
  Feedback effect's *Sierpinski* preset, Screen-blended
* **Overlay**: generators through a kaleidoscope, Add-blended at 35%

Press `1`–`4` to launch scenes. Launches are quantized to the next beat.

Any file arguments are loaded into the first layer's cells: `cargo run --release -- a.mp4 b.png`.

Sample credits: *Jellyfish* test clip via test-videos.co.uk (footage from jell.yfish.us);
*Big Buck Bunny* © Blender Foundation, CC BY 3.0; *Pillars of Creation* (2014) and *Crab Nebula*
from NASA/ESA Hubble via Wikimedia Commons. See the source pages for the exact terms.

## Concepts

**Grid.** Rows are layers (the top row is drawn on top) and columns are scenes.
* Click a cell to launch its clip.
* Click a ▶ column header (or press its number key) to launch a whole scene, like an Ableton
  scene. Layers with an empty cell in that scene stop.
* *Launch: next beat / next bar* quantizes launches; pending ones blink amber.
* Drop files onto a cell (several files fill the cells that follow it), or right-click a cell
  to load a file, a generator or a camera.

**Layers.** Each layer has opacity, a blend mode (Normal, Add, Screen, Multiply, Difference,
Lighten, Darken, Overlay, Subtract), an A/B crossfader assignment, bypass/solo, a transform
(position, scale, rotation) and a clip transition time (a crossfade when switching clips).
It also has its own effect chain. Drag or scroll on the output monitor to move or scale the
selected layer.

**Clips.**
* Loop modes: **Loop**, **Bounce**, **Random** (jumps somewhere new every beat), **Play
  once** (then the layer goes empty) and **Play once & hold**.
* Direction: forward, reverse or paused, with scrubbing.
* Speed, or **BPM sync**, which stretches the clip to N beats.
* Fit modes: Fill, Fit or Stretch.

On import, each video is transcoded once into in-memory JPEG frames at 1280×720 (the same
idea as Resolume's DXV codec). That gives instant random access for bounce, reverse and random
playback. It takes about 100–150 KB per frame, so a 10 s clip at 30 fps is roughly 40 MB of RAM.

**Generators.** Bars, rings, plasma, checker, an orbiting dot, noise and solid color. Each
has frequency, speed and hue controls.

**Effects** (per layer, plus a master chain on the Composition tab):

| Category | Effects |
| --- | --- |
| Feedback | **Feedback / Fractal**: N scaled, rotated copies of the delayed output with a keyer, hue drift and symmetry. It has presets: Tunnel, Sierpinski, Mandala, Slow trails, Spiral galaxy, Hall of mirrors, Melt |
| Time | Echo trails, RGB time split |
| Space | Kaleidoscope, Mirror, Transform (zoom/rotate/tile), Wave warp, **Shape projector** (the layer mapped onto a spinning 3D prism, pyramid or diamond with 3–12 sides; size, height, rotation and beat-synced spin on each axis, per-face or wrapped mapping, lighting) |
| Color | Color (hue/sat/contrast/brightness/gamma/invert), Luma key, Pixelate / posterize |
| Stylize | Blur, Edges, CRT, Strobe (beat-synced) |

Effects can be reordered, bypassed and removed. Effects that need history (feedback and the
delays) each allocate their own 32-frame delay buffer when you add them.

Tip: the feedback rig's fractals need a seed that doesn't cover the whole frame. Scale the
layer down or use a generator. With a full-frame opaque clip, the input simply covers the loop.

**Custom shaders (live coding).** Paste a Shadertoy shader, or write your own, and it
compiles as you type, like KodeLife. Add one from a grid cell's right-click menu
(*Shader (GLSL)*), or as an effect (*+ Add effect → Custom shader*). You can also drop a
`.glsl` / `.frag` / `.fs` / `.wgsl` file onto a cell.
* **Shadertoy compatible** (single pass): `mainImage`, `iTime`, `iTimeDelta`, `iFrame`,
  `iResolution`, `iMouse` (Alt-drag on the output monitor), `iDate`, `iChannelResolution`.
  GLSL Sandbox style (`void main` + `gl_FragColor`, `time`, `resolution`, `mouse`,
  `backbuffer`) works too.
* **ISF** (the VDMX `.fs` format) works too. It's detected by its `/*{ JSON }*/` header.
  * `INPUTS` become controls:
    * `float` → a slider
    * `bool` / `event` → a toggle
    * `long` → a dropdown of its `LABELS`
    * `point2D` → x/y sliders
    * `color` → r/g/b/a sliders
  * Built-ins: `TIME`, `TIMEDELTA`, `RENDERSIZE`, `FRAMEINDEX`, `DATE`, `isf_FragNormCoord`,
    and the `IMG_NORM_PIXEL` / `IMG_PIXEL` / `IMG_THIS_PIXEL` / `IMG_SIZE` macros.
  * The first `image` input (e.g. `inputImage`) is the layer input.
  * A single `PERSISTENT` pass target is the shader's own previous frame. Multi-pass ISF isn't
    supported yet.
* **WGSL** also works. It's detected by `@fragment`, and your own entry point is used. It runs
  in screen space (y down). It can use:
  * `inputs.size` (`vec3f`), `inputs.time`, `inputs.mouse`, `inputs.date`, `inputs.frame`,
    `inputs.time_delta`, `inputs.beat`, `inputs.bpm`
  * `iChannel0`–`3` with the sampler `samp`
  * sliders declared as `// @param speed 0 4 1` and read as `inputs.speed`

  `src/shaders/marble.wgsl` pastes in unchanged, or run `cargo run --release -- src/shaders/marble.wgsl`.
* **Channels**:
  * `iChannel0`: the layer's input (when the shader is an effect)
  * `iChannel1`: the shader's own previous frame (feedback)
  * `iChannel2`: an RGBA noise texture
  * `iChannel3`: last frame's composition output
* **Sliders from code**: `uniform float amount; // min max default` (or `int`) becomes a
  slider with the usual `~` automation.
* **Tempo**: `iBeat` and `iBpm` expose the tempo clock.
* **Errors** appear with their line numbers (marked red in the gutter), and the last working
  version keeps running until you fix them. A bad shader can't crash the app, because the
  code is compiled and validated by naga before it reaches the GPU.
* **Alpha**: *Opaque*, *Luminance* (black becomes transparent) or *Shader alpha*.
* The editor has templates, Load/Save of `.glsl` files, and Cmd/Ctrl+Enter to compile
  immediately.
* **Limits**: no Shadertoy multipass buffers (A–D), audio, video or cubemap inputs, and
  samplers can't be passed into functions. naga is also stricter than WebGL; for example,
  write `ivec2(p) % ivec2(4)`, not `ivec2(p) % 4`.

**Automation.** Every parameter has a `~` button that attaches a signal generator:
* Shapes: sine, triangle, saw up/down, square, sample & hold, smooth random drift, or a
  hand-drawn envelope.
* Rate: synced to the BPM or free-running in Hz.
* Depth, polarity and phase.

The **Composition** tab lists everything that's automated.

## Recording

Press `Cmd/Ctrl+R` (or **⏺ Record**) to start and stop. Frames are captured straight from the
renderer, so the file has no UI and doesn't drop frames. It's saved as
`tripslop-<timestamp>.mp4` (H.264, 1280×720, 60 fps) in the directory you launched from, using
the hardware encoder on macOS and x264 elsewhere. The file is finalized even if you quit
mid-recording. `--record` starts recording at launch. There's no audio.

## Punch-in FX

Inspired by the OP-Z / KO II punch-in effects: **hold** a pad for a momentary, beat-synced
transformation, and release to snap back. Hold several at once to stack them, or
**Shift+key / Shift+click** to latch a pad for hands-free builds. Pads ease in and out, and
know how long they've been held, so builds intensify the longer you hold.

Punches aren't just master effects. Each one is a small routine that can:
* inject temporary effects into individual layers (alternating layers, the top layer, a
  random layer each beat…)
* gate or pump layers in beat-locked patterns
* take over clip playback

They never modify your set: release a pad and everything is exactly as it was.

| Key | Pad | What it does |
| --- | --- | --- |
| Q | Stutter | Beat repeat on every clip: 1/2 beat, then 1/4, then 1/8 the longer you hold |
| W | Chase | One layer visible at a time, stepping through the layers on 1/16s |
| E | Mirror split | Alternate layers mirror sideways / vertically; the top layer turns kaleidoscope |
| R | Pump | Each layer zoom-pumps on the beat, phase-shifted per layer |
| T | Tape stop | Clips grind to a halt over a beat while the picture sags and drains |
| Y | Reverse | Clips play backwards with an RGB time split and a hue flip |
| U | Echo build | 1/16-note echo trails that thicken while held |
| I | Riser | Two-bar build: zoom, blur and brightness climb; an accelerating strobe kicks in |
| A | Strobe split | Even layers on the beat, odd layers on the off-beat |
| S | Kaleido spin | Master kaleidoscope spinning with the beat |
| D | Tunnel | Master feedback tunnel, turning the other way every bar |
| F | Fractal bloom | The top layer blooms into a Sierpinski fractal; the others dim |
| G | Glitch | Random per-layer jumps, a random layer pixelates, CRT master |
| H | Invert flip | Layers invert in alternation, flipping every beat |
| J | Wash out | Whiteout transition over a bar |
| K | Trance gate | Master chopped by a 16-step gate |

## Controls

| Key | Action |
| --- | --- |
| Q–I, A–K | punch-in FX (hold; Shift = latch) |
| 1–9 | launch scene (column) |
| Space | play / pause all clips |
| Enter | tap tempo |
| ← / → | crossfader |
| Cmd/Ctrl+R | start / stop recording |
| Cmd/Ctrl+S | save a PNG snapshot |
| Cmd/Ctrl+K | clear all feedback / delay memory |
| Cmd/Ctrl+F, Esc | performance mode (fullscreen output only) |
| Delete | remove the selected clip |
| drag / scroll on output | move / scale the selected layer |

**Output window** opens a second window you can drag to a projector and double-click to make
fullscreen.

## Scripts and testing

`--script FILE` runs timed commands against the real app. Use it for automated tests and CI,
for capturing stills, or to set up a performance.

```
cargo run --release -- --script scripts/smoke.tripslop
```

Scripts run in **fixed-step** mode: the 60 Hz clock advances by simulated ticks rather than
wall time. That makes results independent of machine speed, and runs finish faster than real
time. `--realtime` turns this off and `--fixed-step` forces it on without a script.

The process exits with status **1** if an `assert` fails or a command errors. A bad script
exits with **2** before opening a window.

```text
# comments start with #
demo                          # runs at time 0
at 1.25b launch-scene 2       # times: frames (default), Ns seconds, Nb beats
at +4b   hold Q 4b            # +N = relative to the line above; hold = pad down, up 4 beats later
set Fractal/feedback/rotate 15
assert active 1 == 2
at 600   snapshot target/script-out/end.png
screenshot target/script-out/ui.png
quit
```

Layers and columns are **1-based**, like the UI.

| Command | |
| --- | --- |
| `demo` | load the demo set |
| `launch-scene N` · `launch L C` · `stop L` | launch a scene / a clip; stop a layer |
| `load L C PATH` · `generator L C NAME` · `shader L C TEMPLATE` · `camera L C INDEX` | put media in a cell (layers are created as needed) |
| `add-effect L NAME` · `add-effect master NAME` | add an effect by name; `shader:TEMPLATE` adds a code effect |
| `set PATH VALUE` | set a parameter (see paths below) |
| `bpm N` · `quantize off/beat/bar` · `play` · `pause` | transport |
| `pad KEY down/up/latch/unlatch` · `hold KEY DURATION` | punch-in pads, by key, name or number |
| `select L C` · `tab layer/composition` · `open-editor L C` | UI state, for screenshots |
| `snapshot PATH` · `screenshot PATH` · `record start/stop` | output frame, full window, video |
| `print WHAT` · `assert WHAT OP VALUE` · `quit` | checks; `OP` is one of `== != < <= > >= ~=` |

**`WHAT`** is either a parameter path or one of: `playhead L`, `active L` (1-based column, 0
for none), `pad KEY` (envelope 0..1), `errors L C` (shader compile errors), `layers`, `bpm`
or `beat`.

**Parameter paths** have one of these forms:
* `LAYER/PARAM`, `LAYER/EFFECT/PARAM`, `LAYER/clip/PARAM` (the playing clip) or
  `LAYER/clipN/PARAM`
* `master/master`, `master/crossfader` or `master/EFFECT/PARAM`

`LAYER` is a number or a name. Effects can also be given as `fxN`. Names match
case-insensitively by prefix, so `2/feedback/rot` works.

**Tests:**
* `cargo test` runs the unit tests (no window).
* `cargo test --release -- --ignored` also runs the scripts in `scripts/` (smoke, shaders,
  punch tour) as end-to-end tests. These need a GPU and a window session. Captures go to
  `target/script-out/`.

For a quick single still, use `TRIPSLOP_SNAPSHOT=<frames>:<out.png>`. It saves the output after
that many frames and quits.

## How it works

```
for each layer (bottom → top):
    clip(s) ── clip pass: transform, fit, transition ──► layer texture
    layer   ── effect → effect → … (each with an optional history ring) ──►
    comp    ── composite: blend mode × opacity × crossfader ──► comp
comp ── master effects ──► final (master fader) ──► screen / output window / recorder
```

Everything runs on a fixed 60 Hz clock, so effects, delays and BPM sync behave the same on
any display. Shaders are validated by `cargo test`.
