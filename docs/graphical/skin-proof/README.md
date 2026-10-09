# First native skin proof

Generate from the AMP checkout with its KIT skin foundation dependency:

```sh
nix develop -c cargo run --example skin-player -- docs/graphical/skin-proof
```

`approved-player.png` is the unscaled crop from the approved SVG study.
`player-*` are actual Rust nine-slice + native compositor output; they are not
SVG screenshots. `overlay.png` and `difference.png` compare outline output
against the reference at identical dimensions. Fonts use KIT's explicitly selected
bundled Liberation Mono. This is recorded rather than implicitly
substituting the host terminal's font.

## Status

Implemented: independent frame/well/button assets, tiled nine-slice rendering,
custom bitmap control-lettering trial, native outline text on a skin bitmap,
900/1352/1800-pixel fluid-width captures and reference comparison artifacts.

Remaining discrepancies: bevel thickness and edge placement, text baselines,
transport icon shapes, subpixel clock geometry and bitmap label weight.
Remaining acceptance: Mac Retina review, authenticated SSH, final visual approval
and theme recoloring. Linux Kitty interaction and 2x composition are verified. Audio/state/actions and embedded AMP are
not migrated. The installed player remains untouched.

## Interactive proof

```sh
nix develop -c cargo run --release --example skin-player -- --kitty
```

Runs inside the current Kitty terminal, with no extra application window.
It is an isolated sample: buttons update demonstration state; no audio plays.
Tab/Shift-Tab and Enter exercise the same controls as mouse clicks. Seek and
volume support dragging and arrow keys. `c` switches chrome, `b` label fonts,
`r` rigid/rounded corners, `d` 1x/2x density, `u` Unicode and `q` exits.
Pixel-free PTYs use KIT's measured cell size from terminal negotiation.

The default proof now follows the user's revised direction: a calmer FOLD-like
rack, flat rounded chrome and continuous analyzer bars without an inactive grid.
The earlier Classic treatment remains the preserved reference; the calmer proof
is an experiment awaiting review, not an accepted replacement target.
Font-size changes await clarification; current sizes are intentionally held.
The proof pins Liberation Mono explicitly for reproducible outline typography.

Linux Kitty interaction and resize checks pass in an isolated Xvfb display.
See `scripts/visual/skin_kitty.py`; it records actual presentation timings and
screenshots. Mac Retina and authenticated SSH remain unverified. Loopback SSH
login on this machine rejected authentication; no SSH configuration was changed.
