# 15. Cue

Building a new look during a set means building it in front of everyone. Arkestra's *cue* is
pre-fader listen for visuals: the editor's preview shows one track on its own while the output
window, recording and Syphon keep showing the mix.

## Plan

* A **headphones button** in each track header (and `C` for the selected layer) cues that
  layer. Only one layer is cued at a time.
* While a layer is cued, the right rail's monitor shows that layer's output on its own,
  *pre-fader*: after its effects, before opacity, blend and the crossfader. An amber **CUE ·
  Layer name** badge sits on the monitor with a × to stop cueing. The output window,
  fullscreen, recording and snapshots still show the program.
* A cued layer renders even at opacity 0 or muted, so you can build it out of the mix and then
  fade it in. That fade is the take; there's no separate button. The renderer already keeps
  every layer's result in `self.layers`, so this is mostly choosing which texture the monitor
  draws.
* Dragging on the monitor while cued moves the cued layer, as now.
* Perform view gets a small cue button on each channel, and the big output stays on program.
* Cue isn't saved. It's live state.

## Scripts and tests

`cue 2` / `cue off` script commands. `scripts/cue.tripslop` mutes layer 2, cues it, and
asserts with `screenshot` that the monitor shows it while `snapshot` (program) doesn't.

## Done when

* A layer can be built with nobody seeing it, then faded in.
