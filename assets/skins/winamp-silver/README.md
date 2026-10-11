# Winamp Silver

Original STAR/AMP silver transport artwork inspired by the Winamp 2 reference,
with the existing 29px control geometry and glyph atlas. No Winamp bitmap is
redistributed. `scripts/design/silver_buttons.py` creates the six SVG states and
independently exports 1x/2x PNGs; `bundle_skin.py` embeds them in the native skin.

The named `winamp-silver` theme pairs these plates with a blue-gray three-stop
panel gradient and muted sage green display accents. Panel shading is bounded
native geometry behind the original fixed-corner bitmap frames. Artwork padding
and transparent pixels use the same gradient as the Album panel. Minimal action
buttons retain their selected treatment.

Rounded and rigid panel frames use Thin Outline (study D): a single muted
silver #60657c outline, with no stepped highlight or shadow. The three-stop
body gradient reaches the outline. Fixed corners and tiled edges retain the
original geometry at 1x/2x. Every module shares the EQ title strip, including
Player, Album, Activity and Playlist; content well palettes do not affect headers.
