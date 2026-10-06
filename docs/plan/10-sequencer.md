# 10. Step sequencer and response shaping

The modulator has smooth shapes and random ones, but nothing rhythmic that you write yourself.
The drawn envelope comes closest, but it's one continuous curve. Arkestra's 16-step sequencer
with Elektron-style conditional trigs is how it gets patterns that change from bar to bar, and
its per-mapping smoothing (separate rise and fall) is how audio and MIDI stop looking jittery.

## Plan

**Sequencer shape.** A new `Shape::Steps` in `modulation.rs`:
* 1–16 steps, each with a level (0..1) and an on/off. The rate is a beat division, like the
  other synced shapes, and gives the length of one step.
* **Glide** (0..1): 0 holds each step's level until the next step, 1 slides all the way to the
  next one.
* **Conditions** per step, chosen in a small menu and shown as a badge on the step:
  * *always* (the default)
  * *probability* (10–90 %, rolled once when the step starts so it can't flicker)
  * *cycle* `n:m` (fires on pass n of every m loops through the pattern)
  * *fill* and *not fill*
  * *first* (only the first pass after the beat clock starts)
  
  A step that doesn't fire holds the previous level.
* **Fill** is a global momentary switch: a button in the transport, a key (`Shift+Enter`), and
  MIDI-learnable. Every sequencer sees the same fill state.
* The editor in the automation popup is a row of 16 bars. Drag to set the levels, click to turn
  a step off, right-click to set the condition, and shift-drag to paint across several steps.
  A playhead shows the current step, and steps whose condition fails this pass are dimmed.
* Odd step counts against 4/4 drift, which is the point.

**Response shaping**, on every modulator:
* **Smooth rise / smooth fall** (0 to 2 s each): a one-pole follower applied to the signal
  before it's scaled, with separate time constants for going up and coming down. A kick with fast
  rise and slow fall pumps nicely, and S&H with some smoothing becomes a stepped drift.
* **Invert** flips the signal.
* The smoothing state lives on the `Param`, not the `Modulator`, so the follower runs once per
  tick whatever reads it.

**More audio triggers.** The kick detector becomes one of three onset detectors: **kick**
(45–300 Hz), **snare** (1.5–4 kHz) and **hat** (4–12 kHz). They use the same envelope follower,
and each has its own threshold. They're added to the Audio shape's band list and as shader
uniforms (`iSnare`, `iHat`, ISF `audioSnare`, `audioHat`).

## Scripts and tests

* Unit tests for the step clock: the level at each beat position, glide, a `1:4` cycle firing
  once in four loops, a seeded probability being repeatable, fill.
* A smoothing test: a square wave through rise 0 / fall 0.5 s decays at the right rate.
* `scripts/sequencer.tripslop` drives a layer's opacity from a 5-step pattern and asserts the
  values on chosen beats (fixed-step mode makes them exact).

## Done when

* A 16-step pattern can strobe a layer's opacity in time, and a fill pattern takes over while
  the Fill button is held.
* An audio-driven knob can be made to snap up on the kick and fall back slowly.
