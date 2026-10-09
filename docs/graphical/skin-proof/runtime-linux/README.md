# Linux Kitty runtime evidence

2026-10-09: release build, Kitty 0.49.2, private 1800×1000 Xvfb display,
Mesa software GL. `report.json` records 17 presentations and ten interaction
checks from `scripts/visual/skin_kitty.py`. Presentation time includes composition
and terminal protocol output; it does not measure photons, network latency or
production playback/analysis. Initial presentation 14.33 ms; maximum subsequent
presentation 16.89 ms in this run, including resize and density transitions.

No claim of macOS, authenticated SSH, audio, all-action parity or final visual
acceptance is supported by these sample-control checks.
