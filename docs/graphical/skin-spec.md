# Classic player: measured design specification

Reference: `treatments/winamp-01.svg`, player bounds x24/y48/1352x230.
All coordinates below are local to the player, at 100% UI scale and 1x density.

| Part | X | Y | Width | Height |
| --- | ---: | ---: | ---: | ---: |
| Outer frame | 0 | 0 | 1352 | 230 |
| Title strip | 7 | 7 | 1338 | 24 |
| Display well | 14 | 43 | 1324 | 110 |
| Clock origin | 28 | 58 | 134 | 54 |
| Analyzer | 190 | 52 | 1132 | 48 |
| Track title baseline | 190 | 122 | flexible | 13 px font |
| Technical baseline | 190 | 141 | flexible | 11 px font |
| Seek track | 78 | 169 | 1196 | 3 |
| Transport buttons | 16 + n×35 | 187 | 29 | 29 |
| Shuffle | 237 | 187 | 100 | 29 |
| Repeat | 346 | 187 | 100 | 29 |
| Volume track | 1172 | 202 | 130 | 4 |

Player width is fluid. Analyzer, title, technical text and seek gain width.
The clock, transport, toggle widths and title strip height retain their sizes.
Volume anchors to the right. UI scale and display density are independent.
The first narrow proof retains a minimum width of 720 logical pixels; compact
layouts need a separate specification rather than arbitrary font shrinking.

## Typography roles

- Branding and panel labels: 12 px, bold, mono; separately trial custom bitmap.
- Track title: 13 px, regular, mono, accent; Unicode fallback required.
- Technical metadata: 11 px, regular, mono, muted.
- Control labels: 14 px, bold, centered; separately trial bitmap lettering.
- Clock: custom segment artwork with inactive segments.

## Visual gate

Use the exact reference palette and content first. Compare all bitmap/outline
label options in the actual native compositor. Provide 1352x230, 900x230,
1800x230, and 2x-density captures, plus source assets and an overlay/difference.
Inspect Linux and Mac at physical 1:1 size. Do not replace the installed player
or migrate the remaining modules until this proof is visually accepted.
