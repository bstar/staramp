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
and real player bindings. Linux Kitty interaction, 2x composition and all built-in theme masks are verified. Audio/state/actions and embedded AMP are
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
`Alt+T` cycles the existing built-in themes and briefly displays their names.
Pixel-free PTYs use KIT's measured cell size from terminal negotiation.

The default proof follows Classic Stacked Rack again, with beveled chrome and
LED analyzer detail. `c` selects the separate flat FOLD-like experiment.
The user explicitly selected option 01 on October 9. This isolated proof has
not passed visual review. Compare the actual shared player separately at
`../treatments/player-comparison.html`; font sizes still need review.
The proof pins Liberation Mono explicitly for reproducible outline typography.

Linux Kitty interaction and resize checks pass in an isolated Xvfb display.
See `scripts/visual/skin_kitty.py`; it records actual presentation timings and
screenshots. Mac Retina and authenticated SSH remain unverified. Loopback SSH
login on this machine rejected authentication; no SSH configuration was changed.

Theme comparison: `Alt+T` cycles the sixteen built-ins plus the original reference
palette, displaying a temporary footer notice. `themes.html` shows the exports.
Palette masks are composited before text rasterization; no whole-frame recoloring.
