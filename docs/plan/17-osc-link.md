# 17. OSC and Ableton Link

MIDI covers hardware. Software controllers (TouchOSC, Max, a phone) and, through the
[TDLiDAR](https://tdlidar.github.io/) app, ~40 iPhone sensors (motion, attitude, face and hand
tracking, audio, touch) all speak **OSC**. TDLiDAR sends it over UDP port 9000. **Ableton Link**
is how a laptop VJ stays in time with the musicians' laptops without a MIDI cable.

## Plan

**OSC in** (`rosc`, a UDP socket on a background thread like `midi.rs`):
* Off by default. Pick a port in a new **OSC** chip in the transport (default 9000, so TDLiDAR
  works out of the box). The chip blinks on incoming messages and lists the last few addresses.
* **OSC learn**, next to MIDI learn in the knob's right-click menu: move the control on the
  sender and the address that changed most in that moment is bound (an iPhone sends dozens at
  once). The first float, int or bool argument is used.
* **Range:** each mapping has an input min and max (0..1 by default) mapped onto the knob's
  range. Learn records the range it saw, so −1..1 attitude values just work.
* Messages can also hit pads (`> 0.5` = down) and scenes, the same targets as MIDI.
* Mappings are saved with the MIDI ones (`midi.json` becomes `control.json`, still reading the
  old file) by parameter path.
* Script command `osc /addr 0.5` for tests.

**Ableton Link** (the `rusty_link` crate, which builds Link's C++ with cmake; behind a cargo
feature if the build is a problem on some platform):
* A **Link** toggle in the transport, with the peer count. When it's on, Link sets the tempo and
  the beat phase (quantum 4, so bars line up with other Link apps). Tap tempo and the BPM field
  change the session tempo for everyone.
* If both MIDI clock and Link are on, MIDI clock wins. That's the order Arkestra uses, and it
  gives one obvious owner of the tempo.

## Scripts and tests

* Unit tests: OSC message parsing and address matching, and range scaling.
* `scripts/osc.tripslop` injects messages and asserts on parameters. Link gets a unit test
  around the beat/phase mapping only; the network part needs two machines and is tested by
  hand.

## Done when

* Tilting a phone running TDLiDAR rotates the feedback tunnel.
* tripslop and Ableton Live share a tempo and their downbeats line up.
