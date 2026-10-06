# 9. Dry/wet on every effect

Every effect is all or nothing today. To use a little kaleidoscope you have to find a knob in
that particular effect that happens to soften it, and most don't have one. Arkestra puts a
**Mix** slider and a **blend mode** on every device, and that's most of what makes stacking
effects feel like an audio FX send.

## Plan

* Each `Effect` gets two more parameters after its own: **wet** (0..1, default 1) and
  **wet blend** (the layer blend modes: Normal, Add, Screen, …, default Normal). Because they're
  ordinary `Param`s, they can be automated, MIDI-learned and set from scripts
  (`set 2/fx1/wet 0.5`) with no extra work. They aren't passed to the effect's shader.
* The renderer only does extra work when it's needed: at wet 1 with Normal, the effect writes
  straight into the next buffer as today. Otherwise it renders into a shared scratch texture, and
  one composite pass mixes that wet picture with the dry input into the next buffer:
  * **Normal** is a straight dissolve from dry to wet (composite mode 9), which suits
    effects that change the whole frame.
  * The other modes lay the wet picture over the dry one at *wet* opacity, the same way a
    layer lands on the composition. Add with a blurred or edge-detected copy gives a glow.
* An effect's history (feedback, delays) keeps recording its own wet output, so a feedback loop
  at wet 0.3 still builds up its full trails. Only what you see is mixed.
* At wet 0 the effect still runs, so its history stays warm and you can fade it back in. Bypass
  (the card's on/off) still skips it entirely.
* **UI:** the wet knob and blend dropdown go in a strip along the bottom of every effect card,
  so they're in the same place on every card. The card header dims with the wet amount.
* Punch-in effects and custom shader effects get them too.

## Scripts and tests

`scripts/dry-wet.tripslop`: a layer with Kaleidoscope at wet 0, 0.5 and 1, and Edges in Add at
0.5, with snapshots and asserts that the halfway picture differs from both ends. A unit test
checks that the two parameters appear in `visit_paths` as `fx1/wet` and `fx1/wet blend`.

## Done when

* A kaleidoscope can be faded in on an LFO without touching its other knobs.
* Edges in Add mode gives a glow on top of the clip.
* Feedback at low wet keeps its full trails.
