# 8. UI redesign: "Plum Console"

The features are in good shape, but the window still looks like a default egui debug UI. Today
it's a flat row of text buttons, a grid that takes the full width, then three columns of long
vertical slider lists (Layer/Composition tab, Clip, Library). Nothing tells you what is live,
what you're editing, or what's about to happen.

Mockups (lo-fi and hi-fi): https://claude.ai/artifact/RVXMgqBfFJz1TyvMiknSs5

Where the ideas come from:
* **Bitwig:** the horizontal device chain in a bottom panel, a browser docked on the left, and
  collapsible device cards.
* **Ableton:** the clip launcher as the home screen, track headers with mixer controls, and
  scene launch buttons.
* **Pigments:** knobs with a modulation ring, a colour reserved for modulation, and a list of
  every modulator.

## Layout

```
┌ transport: logo │ ▶ BPM ÷2 ×2 TAP ●○○○ bar.beat Launch▾ │      │ audio chip  MIDI chip │ REC 📷 ⧉ │ Session|Perform ┐
├ browser ┬ clip launcher (layers × scenes)               ┬ output monitor                        ┤
│ search  │ track header: colour, name, M S,              │ crossfader + curve + master            │
│ Gen/FX/ │   blend ▾, A|B, opacity                        │ audio-in spectrum, bands, kick         │
│ Files   │ cells: 16:9 thumb + name strip                │ 4×4 punch pads                         │
├─────────┴───────────────────────────────────────────────┤ (right rail spans to the status bar)   │
│ device panel: [Devices|Modulators|Code]                  │                                        │
│ Source → FX → FX → + → Layer out                         │                                        │
└ status: hover help · automation summary · GPU MB · fps ──────────────────────────────────────────┘
```

* **Transport** gets groups and icons instead of "Record (Cmd R)" labels (the shortcuts move to
  tooltips). Audio and MIDI become status chips: device name plus activity. The status line
  moves to the bottom bar.
* **Browser** (left, toggle with `B`): the current Library panel moved from the right, with a
  category *list* instead of wrapped chips. A *Files* tab is a new feature (left for later).
* **Launcher**: the old Layer tab's top section (opacity, blend, crossfader side, bypass/solo)
  moves into the track header, so you can mix without changing selection. Each layer gets a
  colour (8-colour palette) that's used for its header stripe, playing cells, device-panel
  accents and Perform channels. Scenes get optional names.
* **Device panel** (bottom): replaces the Layer/Composition tab and the Clip column. It shows
  the selection's chain left → right:
  * **Source**: the selected clip (loop mode, direction, BPM sync, fit, transition) or the
    generator/shader params.
  * The effects, as cards with knob grids; double-click a header to collapse it.
  * **Layer out**: opacity, blend, transform.

  Selecting the master row shows the master chain plus the Output and Crossfade settings.
  Tabs: *Devices*, *Modulators* (the old automation list, with a live mini-plot for each row),
  and *Code* (the shader editor, docked here instead of a floating window).
* **Right rail**: everything you touch while playing, always visible: the monitor, the
  crossfader, audio-in and the pads.
* **Perform view** (a view switch, not fullscreen): a large output, big scene buttons with
  thumbnails, layer channels (fader, M/S, the current clip), large pads and the crossfader.
  Cmd/Ctrl+F still gives fullscreen output only.

## Visual language

* Background shades tinted from the logo plum: ground `#0E0912`, panel `#160F1C`, raised
  `#1F1627`, control `#2A1F34`, line `#34283F`; text `#E9DFCC`, muted `#A79CB0`.
* One colour, one meaning:
  * lime `#D9FF58`: live / held / current scene (from the logo)
  * pink `#FF78DC` (today's `ACCENT`): automation only
  * amber `#FFB547`: queued
  * red `#FF5A5F`: recording / errors
  * cyan `#5EE6FF`: audio
* Type: Barlow Semi Condensed (UI) and JetBrains Mono (values, BPM). Both are OFL, and they're
  embedded with `include_bytes!` into `FontDefinitions`.
* Radius 4 on controls, 6 on cards, 8 on panels. 1 px lines. Shadows only on popovers and lit
  pads.

## Widgets to build (`src/ui/widgets.rs`)

* **`knob`**: drag vertically (Shift for fine), double-click to reset, scroll to nudge.
  Right-click opens the existing menu (automate / learn MIDI / forget). It draws a value arc
  (unipolar, or from 12 o'clock for bipolar params), a pink outer arc for the modulation
  range with a dot at the live value, and a small **M** badge when mapped. Choice params stay
  dropdowns or segments. `param()` picks the knob in device cards and keeps the slider where
  there's room for it (track headers, crossfader).
* **`device_card`**: header (bypass LED, name, preset, menu), knob grid, collapse. Drag it to
  reorder.
* **`clip_cell`**: one painter function with a state for each look: empty, loaded, playing,
  queued, importing, error, selected, drop target.
* **`pad`**: idle, held (with a hold-time bar), latched and MIDI-learn.
* `theme.rs`: the colour constants plus `apply(ctx)`, which sets `Visuals`, `Spacing` and
  fonts once at startup.

## Steps

Each step should be usable on its own.

1. **Theme**: `theme.rs`, fonts and visuals. The existing layout immediately looks better.
2. **Knob widget**, used in effect sections first, behind the existing `param()` API.
3. **Layout shuffle**: browser on the left, right rail, status bar, grouped transport.
4. **Track headers + layer colours**: move the layer controls out of the Layer tab.
5. **Device panel**: Source / FX / Layer out cards. Delete the Layer and Clip panels. The
   master row selects the master chain.
6. **Modulators and Code tabs**: the automation list, and the docked shader editor.
7. **Perform view**.

Update the `screenshot` scripts after each step (`scripts/smoke.tripslop` already saves
`ui.png`), so changes to the look show up in review.

## Not now

* Shared modulators (Pigments-style LFO1 driving many params). The per-param modulators stay;
  the Modulators tab just lists them.
* User themes / light mode.
* Docking or rearranging panels freely. Fixed regions with resize handles are enough.
