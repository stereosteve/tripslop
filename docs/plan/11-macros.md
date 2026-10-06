# 11. Macros and shared modulators

Each parameter has its own private modulator, so three knobs that should breathe together need
three LFOs set up by hand, and they drift apart as soon as you change one. And there's no way to
make one big "intensity" knob for a look. Arkestra solves both with chain macros (named knobs
that drive many parameters at once) and global LFOs and sequencers that any parameter can follow.

## Plan

Both fit the existing `Modulator` as new signal sources, so depth, polarity, smoothing and the
modulation ring on the knob keep working unchanged:

* **Macros.** Every layer gets 8 macro knobs and the master gets 8 more, each with a name.
  They sit in a strip in the Layer out card and in the layer's Perform channel.
  * A new `Shape::Macro(n)` makes a parameter follow macro *n* of its own layer (or the master's
    macro for master effects), scaled by depth and polarity like any other signal.
  * Right-click a knob → *Map to macro* → pick one, or *New macro*. The new macro starts at the
    knob's current position, so nothing jumps.
  * The macros are `Param`s themselves, so they can be MIDI-learned, automated (an LFO on a
    macro moves everything mapped to it), and set by scripts (`set 2/macro1 0.7`).
* **Shared modulators.** The composition keeps a list of named modulators: LFOs, sequencers,
  envelopes and audio followers, using the same `Modulator` type.
  * `Shape::Shared(id)` makes a parameter follow one. Right-click → *Automate…* gets a *Follow:*
    menu listing the shared modulators, and *Share this* promotes a knob's own modulator to the
    shared list.
  * Each mapping keeps its own **phase offset**, so X and Y can follow the same LFO 90° apart and
    trace a circle.
  * The Modulators tab gets a *Shared* section at the top, with each shared modulator's plot and
    how many parameters follow it. Click it to list them.
* A parameter still has a single mapping. Stacking several sources on one knob (Arkestra sums
  them) waits until someone actually needs it.

## Scripts and tests

* Unit tests: a macro at 0, 0.5 and 1 against depth and polarity, and two parameters on one
  shared LFO with a quarter-cycle phase offset.
* `scripts/macros.tripslop`: map hue and zoom on two effects to one macro, sweep it, and assert
  both move.

## Done when

* One macro knob takes a layer from calm to wild across several effects.
* Changing the rate of one shared LFO changes it for every knob that follows it.
