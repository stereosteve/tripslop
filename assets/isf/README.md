# Bundled ISF shaders

The shaders the *Library* panel shows. `build.rs` bakes every `.fs` / `.vs` file here into
the binary, so the library works the same everywhere and never reads these files at runtime.

`library.json` is the index. For each file it gives the display name, the category, optional
search tags and tuned starting values for parameters (`defaults`). `categories` sets the order
of the category chips. A `.fs` file that isn't listed still shows up, with its stem as its name
and the first useful entry of its ISF `CATEGORIES` as its category.

To add a shader, drop the `.fs` in here (subfolders are fine), add an entry to `library.json`,
and run `cargo test isf` (this checks that the index and the files agree) and
`cargo test --release isf_report -- --ignored --nocapture` (this shows what compiles).

## Where they come from

Copied on 2026-10-05 from [ghost-arcade](https://github.com/riskcapital/ghost-arcade) at commit
`5f402f7`:

* `*.fs`: `public/ISF/*.fs`, the library ghost-arcade ships
* `cube/`: `public/ISF/cube shaders/`
* `user/`: `user-shaders/` (three AI-generated shaders)

Other copies elsewhere in that repo (`ISF/`, `CuratedISF/`) are older versions of these and
weren't taken. The `.txt` drafts in `public/ISF` weren't taken either.

The names and categories in `library.json` are tripslop's own. Its `tags` and `defaults` come
from ghost-arcade's `public/ISF/manifest.json`.

## Licensing

ghost-arcade is licensed **AGPL-3.0-only**. The shaders keep their own `CREDIT` lines. Most
credit Ghost Arcade or its author; others credit Shrink Wrap, mojovideotech, and AI tools.
`AnotherGridThingy.fs` and `InnerDimensionalMatrix.fs` say **CC BY-NC-SA 3.0**
(non-commercial). Check those terms before distributing a tripslop build that includes this
folder.
