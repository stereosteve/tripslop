# 13. Time and glitch effects

The history ring that feedback and echo use (32 past frames per effect) is good for a whole
family of effects Arkestra has and we don't. Most of them fit `EffectDef` as it is: one pass,
some parameters, a history ring. Pixel sort is the exception and goes last.

## Plan

In the **Time** category:
* **Frame stutter**: every beat division (1/16 to 1 bar) it rolls a probability, and if it
  fires, it freezes the frame for *hold* (1/32 to 1 beat), then snaps back to live. Built on the
  history ring: while frozen, the tap stays on the frame where the freeze started. The beat
  arrives as a uniform, so the logic is all in the shader plus a small `taps` function.
* **Buffer loop**: *grab* (a toggle or a pad) captures the last N beats (up to the ring's length)
  and loops them, optionally ping-pong. Releasing it goes back to live. The ring needs to be
  longer for this one (64 frames at half size). That's an `EffectDef` setting, not a new system.
* **Slit scan**: each row (or column, or ring outward from the centre) reads from a different
  frame in the history: top is now, bottom is N frames ago. *Speed* scrolls the offset.

In the **Glitch** category (new):
* **JPEG crush**: 8×8 block DCT, quantise, inverse DCT in one fragment pass (each pixel works
  out its own block's coefficients). Gives blocking, ringing and chroma bleed. *Quality*,
  *chroma subsampling*, and *block jitter* (on the beat).
* **Datamosh (fake)**: no codec needed. Each frame, estimate block motion between the input and
  the previous input (a small search per 16×16 block), then advect the effect's *own previous
  output* by those vectors instead of showing the new frame. *Refresh* (a trigger) brings a clean
  frame in, like an I-frame. *Bloom* repeats the advection. It's a two-tap history effect.
* **Pixel sort**: a compute pass (bitonic sort per row or column, in shared memory, up to 2048
  wide), sorting by luma, hue or saturation. The run modes are *threshold* (sort only spans
  between a low and a high key), *edges* and *random runs*, with *direction* and *reverse*. This
  is the first compute pipeline in the renderer, so it lands on its own, after the others.

All of them get wet and wet blend from item 9 for free.

## Scripts and tests

`scripts/time-fx.tripslop` puts each effect on a moving generator, runs a few beats in
fixed-step mode, and snapshots them. It asserts that the frozen frames of the stutter are
identical (comparing snapshots) and that the slit scan's top row matches the live input.

## Done when

* Stutter and buffer loop make a straight video clip feel cut to the beat.
* Datamosh smears a cut between two clips the way the real thing does.
