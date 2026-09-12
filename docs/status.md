# Status and the numbers

Usable, and used daily against the reference library. Every format there
plays, the index and playlists work against real data, and the player runs on
Linux and Apple Silicon macOS.

## Done

| Area | State |
| --- | --- |
| Decoding | 9 of 9 formats, 7 of them bit-identical to ffmpeg |
| Audio output | bit-perfect at 44.1/96/192 kHz, zero underruns; converts only where the device cannot follow |
| Gapless and crossfade | album-aware prefetch, and a five-second crossfade on request |
| CUE sheets | 1,054 of 1,124 sheets, 10,840 virtual tracks |
| Library index | 34k files, 458 s cold, 1.1 s warm |
| Playlists and queue | 29 MPD playlists resolve at 96.7%; unresolved lines survive a rewrite untouched |
| TUI | docked Winamp windows, themes, keymap, mouse, per-panel focus |
| Winamp `.wsz` import | ramp verified exact against VISCOLOR.TXT |
| MPRIS and IPC | `playerctl -p staramp`, `staramp ctl`. MPRIS is Linux only |
| Packaging | Nix flake and home-manager module, PKGBUILD, `.deb`, AppImage, portable tarball, CI |
| Text effects | audio-reactive, and switchable off |
| Visualizer | seven modes plus off, cycled with `w` and `W` |
| Session resume | offers to pick up where you left off |
| Album art | embedded, folder, `Covers/`, and the Cover Art Archive; kitty graphics where there are any |
| Equalizer | ordered parametric filters, APO import/export, f64 DSP, managed profiles |
| Listening history | permanent local plays/skips, available to smart playlists |
| Scrobbling | independent Last.fm and ListenBrainz accounts, now-playing and durable retries |
| Discord presence | local desktop client only, opt-in |
| Remote libraries | one SSH connection, index copied once, audio streamed; Linux and macOS at either end |
| Staging imports | review, audition, import or quarantine, verified with BLAKE3 |

## Partial

| Area | State |
| --- | --- |
| ReplayGain | tags are read and applied; no EBU R128 scanner yet, so 31,700 tracks carry no gain to use |
| Smart playlists | the query language and its SQL compiler work from the CLI; no rule builder in the TUI |
| Gravity Queue | experimental: journeys work from a local spectral analysis; no licensed embedding model bundled |

## Not yet

| Area | State |
| --- | --- |
| macOS Now Playing | MPRIS has no macOS equivalent in this release |

## The numbers quoted here

They come from one library, and three different true counts of it get used
across these pages, so they are worth separating once:

| Count | What it counts |
| --- | --- |
| ~22,000 | audio files on disk |
| 31,928 | addressable tracks, once cue sheets are expanded |
| 34,071 | files a scan looks at, counting sheets, images and side-cars |

The library is 1.1 TB, which is enough to break most players in the same
three places: formats, cue sheets, and scale.
