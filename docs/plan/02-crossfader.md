# 2. Bank crossfader

**Status: done**, with one change from the plan below. Unassigned (`Both`) layers keep their
place in the stack and go into *both* banks, instead of being composited on top of the mix.
That way a background layer left unassigned stays underneath, and the result is still
exact: `mix(A + L, B + L) = L + mix(A, B)`. Each layer still renders its clips and effects
once; only the cheap composite pass runs twice. Curves are Linear / Smooth / Cut (an
equal-power curve brightens a video dissolve, so it was left out). There are no saved sets
yet, so bank mode is simply the default, with *Layer opacity* as the alternative.
`scripts/crossfade.tripslop` and the `crossfade` test in `tests/scripts.rs` check the actual
pixels.

## Problem

The A/B crossfader currently scales each layer's opacity (`Composition::side_gain`):

```
A gain = min(1, 2·(1 − x))     B gain = min(1, 2·x)
```

Both sides are at full opacity at the midpoint, and layers are still blended in layer
order. So if an opaque A layer sits above a B layer, B stays hidden until the fader passes
the middle and A starts fading out. It feels like a late cut, not a dissolve. With
additive blending it also brightens at the midpoint.

## Plan

Add a *bank* mode that crossfades between finished images:

1. Composite the A layers into texture `bank_a` and the B layers into `bank_b`, in layer
   order, using the existing composite pass.
2. Dissolve `bank_a` → `bank_b` with a tiny shader: `mix(a, b, curve(x))`.
3. Composite the `Both` layers over the result in their usual order.
4. Master effects and the master fader follow as they do now.

The two extra textures are 720p RGBA8, about 3.5 MB each.

- Keep the current behavior as *layer* mode, as a toggle in the Composition tab. Pick one
  as the default for new sets; existing sets load in layer mode.
- Fader curve: linear, plus an equal-power-ish option. Other transitions (wipe, luma, etc.)
  can wait.
- Effect histories on a side whose gain is 0 keep running, so feedback doesn't restart when
  you fade back to it.

The `Both`-on-top rule changes the stacking order a little compared with layer mode.
That's fine, but document it in the README.

## Done when

- x=0 matches A alone, x=1 matches B alone, and x=0.5 is a 50/50 mix. Test it with a script
  using solid-color generators and `snapshot`, plus a unit test of the curve.
- Punch-ins and feedback carry on during a fade.
