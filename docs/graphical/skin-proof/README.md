# First native skin proof

Generate from the AMP checkout with its KIT skin foundation dependency:

```sh
nix develop -c cargo run --example skin-player -- docs/graphical/skin-proof
```

`approved-player.png` is the unscaled crop from the approved SVG study.
`player-*` are actual Rust nine-slice + native compositor output; they are not
SVG screenshots. `overlay.png` and `difference.png` compare outline output
against the reference at identical dimensions. Fonts currently use KIT's
bundled Liberation Mono fallback. This is recorded rather than implicitly
substituting the host terminal's font.

## Status

Implemented: independent frame/well/button assets, tiled nine-slice rendering,
custom bitmap control-lettering trial, native outline text on a skin bitmap,
900/1352/1800-pixel fluid-width captures and reference comparison artifacts.

Remaining discrepancies: bevel thickness and edge placement, text baselines,
transport icon shapes, subpixel clock geometry and bitmap label weight.
Remaining acceptance: interactive Kitty proof, 2x rendering, Linux/Mac review,
pointer states and theme recoloring. Audio/state/actions and embedded AMP are
not migrated. The installed player remains untouched.
