# 19. Saving sets ✅

tripslop couldn't save what you'd built, and the demo was a hundred lines of Rust. Sets fix
both: a set file holds a whole composition, and the demos are now sets, so there can be
several, each with its own look, behind a welcome screen.

## What was built

The format is described in [../sets.md](../sets.md).

* **One JSON file, not a folder.** The first plan was a `.tripset` folder with `set.json`,
  thumbnails and a screenshot. A single file is easier to write by hand, diff, mail and bundle
  into the app (`build.rs` compiles in everything in `sets/`). Media is referenced by path
  relative to the set; a folder holding a set and its collected media can come later
  (*Collect media*, below) without changing the format.
* **Sparse and hand-writable.** Only what differs from a fresh composition is written.
  Parameters go by label (prefix match, like scripts), choice parameters by option name, enums
  in snake case. Feedback presets can be named instead of spelled out. Unknown effects,
  parameters, options and missing media are skipped and reported in the status bar and on
  stderr, never fatal.
* **Fallbacks.** A clip can name another clip to use when it can't open. The demos use this so
  the browser build (no video) gets stills or generators instead of the footage.
* **Compiled-in media.** The two sample stills and the six logos are compiled in and can be
  named as `builtin:PATH`, so the demos work from any folder and in the browser.
* **Scene names** are now part of the composition: the grid's scene buttons and Perform's
  scene pads show them.
* **Shaders are saved inline**, so edits in the Code tab survive. Their parameters don't exist
  until they compile, so saved values wait in `CustomShader::initial` / `initial_mods`.
* **UI:** the set menu at the top left shows the set's name (editable, with a description) and
  has New, Open (Cmd/Ctrl+O), Save (Cmd/Ctrl+S), Save as (Shift+Cmd/Ctrl+S) and the demo
  sets. The PNG snapshot moved to Cmd/Ctrl+E. The window title shows the set's name. A
  `.tripset` on the command line opens it; `--set NAME` opens a demo.
* **Welcome screen:** shown when tripslop starts with nothing to open, and always in the
  browser. A card per demo set with a picture, its description and its scenes; number keys
  pick one. The first demo plays behind it.
* **Six demo sets** in `sets/`: Public Access (the old demo), Patch Bay, Deep Field, Warehouse,
  Showroom and Lava Lamp. `scripts/bake-sets.sh` renders their welcome pictures into
  `sets/thumbs/`.
* **Scripts:** `open NAME|FILE`, `save FILE`, `snapshot PATH WIDTH` (`.jpg` writes a JPEG),
  `tab welcome`.

* **Cost.** The first Lava Lamp spent 686 ms a frame in one shader (*Lava*). `--bake-library`
  now times every bundled shader (`ms` in `baked.json`), the browser marks slow ones, a unit
  test keeps them out of the demos, and the `frame-ms` script query measures a whole scene
  (`scripts/sets-cost.tripslop`).

Unit tests cover a round trip (open, save, open again: the same values and modulators, and
saving twice gives the same file), unknown things being reported rather than fatal, relative
paths, and every bundled set opening without a warning. `scripts/smoke.tripslop` runs on the
Public Access set unchanged.

## Not done yet

* **Unsaved-changes dot** in the title, and asking before New / Open throws work away.
* **Recent sets** list.
* **Collect media:** copy referenced files next to the set so it can move machines.
* **Missing media UI:** a red outline on cells whose file is gone, and *Locate…* that searches
  that folder for the others.
* **Browser:** saving a set as a download and opening one by dropping it on the page.
* Snapshots (12) and macros (11) go in the format when they exist.
