# Linux Kitty runtime evidence

2026-10-09: release build, Kitty 0.49.2, private 1800×1000 Xvfb display,
Mesa software GL. `report.json` records 17 presentations and ten interaction
checks from `scripts/visual/skin_kitty.py`. Presentation time includes composition
and terminal protocol output; it does not measure photons, network latency or
production playback/analysis. Initial presentation 14.33 ms; maximum subsequent
presentation 16.89 ms in this run, including resize and density transitions.

No claim of macOS, authenticated SSH, audio, all-action parity or final visual
acceptance is supported by these sample-control checks.

Theme runtime: `themes-report.json` adds all built-in cycling and the expiring
footer notice to the checks (35 presentations; max subsequent presentation
16.23 ms in this run). `kitty-theme.png` is the captured themed terminal.
All sixteen active-control palettes also pass the example's 4.5:1 contrast test.
