# 14. LUTs

The Color effect does hue, saturation and contrast, but not a *look*. Colourists and every LUT
pack on the internet use `.cube` files, and a LUT costs one texture lookup however complex the
grade is. Arkestra ships a LUT effect with 8 starter looks and a folder to drop more into.

## Plan

* A **LUT** effect (category Color): *look* (a dropdown) and *amount*. The `.cube` parser handles
  `LUT_3D_SIZE`, `DOMAIN_MIN` / `DOMAIN_MAX` and comments; 1D LUTs are refused with a message.
  The table becomes an `Rgba16Float` 3D texture sampled with trilinear filtering.
* `EffectDef` grows an optional extra texture binding for this. The header gets
  `fn lut(c: vec3f) -> vec3f`, used only by this shader.
* **Looks:** about 8, *generated* by a small Rust function at build time rather than copied from
  anywhere: warm film, teal & orange, bleach bypass, cross-process, night, mono contrast,
  faded matte, neon. The function writes `.cube` text, so they're also examples of the format.
* **Your own:** `.cube` files in `luts/` under the config dir, plus `--lut-dir DIR`, are added to
  the list. Dropping a `.cube` file on an effect card or a layer adds a LUT effect with it.
* The look is a choice parameter, so it can be sequenced to switch grades on the bar.

## Scripts and tests

A unit test parses a hand-written 2×2×2 cube and checks that the identity LUT is a no-op on a
test image (within 1/255). `scripts/lut.tripslop` snapshots each bundled look on the demo set.

## Done when

* A `.cube` from Resolve grades the output the way Resolve does.
