# 16. Effect-bus layers and masks

Effects are per layer or on the master: nothing in between. Arkestra's *Effect Bus* track
processes everything below it in the stack, so you can run a kaleidoscope over the bottom
three layers and leave the overlay on top clean. It also has a *mask* on every track, which
is how one layer cuts a shape out of another.

## Plan

**Effect-bus layers.**
* A layer can be switched to **Bus** in its … menu. A bus layer has no clips (its grid row shows
  a band instead of cells). Its input is the composition so far, meaning everything below it, and
  its chain processes that.
* The result goes back into the composition with the layer's blend and opacity. At Normal and
  100 % it replaces what's below, so opacity works as dry/wet for the whole bus.
* Mute, solo, the A/B side, snapshots and macros all work as on a normal layer. In Banks
  crossfader mode a bus processes the bank it's on (or both, when unassigned).
* In the renderer it's one more case in the layer loop: run the chain on `comp_bufs[cur]` instead
  of on the layer's clips.

**Masks.** Each layer gets an optional **mask** section in its Layer out card:
* **Shape**: circle, rectangle, diamond or a linear or radial ramp, with position, size,
  softness and rotation, all automatable.
* **Layer**: the luma or alpha of another layer's output (pre-fader). That layer can be muted
  so it only works as a mask, the way Arkestra's "background render" tracks are used.
* **Invert** and **threshold / softness** for both.
* It's applied as the last step before compositing, so effects and feedback inside the layer
  aren't affected.

## Scripts and tests

`scripts/bus.tripslop`: three layers with a bus between layers 2 and 3 running Invert. It
asserts that the top layer's pixels are unchanged and the bottom ones are inverted.
`scripts/mask.tripslop`: a circle mask with a snapshot, and a layer mask driven by a moving
generator.

## Done when

* A kaleidoscope bus over the footage leaves the overlay layer on top untouched.
* One layer's moving shapes cut holes in another.
