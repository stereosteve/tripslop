# 4. Audio reactivity

Visuals that respond to the music. Right now everything is driven by the BPM clock, so
nothing reacts to the music itself.

## Plan

**Input**
- `cpal` for the default input device (a mic or a loopback like BlackHole). Pick the device
  in the top bar.
- An audio thread fills a ring buffer. Each tick, the UI thread takes the latest window,
  runs an FFT (`realfft` or `rustfft`), and computes a few smoothed values:
  - `level`: overall RMS
  - `bass`, `mid`, `high`: band energies
  - `kick`: a simple onset detector on the bass band, giving a 0..1 envelope that decays
- Each value has attack/release smoothing and auto-gain, so it sits in 0..1 without
  constant tweaking.

**Using it**
- **Automation:** add an *Audio* source next to the waveform shapes in the `~` panel. Pick
  band + depth + polarity, the same as the existing modulators (`src/modulation.rs`).
- **Shaders:** expose the values as `iAudio` (`vec4`: level, bass, mid, high) and `iKick`
  in GLSL, `inputs.audio` / `inputs.kick` in WGSL, and as an ISF audio input where that's easy.
  Full ISF `audioFFT` image inputs can come later.
- A small meter in the top bar showing the four bands.

**Nice-to-haves (later)**
- Auto-BPM from kick onsets as an alternative to tap tempo.
- Kick-triggered punch pads (for example, *Strobe* firing on each kick).

## Scripts and tests

Fixed-step scripts must stay deterministic, so in fixed-step mode audio comes from an
`audio FILE.wav` script command or is silent. Unit-test the band and onset maths on
generated sine/click buffers.

## Done when

- A sine-modulated param can be switched to *Audio → bass* and pumps with a kick drum.
- A Shadertoy shader using `iAudio.y` reacts live.
