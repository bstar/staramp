# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Parametric equalizer and Equalizer APO profiles.** The old fixed ten-band
  control is now an ordered, per-channel filter chain with exact-value editing,
  bypass, duplicate and reorder controls. It imports and exports EQ-focused APO
  profiles (`Preamp`, standard filters, IIR, GraphicEQ, Channel and Include),
  stores managed profiles under `equalizers/`, recompiles against the real
  device rate, and processes the chain in f64. Panel visibility remains purely
  visual, including in windows following another session owner.
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
- A device that could not do the file's sample rate played it at the wrong
  speed. The mismatch was detected and reported — the indicator said
  "resampled" — but nothing resampled: the ring was filled at the file's rate
  and drained at the device's, so a 48 kHz track on a 44.1 kHz-only device ran
  8.4% slow and a semitone and a half flat. The conversion now actually
  happens, on the way into the ring. Follow-the-file is unchanged, so anything
  the device can take is still bit-perfect and no conversion is involved.

  Invisible on a typical Linux desktop, where cpal opens ALSA's `default` and
  its `plug` layer advertises every rate and converts underneath. It bites
  wherever the device is reached directly: CoreAudio always, and any `hw:`
  device or plugless `.asoundrc` on Linux.
- A file with fewer channels than the device offers no longer refuses to play.
  CoreAudio advertises only the channel counts the hardware physically has, so
  every mono file on a Mac failed with "device supports no 1-channel output
  configuration". Mono is now copied across the device's channels at unity
  gain, matching what `plug` and CoreAudio's own up-mix do — not swresample's
  power-preserving matrix, which would have played mono 3 dB quieter on macOS
  than on Linux.
- The bit-perfect indicator accounts for the channel count as well as the rate.
  A remixed mono file reported itself bit-perfect.
- `cargo test` no longer fails when a real instance is running. The session
  lease was taken on the configured socket path rather than the one under test,
  so every test that binds a socket contended with the developer's own player.
- The transport buttons are square, and their faces are centred in them. They
  were drawn as plates four cells by one -- 32 by 17 pixels, twice as wide as
  tall -- and `play` and `stop` sat hard against the left edge, because the
  faces were padded to a common width with a trailing space and the centring
  saw two cells where only one had ink in it. The plate is now three cells by
  a quarter-row either side of the face's own row, which is 24 by 25.5: square
  to within 6%, with every face dead centre.

### Changed

- `[output] mode = "fixed"` and `fixed_rate` are now read. They have been
  documented in the generated config since 0.1.0 and were never consulted.
- The `unicode` transport faces are one cell each -- `«`, `▶`, `⏸`, `■`, `»` --
  where they used to pair the triangles. A face centres in its plate only when
  the two share parity, so a set of mixed one- and two-cell faces cannot centre
  all of them at any plate width.

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
