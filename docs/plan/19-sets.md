# 19. Saving sets

tripslop can't save what you've built. Scripts can rebuild a set, but nobody writes a script
after a good jam. Arkestra's `.vfx` project is a folder holding a JSON document, a screenshot and
thumbnails. Snapshots (12), macros (11) and sequences (10) make the problem bigger, because
they're exactly the kind of work you want to keep.

This is infrastructure, which the ground rules say should only go in when something is blocking.
Item 12 is what makes it blocking.

## Plan

* **Format:** a `.tripset` folder with `set.json` (versioned), `thumbs/` and a `screenshot.png`.
  Media is referenced by path, both relative to the set and absolute; shaders and generators
  are stored inline. A *Collect media* option copies the referenced files into `media/` inside
  the set, so it can be moved to another machine.
* **What's in it:** layers (name, colour, blend, side, opacity, transform, transition, mute and
  solo), clips (source, loop mode, direction, speed, sync, fit, the parameters), effects (kind,
  enabled, parameters, custom shader code, model reference), every parameter's value and
  modulator, macros, shared modulators, snapshots, scene names, tempo, quantize, crossfader
  mode, output size. Not in it: MIDI and OSC mappings (they belong to the controller), and
  live state like cue, held pads and the playing position.
* **Serialisation:** `serde` derive on the plain data types. Effects and parameters are stored by
  label, not index, so adding parameters to an effect later doesn't break old sets. Unknown
  labels are dropped with a warning in the status bar. Videos are re-imported (transcoded) on
  load, in the background, with the cells showing progress.
* **UI:** *Save* (Cmd/Ctrl+S, moving snapshot PNG to Cmd/Ctrl+Shift+S), *Save as…*, *Open…*
  (Cmd/Ctrl+O), and a recent-sets list. The window title shows the set name, with a dot for
  unsaved changes. Opening a `.tripset` folder from the command line loads it.
* **Missing media:** cells whose file is gone show a red outline. *Locate…* on one searches
  that folder for the other missing files too.
* **Scripts:** `save PATH` / `open PATH`.

## Scripts and tests

* Round trip: build the demo set, change things, save, load, and compare `visit_paths` values
  and a snapshot image.
* Loading a set with an unknown effect parameter keeps the rest.
* `scripts/sets.tripslop` saves and reloads mid-script.

## Done when

* Quit, relaunch, open the set, and everything is where you left it, snapshots included.
