#!/usr/bin/env python3
"""Bundle original player masks; run after skin_assets.py."""
import json
from pathlib import Path
root = Path(__file__).resolve().parents[2]
manifest = json.loads((root / "assets/skins/classic/manifest.json").read_text())
names = ["panel", "panel-rigid", "well", "button-normal", "button-active", "button-hover", "button-pressed", "button-focus", "button-disabled"]
ids = sorted({layer["sprite"]["asset"] for density in [1, 2] for name in names for layer in manifest["layers"][f"{density}/{name}"]})
lines = ["// Generated from assets/skins/classic/manifest.json by scripts/design/bundle_skin.py.", "pub(super) const PNGS: &[(&str, &[u8])] = &["]
for asset in ids:
    density, name = asset.split("/")
    lines.extend(["    (", f'        "{asset}",', f'        include_bytes!("../../assets/skins/classic/{density}x/{name}.png"),', "    ),"])
lines.append("];\n")
lines.append('pub(super) const GLYPHS: &[(u16, &str, &[u8])] = &[')
for density in [1, 2]:
    for name in ['clock-on', 'clock-off', 'transport-glyphs', 'fixed-labels', 'focus-rounded', 'focus-rigid', 'minimal-labels', 'readout-glyphs']:
        lines.extend([
            '    (', f'        {density},', f'        "{name}",',
            f'        include_bytes!("../../assets/skins/classic/{density}x/{name}.png"),',
            '    ),',
        ])
for density in [1, 2]:
    for name in ['panel-rounded', 'panel-rigid']:
        lines.extend([
            '    (', f'        {density},', f'        "silver-{name}",',
            f'        include_bytes!("../../assets/skins/winamp-silver/{density}x/{name}.png"),',
            '    ),',
        ])
    for state in ['normal', 'hover', 'pressed', 'focus', 'active', 'disabled']:
        lines.extend([
            '    (', f'        {density},', f'        "silver-button-{state}",',
            f'        include_bytes!("../../assets/skins/winamp-silver/{density}x/button-{state}.png"),',
            '    ),',
        ])
lines.append('];\n')
lines.append('pub(super) const LABELS: &[(&str, u16, u16, u16)] = &[')
for text,y,width,height in json.loads((root / 'assets/skins/classic/fixed-labels.json').read_text()):
    lines.append(f'    ({json.dumps(text)}, {y}, {width}, {height}),')
lines.append('];\n')
lines.append('pub(super) const MINIMAL_LABELS: &[(&str, u16, u16, u16)] = &[')
for text,y,width,height in json.loads((root / 'assets/skins/classic/minimal-labels.json').read_text()):
    lines.append(f'    ({json.dumps(text)}, {y}, {width}, {height}),')
lines.append('];\n')
lines.extend([
    'pub(super) struct ReadoutCell {',
    '    pub size: u16,', '    pub character: char,',
    '    pub rect: starkit::native_surface::PixelRect,', '    pub advance: u16,', '}',
    'pub(super) const READOUT_CELLS: &[ReadoutCell] = &[',
])
for cell in json.loads((root / 'assets/skins/classic/readout-glyphs.json').read_text()):
    lines.append(f"    ReadoutCell {{ size: {cell['size']}, character: '{cell['character']}', "
                 f"rect: starkit::native_surface::PixelRect {{ x: {cell['x']}, y: {cell['y']}, "
                 f"width: {cell['width']}, height: {cell['height']} }}, advance: {cell['advance']} }},")
lines.append('];\n')
(root / "src/embed/skin_assets.rs").write_text("\n".join(lines))
