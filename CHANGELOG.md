# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Remote libraries over SSH.** `staramp remote <host>` plays a library that
  lives on another machine. Nothing is installed or left running there: one
  supervised `ssh` connection is opened and the files are read through its
  `sftp` subsystem, using the user's own `~/.ssh/config`, keys and agent. The
  index is copied once and queried locally, so browsing and search stay at full
  speed; the audio is streamed on demand behind a read-ahead window, so a brief
  drop in the link is inaudible and nothing but the index is written to disk.
  Both decode backends stream — symphonia through `MediaSource`, libav through
  a custom AVIO context — so WavPack, APE, Musepack and DSD are not
  second-class.
- **macOS.** Darwin is a supported target, as either end of a remote library.
  The Nix flake builds on `aarch64-darwin`, a plain `cargo` build works against
  Homebrew's ffmpeg, and CI covers both. MPRIS is compiled out where there is
  no D-Bus rather than shipped as dead weight.

### Changed

- Cue virtual tracks are opened from the index rather than by re-reading their
  sheet: one query in place of reading and character-set-guessing the sheet,
  listing the directory to match its `FILE` references, and opening the backing
  audio file purely to ask its sample rate. Covers 10,808 of the reference
  library's 10,840 cue tracks; the rest fall back to the sheet, as does any
  sheet the scan has never seen.
- The picture embedded in an audio file is read once per album instead of
  twice.
- A remembered album-art choice is now keyed on the library-relative path
  rather than the absolute one, so it survives a remount as its documentation
  always claimed. Existing choices are re-derived once.

### Fixed

- On platforms without Linux's abstract sockets — macOS — the leader election
  between several windows was racy: two instances started together could both
  conclude they were the leader and fight over the audio device. A `flock`
  makes it atomic.

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
