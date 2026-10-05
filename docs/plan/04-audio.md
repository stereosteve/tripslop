# 4. Audio reactivity

**Status: done.** `src/audio.rs`: a `cpal` input, or a WAV file read at the simulation clock
(`audio FILE.wav` in scripts), analysed each tick into level / bass / mid / high / kick /
centroid, plus a 512-band log spectrum and a waveform. The **Audio** automation shape follows
a band. Shaders get `iAudio` / `iKick` (GLSL), `inputs.audio` / `inputs.kick` (WGSL), and the
spectrum and waveform as textures on bindings 6 and 7. The library's ghost-arcade audio names
(`audioBass`, `audioBeat`, `sampleFFT`, …) now read the real input, which covers the plan's
"ISF audio input" and the `audioFFT` part of "later". Picking an input and the meter are in
the top bar. The analyzer is unit-tested on sines, silence and a kick pattern, and
`scripts/audio.tripslop` runs the generated `samples/beat.wav` through automation and
*Spectral Aurora*. The live-device path was only checked as far as listing devices. Opening
one needs microphone permission, so try it by hand.

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
