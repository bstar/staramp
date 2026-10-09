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
(root / "src/embed/skin_assets.rs").write_text("\n".join(lines))
