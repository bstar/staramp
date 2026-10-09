# Classic rack skin

Original STAR/AMP artwork derived from the selected Classic stacked rack study.
No Winamp artwork or proprietary font files are included. Production standalone
AMP and negotiated FOLD embedding use these assets through `src/embed/skin.rs`.
The independent `skin-player` example remains a separate diagnostic tool.

Regenerate from the editable drawing sources with Python and `rsvg-convert`:

```sh
python3 scripts/design/player_glyphs.py
python3 scripts/design/skin_assets.py
python3 scripts/design/bundle_skin.py
nix develop -c cargo fmt
```

Panel/button masters and coverage masks have original 1x/2x density variants.
`manifest.json` records density-specific sprites, slice/content insets, ordered
mask layers and palette roles. Fixed corners and tiled edges preserve geometry
when panels expand; the application does not stretch a completed screenshot.

`clock-on/off` atlases retain ten fractional phases from the reference's 34.1 px
digit advances. `transport-glyphs` preserves the five original square-button
symbols. `fixed-labels` rasterizes the source study's JetBrains Mono control
lettering; its bounds are recorded in `fixed-labels.json`. Variable Unicode
metadata uses KIT's native shaped text. The earlier 5×7 control font is retained
only for comparison in the independent proof.

The graphical default reproduces the source palette. Theme masks adapt the art
to existing AMP themes; production checks cover all sixteen built-ins, actual
button contrast, rounded/rigid frames, and 1x/2x layout. This is not a claim of
user visual acceptance or physical macOS/SSH verification.
