# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-01

First release.

### Added

- Playback of FLAC, MP3, Vorbis, AAC and ALAC through symphonia, and of
  Monkey's Audio, WavPack, Musepack, DSD, Opus and WMA through libavcodec
  linked in-process. Bit-perfect output at the file's own sample rate.
- CUE sheets as first-class virtual tracks, including MPD's
  `Album/rip.cue/track0007` URI form, played as one linear read of one file.
- A SQLite library index with resumable scans, change detection and a file
  watcher that survives the disk being unplugged.
- A docked Winamp-style TUI: playlist, album art, equalizer, analyzer, sixteen
  built-in themes, Winamp `.wsz` skin import, and full mouse support.
- A library browser, and bulk playlist editing by tagging rows.
- One session across several terminals, with any window able to take it over.
- MPRIS, a control socket, and `staramp ctl`.
- A smart-playlist query language with a SQL compiler.
- Album art from tags, the folder, `Covers/`, and optionally the Cover Art
  Archive.
- ReplayGain from tags, applied at track boundaries. Off by default.
- Packaging: Nix flake with a home-manager module, `.deb` per Debian
  generation, PKGBUILD, AppImage, and a portable tarball.

[0.1.0]: https://github.com/bstar/staramp/releases/tag/v0.1.0
