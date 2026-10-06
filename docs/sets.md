# The `.tripset` format

A set is a whole composition in one JSON file: layers, clips, effects, every parameter and its
modulator, scene names, tempo. tripslop writes it with **Save** (Cmd/Ctrl+S) and reads it with
**Open** (Cmd/Ctrl+O), `tripslop FILE.tripset`, or the `open` script command. The demo sets in
`sets/` are written by hand in the same format and compiled into the app.

The file only needs what differs from a fresh composition, so a hand-written set stays short:

```json
{
  "tripslop": 1,
  "name": "Two scenes",
  "bpm": 120,
  "scenes": ["Rings", "Plasma"],
  "effects": [ { "kind": "crt", "params": { "noise": 0.2 } } ],
  "layers": [
    {
      "name": "Base",
      "effects": [ { "kind": "feedback", "preset": "Tunnel" } ],
      "clips": [ { "generator": "rings" }, { "generator": "plasma" } ]
    },
    {
      "name": "Logo",
      "blend": "screen",
      "params": { "opacity": { "value": 0.8, "mod": { "shape": "sine", "beats": 8, "depth": 0.3 } } },
      "clips": [ { "file": "logo.png" }, null ]
    }
  ]
}
```

## The set

| Field | |
| --- | --- |
| `tripslop` | format version (1). A newer set still opens, with a warning |
| `name`, `description` | shown in the title bar, the set menu and the welcome screen |
| `bpm` | tempo (default 120) |
| `quantize` | when launches happen: `off`, `beat` or `bar` (default `off`) |
| `crossfade`, `fade_curve` | `bank` / `layer`; `linear` / `smooth` / `cut` |
| `columns` | at least this many scenes (default 8; there are always enough for the names and clips) |
| `scenes` | scene names, left to right |
| `start` | the scene launched on opening, 1-based; `0` for none (default 1) |
| `params` | `master` and `crossfader` |
| `effects` | the master chain, first to last |
| `layers` | **bottom layer first**, so the first is layer 1 in scripts and the UI |

## Layers

| Field | |
| --- | --- |
| `name` | |
| `color` | index into the UI's eight layer colors |
| `blend` | `normal`, `add`, `screen`, `multiply`, `difference`, `lighten`, `darken`, `overlay`, `subtract` |
| `side` | crossfader side: `a`, `b` or `both` |
| `bypass`, `solo` | `true` / `false` |
| `params` | `opacity`, `transition`, `position x`, `position y`, `scale`, `rotation` |
| `effects` | the layer's chain, first to last |
| `clips` | one per scene, `null` for an empty cell |

## Clips

A clip has exactly one source:

| Source | |
| --- | --- |
| `file` | a video, image, SVG or shader file, relative to the set's folder (absolute works too). `builtin:PATH` names media compiled into the app: `builtin:samples/crab-nebula.jpg`, `builtin:samples/pillars-of-creation.jpg`, and the six logos in `logos/` (`builtin:logos/tripslop-flower-color.svg`, …) |
| `generator` | `bars`, `rings`, `plasma`, `checker`, `dot`, `noise`, `solid` |
| `library` | a generator from the shader library, by name (`Lava`), `category/name` when names repeat (`Neon & Electric/Digital Rain`), or key |
| `shader` | `{ "name": …, "code": …, "vertex": … }`: GLSL / ISF / WGSL kept in the set. Saving always writes shaders this way, so edits survive |
| `model` | a 3D model from the library, by name (`Utah Teapot`) or key |
| `camera` | a camera's index (desktop only) |

Then, all optional: `name`; `fill` (an SVG's size in the frame, 0..1, default 0.9); `loop`
(`loop`, `bounce`, `random`, `play_once`, `play_once_hold`); `direction` (`forward`,
`reverse`, `paused`); `sync` (`timeline`, `bpm`); `fit` (`fill`, `contain`, `stretch`); `params`
(`speed`, `length (beats)`, and the generator's, shader's or model's own); and `fallback`, a
clip to use instead when this one can't be opened: a video in the browser, or a missing file.

## Effects

| Field | |
| --- | --- |
| `kind` | `feedback`, `echo`, `rgb_split`, `kaleidoscope`, `mirror`, `transform`, `shape_projector`, `projection_mapping`, `wave`, `color`, `luma_key`, `pixelate`, `blur`, `edges`, `crt`, `strobe`, or `shader` |
| `shader`, `library`, `file` | the code of a `shader` effect (which `kind` can then leave out): inline, from the library by name, or a file |
| `preset` | a Feedback preset by name (`Tunnel`, `Sierpinski`, `Mandala`, `Slow trails`, `Spiral galaxy`, `Hall of mirrors`, `Melt`), applied before `params` |
| `model` | the Shape projector's / Projection mapping's library model (sets the shape to *Model* unless `params` says otherwise) |
| `enabled` | `false` to switch it off |
| `half_history` | keep delay / feedback history at half size |
| `params` | its parameters, plus `wet` and `wet blend` |

## Parameters

`params` maps a parameter's label to its value. Labels are the ones the UI shows, matched
without regard to case, and a prefix is enough (`rotate` finds `rotate °`). A value is:

* a number: `"scale": 0.42`
* a choice parameter's option, by name: `"tile": "Repeat"`, `"mode": "Add"`
* or `{ "value": …, "mod": { … } }` for a modulator, where `value` can be left out

A modulator's fields, all optional: `shape` (`sine`, `triangle`, `saw_up`, `saw_down`,
`square`, `sample_hold`, `smooth_random`, `envelope`, `audio`), `beats` (cycle length, default
8) or `hz` (free-running), `depth` (fraction of the parameter's range, default 0.25),
`polarity` (`bipolar`, `up`, `down`), `phase` (cycles), `width` (square duty cycle), `points`
(the envelope, `[[position, level], …]`), `band` (`level`, `bass`, `mid`, `high`, `kick`, for
`audio`), `enabled`.

A shader's parameters only exist once it has compiled, a moment after opening, so its values
are held until then (choices by name don't work there: use the option's number).

## Keep the demos cheap

The bundled sets have to play smoothly for someone trying tripslop on a laptop. A unit test
(`bundled_sets_use_cheap_shaders`) fails if a demo uses a library shader slower than
`isf_library::SLOW_MS` (6 ms per 720p frame, timed by `--bake-library`) or puts more than
10 ms of library shaders in one scene. `scripts/sets-cost.tripslop` measures each whole scene,
built-in effects included; on the machine they were made on, every scene is under 6 ms.

## What's not in it

MIDI mappings (they belong to the controller, and live in the config folder), the output size,
and live state: the beat, playheads, held pads, the crossfader's position mid-move.

## When something's missing

Opening never fails halfway. Media that can't be found, unknown effects, parameters or
options are left out and listed: the status bar shows the first, and every one goes to
stderr. Saving leaves out clips with no file behind them (an image dropped into the browser)
and says so.
