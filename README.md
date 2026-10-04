# trippy

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
| Space | Kaleidoscope, Mirror, Transform (zoom/rotate/tile), Wave warp |
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

Press `R` (or **⏺ Record**) to start and stop. Frames are captured straight from the
renderer, so the file has no UI and doesn't drop frames. It's saved as
`trippy-<timestamp>.mp4` (H.264, 1280×720, 60 fps) in the directory you launched from, using
the hardware encoder on macOS and x264 elsewhere. The file is finalized even if you quit
mid-recording. `--record` starts recording at launch. There's no audio.

## Controls

| Key | Action |
| --- | --- |
| 1–9 | launch scene (column) |
| Space | play / pause all clips |
| T | tap tempo |
| ← / → | crossfader |
| R | start / stop recording |
| S | save a PNG snapshot |
| C | clear all feedback / delay memory |
| Delete | remove the selected clip |
| F / Esc | performance mode (fullscreen output only) |
| drag / scroll on output | move / scale the selected layer |

**Output window** opens a second window you can drag to a projector and double-click to make
fullscreen.

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
