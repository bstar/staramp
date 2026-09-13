# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-09-12

### Fixed

- The next-track button drew at half its width, with the lit pause icon left
  over from an earlier frame showing in the half it did not cover. A picture
  is transmitted as pixels and placed over a number of cells, and the cache
  kept only the first of those two facts, so a button spanning four cells
  could be served a picture built to cover two.
- A read-ahead test counted the reads the server had got round to serving
  rather than the reads the client issued. The two differ by whatever is in
  flight, which made it fail at random.

### Security

A review of everything staramp parses, launches and downloads, written up in
[docs/security-review.md](docs/security-review.md). What changed:

- **The control socket asks who connected.** On Linux it is bound in the
  abstract namespace, which has no permission bits, so any local user could
  connect, drive playback and read the queue with its file paths. The peer's
  uid is now checked. On macOS the socket and its directory are private, and
  staramp's own directories are 0700 rather than whatever the umask gave --
  the index lists every path in the library and the activity database is a
  complete listening history.
- **An Equalizer APO preset cannot reach the output device with an absurd
  gain.** `Preamp: 400 dB` was a legal line that compiled to a
  hundred-billion-fold multiplier. Gains, widths and frequencies are bounded,
  coefficients are checked to be finite, and a soft limiter runs where the
  signal could have been pushed past full scale -- never on a transparent
  chain at unity, so bit-perfect playback is unchanged.
- **`Include` stays beside its preset**, is measured before it is read, and
  parses each file once. A skin archive's entries are size-bounded, and every
  image is decoded with explicit dimension limits.
- **A library URI cannot leave the library.** A playlist line or an index row
  naming `/etc/shadow` or `../../..` is refused where the bytes would be read,
  rather than opened. Such a line is still preserved on save.
- **A downloaded index is treated as untrusted:** opened with SQLite's
  defensive settings, its schema version checked, and its download bounded by
  the size the far end advertised.
- **Catalog links are checked properly.** The Metal Archives host test ended
  the authority at the first `/`, so a URL from a MusicBrainz relationship
  could send a click somewhere else entirely.
- **The command line no longer prints unfiltered tag text**, `staramp query`
  no longer panics on a non-ASCII error, and the query parser has a depth
  limit. Integer overflow panics in release rather than wrapping.
- **The build is pinned.** The AppImage runtime came from a rolling tag with
  no checksum, and it is the first code that runs when an AppImage is opened.
  Every CI action is pinned to a commit.
- **Fifteen property tests** generate malformed input for the cue, playlist,
  ID3, skin, preset, query, config and SFTP parsers.

## [0.1.0] - 2026-09-11

First release.

### Playback

- Playback of FLAC, MP3, Vorbis, AAC and ALAC through symphonia, and of
  Monkey's Audio, WavPack, Musepack, DSD, Opus and WMA through libavcodec
  linked in-process. Bit-perfect output at the file's own sample rate, on ALSA
  on Linux and CoreAudio on macOS.
- Where the device cannot follow the file (a fixed-rate device, or a device
  with more channels than the file) the conversion happens on the way into the
  output ring and the player reports it, so nothing plays at the wrong speed
  and mono is copied across the device's channels at unity gain.
  `[output] mode = "fixed"` pins one rate.
- CUE sheets as first-class virtual tracks, including MPD's
  `Album/rip.cue/track0007` URI form, played as one linear read of one file.
  Cue tracks are opened from the index rather than by re-reading their sheet.
- Gapless album playback: while album sorting is active, the next track of the
  same release is opened and decoded ahead, and joins the output ring without
  reopening the device.
- Crossfade between tracks, five seconds, toggled with `F` or from the footer
  and saved as `[playlist] crossfade`.
- ReplayGain from tags, applied at track boundaries. Off by default.
- A parametric equalizer: an ordered, per-channel filter chain with exact
  value editing, bypass, duplicate and reorder controls, and a drawn response
  curve. Imports and exports EQ-focused Equalizer APO profiles (`Preamp`,
  standard filters, IIR, GraphicEQ, Channel and Include), keeps managed
  profiles under `equalizers/`, recompiles against the real device rate, and
  processes in f64.

### Library

- A SQLite library index with resumable scans, change detection and a file
  watcher that survives the disk being unplugged.
- A library browser, playlist filtering with `/`, and bulk playlist editing by
  tagging rows.
- A smart-playlist query language with a SQL compiler, including listening
  history fields and, where a track has been analyzed, sonic fields such as
  `energy`, `tempo`, `brightness`, `dynamics` and `distortion`.
- Album art from tags, the folder, `Covers/`, and optionally the Cover Art
  Archive; a chooser over every candidate; a click on the cover opens the
  original in the desktop viewer; album and artist names link to MusicBrainz
  or Metal Archives entries.
- Remote libraries over SSH. `staramp remote <host>` plays a library that
  lives on another machine. Nothing is installed or left running there: one
  supervised `ssh` connection is opened and the files are read through its
  `sftp` subsystem, using the user's own `~/.ssh/config`, keys and agent. The
  index is copied once and queried locally; the audio is streamed on demand
  behind a read-ahead window, and both decode backends stream so WavPack, APE,
  Musepack and DSD are not second-class.
- Staging imports. `staramp import` reviews an inbox of albums in the ordinary
  player, and imports or quarantines whole albums. An import preserves the
  album folder, copies through a verified temporary directory, installs
  atomically, and removes the staging copy only after the permanent index has
  seen the album. Tags are never rewritten.
- Library timelines: a playlist of the whole library in filesystem, first
  indexed, or tagged-year order, with every album intact.

### Listening

- Permanent local listening history, in its own `activity.sqlite`, separate
  from the rebuildable index, so plays, skips and queued scrobbles survive a
  rescan or a replaced remote index. Only the window that owns playback records
  a listen.
- Optional, independent Last.fm and ListenBrainz scrobbling with now-playing
  and durable retries. Credentials live in a mode-0600 `credentials.toml`.
- Optional Discord Rich Presence through the local desktop client's IPC
  socket. No bot, no token, artwork resolved through MusicBrainz and the Cover
  Art Archive, never dependent on Last.fm.
- Gravity Queue (experimental): journeys drawn from the whole indexed library
  and seeded from a track, shaped by a local, deterministic spectral analysis
  that `staramp analyze` fills. Reshapes only the unplayed part of the queue,
  and only when asked.

### Interface

- A docked Winamp-style TUI: player, album, equalizer, activity and playlist
  panels, each with its own keyboard once focused; sixteen built-in themes,
  Winamp `.wsz` skin import, Stylix and COSMIC system theme following, and
  full mouse support.
- Transport buttons rasterised at the terminal's cell size over the kitty,
  sixel or iTerm2 graphics protocol, with an ASCII fallback and a toggle.
- A seven-mode audio visualizer of staramp's own, with audio-reactive text
  effects that can be switched off.
- One session across several terminals, with any window able to take it over,
  and a resume offer on the next start.
- MPRIS on Linux, a control socket, and `staramp ctl`.

### Platforms and packaging

- Linux on x86_64 and aarch64, and macOS on Apple Silicon. On macOS output
  goes through CoreAudio, MPRIS is compiled out, and the multi-window leader
  election uses a `flock` rather than an abstract socket.
- A Nix flake with a home-manager module, a `.deb` per Debian generation, a
  PKGBUILD, an AppImage carrying its own ffmpeg, and a portable tarball built
  against glibc 2.31.

[Unreleased]: https://github.com/bstar/staramp/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/bstar/staramp/releases/tag/v0.1.1
[0.1.0]: https://github.com/bstar/staramp/releases/tag/v0.1.0
