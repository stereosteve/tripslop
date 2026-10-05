# Bundled 3D models

The models the library's *Models* tab shows. `build.rs` bakes every file here into the binary,
so they load the same way everywhere and are never read from disk. `library.json` gives each
one its name, category, description and credit; a model has to be listed there to show up.
Entries with `shape` instead of `file` are generated in code (`src/model/shapes.rs`) and cost
nothing in the binary.

To add a model: put the file here (with its `.mtl` and textures, in a subfolder if it has
any), list it in `library.json`, and run `cargo test model` (it loads every bundled file).
Keep them small: a few tens of thousands of triangles is plenty on a projector.

## Where they come from

Fetched on 2026-10-05.

| File | Source | License |
| --- | --- | --- |
| `teapot.obj` | [common-3d-test-models](https://github.com/alecjacobson/common-3d-test-models) (`teapot.obj`), from Martin Newell's 1975 data | Freely distributed since 1975; no license attached |
| `stanford-bunny.ply` | [Stanford 3D Scanning Repository](http://graphics.stanford.edu/data/3Dscanrep/) via common-3d-test-models; simplified from 69,451 to 20,000 triangles | Stanford terms (below) |
| `stanford-dragon.ply` | Stanford 3D Scanning Repository (XYZ RGB dragon) via common-3d-test-models; simplified from 249,882 to 40,000 triangles | Stanford terms (below) |
| `armadillo.ply` | Stanford 3D Scanning Repository via common-3d-test-models; simplified from 99,976 to 30,000 triangles | Stanford terms (below) |
| `happy-buddha.ply` | Stanford 3D Scanning Repository via common-3d-test-models; simplified from 98,601 to 40,000 triangles | Stanford terms (below) |
| `suzanne.obj` | common-3d-test-models (`suzanne.obj`), Blender's built-in monkey | Blender Foundation |
| `spot/` | [Keenan's 3D Model Repository](https://www.cs.cmu.edu/~kmcrane/Projects/ModelRepository/) (`spot_triangulated.obj`, `spot_texture.png`) | CC0 |
| `bob/` | Keenan's 3D Model Repository (`bob_tri.obj`, `bob_diffuse.png`, texture halved to 1024²) | CC0 |
| `blub/` | Keenan's 3D Model Repository (`blub_triangulated.obj`, `blub_texture.png`, texture halved to 1024²) | CC0 |
| `fox.glb` | [Khronos glTF Sample Assets](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox) (`Fox.glb`) | Model CC0 (PixelMannen); rigging and animation CC BY 4.0 (tomkranis); glTF conversion CC BY 4.0 (@AsoboStudio, @scurest) |

The scans were simplified with quadric decimation (`fast-simplification`) and written as
binary PLY. The OBJ files were rewritten with five decimals and without normals (tripslop
computes them), and the three textured ones got a small `.mtl` naming their texture.

**Stanford terms.** The Stanford Computer Graphics Laboratory lets anyone use, mirror and
redistribute these models for free, with credit, but not for commercial purposes or in a
product for sale without their permission. tripslop is free, and credits them here and in the
library. If you build something commercial on tripslop, remove these four files (and their
entries in `library.json`) or ask Stanford.
