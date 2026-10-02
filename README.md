# trippy

A two-deck video DJ mixer in Rust that emulates an analog **video-feedback fractal rig**
(camera pointed at monitors, with mirrors, delay lines, keyers and proc amps) on the GPU.

```
cargo run --release -- [deckA.mp4|png] [deckB.mov|jpg]
```

Requires `ffmpeg` on your PATH for video and camera decks (images work without it).

## Demo

Sample clips and images are in `samples/` (run `samples/fetch.sh` to re-download them):

```
cargo run --release -- --preset 6 samples/jellyfish.mp4 samples/crab-nebula.jpg
```

This puts the jellyfish on deck A and the Crab Nebula on deck B, starting on the
*Spiral galaxy* preset. Then try:

* press `2` (Sierpinski) or `3` (Mandala), then **scroll on the preview to shrink the deck**:
  a small source acts as a seed and the fractal copies grow around it (full-screen
  bright footage covers the whole loop)
* press `5` for the RGB time-split, `8` for the melt
* swap in the other samples, e.g.
  `cargo run --release -- --preset 8 samples/big-buck-bunny.mp4 samples/pillars-of-creation.jpg`

`--preset N` (1–9) picks the starting preset. Any other arguments are files for deck A and B.

Sample credits: *Jellyfish* test clip via test-videos.co.uk (footage from jell.yfish.us);
*Big Buck Bunny* © Blender Foundation, CC BY 3.0; *Pillars of Creation* (2014) and *Crab Nebula*
from NASA/ESA Hubble via Wikimedia Commons. See the source pages for the exact terms.

## How it works

```
 deck A ─┐
         ├─ mixer ─► input ─┐
 deck B ─┘                  ├─ feedback stage ─► output ─┬─► delay line (64-frame ring buffer)
     delay line[now - d] ───┘   (N copies, keyer,         └─► output stage ─► screen
                                 hue/sat/contrast)
```

* **Feedback / fractal**: every 1/60 s, *N* scaled and rotated copies of an earlier output
  frame are composited, like a camera seeing *N* monitors that show its own output. Do that
  over and over and you get an iterated function system: 3 copies at scale 0.5 make a Sierpinski
  triangle, and adding rotation and twist gives spirals and mandalas.
* **Video delay**: the feedback reads from a ring buffer, so the loop can lag behind by up to
  60 frames. There are also echo taps, plus an RGB time-split where green and blue come from
  older frames.
* **Keyer**: fresh input is luma-keyed (or added, lightened, differenced) over the loop.
* **Placement**: each deck has a size, an x/y position, and Fill (crop to the frame) or Fit
  (show the whole image). Set them with the sliders or by dragging and scrolling on the preview.
* **Automation**: every slider has a `~` button. Clicking it attaches a signal generator and
  opens an editor:
  * shapes: sine, triangle, saw up/down, square (adjustable width), S&H random, smooth random
    drift, or a hand-drawn **envelope** (click to add points, drag to move them, right-click to
    delete)
  * rate synced to the BPM (1/4 to 128 beats per cycle) or free-running in Hz
  * depth (a fraction of the slider's range), polarity (± around the slider, + above, − below)
    and phase
  * a live plot with a playhead

  The slider still sets the center value. Automated sliders show a pink dot for the live value
  and a band for the sweep range. The **AUTOMATION** panel lists everything that's automated,
  with on/off and remove buttons. Presets replace the effect automation but keep any automation
  on the decks and the crossfader.
* **Decks**: images, looping videos (any format ffmpeg reads, GIFs included), live cameras, or a
  built-in oscillator pattern (bars, rings, plasma, checker, orbiting dot).

## Recording

Press `R` (or the **⏺ Record** button) to start and stop recording. Frames are captured
straight from the renderer, so the file has no UI or cursor and doesn't drop frames. It's
saved as `trippy-<timestamp>.mp4` (H.264, 1280×720, 60 fps) in the directory you launched
from. Encoding uses the hardware encoder on macOS and falls back to x264 elsewhere. If you
quit while recording, the file is still finalized.

To record a whole session from launch:

```
cargo run --release -- --record --preset 6 samples/jellyfish.mp4 samples/crab-nebula.jpg
```

Recording uses ffmpeg. There's no audio, so add music afterwards in an editor.

## Controls

| Key | Action |
| --- | --- |
| 1–9 | presets (Tunnel, Sierpinski, Mandala, Slow echo, Time smear, Spiral, Hall of mirrors, Melt, Clean) |
| Space | freeze the loop |
| C | clear the feedback memory |
| T | tap tempo (beat-synced automation follows it) |
| Z / X | cut to deck A / B |
| ← / → | crossfade |
| ↑ / ↓ | copy scale (zoom) |
| R | start / stop recording a video |
| S | save a PNG snapshot |
| G | switch which deck the preview drag/scroll controls |
| drag / scroll on preview | move / resize that deck (trackpad pinch works too) |
| F / Esc | performance mode (fullscreen output only) |

Drop files onto a deck panel to load them. If you drop them anywhere else, they go into the
deck that's currently off-air. **Output window** opens a second window you can drag to a
projector and double-click to make fullscreen.
