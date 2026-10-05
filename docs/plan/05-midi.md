# 5. MIDI

The punch pads were inspired by hardware, so let them be played on hardware.

## Plan

- `midir` for input; pick the device in the top bar (or *All*).
- **Pads:** note on/off → punch pad down/up, with velocity as an optional intensity. A
  default map from notes 36+ (a common drum-pad layout) to the 16 pads, in the same order
  as the Q–K keys.
- **MIDI learn:** right-click any slider → *Learn MIDI*, then wiggle a knob. CC 0–127 maps
  to the slider's range. Note learn works on pads, scene launches and toggles too.
- **Clock in (optional):** follow MIDI clock for BPM and beat phase, as an alternative to
  tap tempo.
- Save mappings in a small JSON file next to the binary's config, not in sets, since
  mappings belong to the controller, not the set.

## Scripts and tests

A `midi note 36 on` / `midi cc 1 64` script command injects messages without hardware, so
pads and learn can be tested in `scripts/`.

## Done when

- A pad controller can hold and latch punch pads.
- A learned knob drives a feedback parameter, and the mapping survives a restart.
