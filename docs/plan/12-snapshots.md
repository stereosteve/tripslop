# 12. Snapshots that morph

Finding a great look by turning ten knobs and then losing it as soon as you turn an eleventh is
the most common way a jam goes wrong. Arkestra's chain snapshots fix that: 8 preset slots per
chain, recalled instantly or crossfaded over a few seconds, stepped through with
prev/next/random, and autoplayed on the bar. The morph is the part to copy: watching every
parameter glide to a new look in time with the music is a performance move in itself.

## Plan

* **Layer snapshots.** Each layer gets 8 slots, and the master gets 8 more. A slot stores, for
  that layer: every parameter's hand-set `value` (the layer's own parameters, its effects'
  parameters including wet and wet blend, and the macros), whether each effect is on, and the
  playing clip's parameters. Effects are matched by id, so reordering cards doesn't break a slot,
  and an effect added after the snapshot was taken just stays as it is.
  * Modulators aren't stored. Whatever is automated keeps moving around its new base value.
* **Recall:** click a slot to recall it. **Morph** (0 to 8 beats, or instant) and a **curve**
  (the existing `FadeCurve`, plus *Cubic*) are per layer. During a morph, continuous parameters
  glide and choices switch halfway. Effects that turn on switch on at the start; effects that turn
  off switch off at the end.
* Moving a knob during a morph takes that knob out of the morph, so it doesn't fight you.
* **Save:** `+` saves into the next empty slot, *Save here* overwrites a slot, and *Clear*
  empties it. Each slot shows a thumbnail of the layer's output at the moment it was saved.
* **Navigation:** ◀ ⤨ ▶ (previous, random, next filled slot), all MIDI-learnable, and
  **autoplay** every 1, 2, 4, 8 or 16 bars, in order or at random.
* **Where:** a slot strip in the Layer out card, and a compact one in each Perform channel.
* **Scenes:** the same ◀ ⤨ ▶ and autoplay for scene launches (`Launch` already quantizes them),
  so a set can play itself.
* **Scripts:** `snapshot-save 2 1`, `snapshot-recall 2 1 4b` (morph over 4 beats).

Snapshots live in memory until sets can be saved (19). Then they're saved with the set.

## Scripts and tests

* Unit tests for the morph: a float at the halfway point, a choice switching at 0.5, effects
  turning on and off at the right ends, and a knob grabbed mid-morph dropping out.
* `scripts/snapshots.tripslop`: save two looks, morph between them over 4 beats, and assert
  values at beat 2 and beat 4. Then autoplay at 1 bar and assert which slot is active.

## Done when

* Two saved looks on the Fractal layer morph into each other in time with the music.
* A set with autoplay on scenes and snapshots keeps evolving with nobody touching it.
