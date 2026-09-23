# STAR/AMP

[![ci](https://github.com/bstar/staramp/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bstar/staramp/actions/workflows/ci.yml)
[![nix](https://github.com/bstar/staramp/actions/workflows/nix.yml/badge.svg?branch=main)](https://github.com/bstar/staramp/actions/workflows/nix.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A terminal player for a more civilized age.

No streaming. No radio stations. No account required. Winamp inspired. It plays
the files on your disk, and it plays _all_ of them.

[![STAR/AMP playing a FLAC album bit-perfect, with the analyzer, album art, listening activity and a library timeline playlist](docs/screenshot.png)](docs/screenshot.png)

## Get it

Linux releases use Nix and AppImage. Apple Silicon macOS builds are available
on the [releases page](https://github.com/bstar/staramp/releases/latest).

```sh
nix run github:bstar/staramp        # Nix, on Linux or Apple Silicon macOS
```

[Installing](docs/installing.md) covers every route, including building from
source on Linux and macOS.

## Play something

```sh
staramp scan ~/Music     # index it, once
staramp                  # play
```

`scan` remembers the directory, so you pass it once. To try a single folder or
a playlist with no index at all:

```sh
staramp ui "/path/to/an/album"
staramp ui some-playlist.m3u
```

Inside the player, `?` shows every key, `space` plays and pauses, `p` opens the
playlist, `i` the album window, `l` the library browser, and `q` quits. The
mouse works everywhere the keyboard does.

## What it does

- **Plays everything.** FLAC, MP3, Vorbis, AAC and ALAC natively; Monkey's
  Audio, WavPack, Musepack, DSD, Opus and WMA through libavcodec linked in
  process. Bit-perfect at the file's own sample rate.
- **Understands rips.** CUE sheets are first-class tracks. Albums play gapless,
  and crossfade is a keypress away.
- **Scales.** A SQLite index that handles a terabyte-sized library: 34,000
  files scan in under eight minutes cold and about a second after that.
- **Looks the part.** Docked Winamp-style panels, sixteen themes, classic
  `.wsz` skin import, a seven-mode visualizer, and album art drawn as real
  pixels where the terminal can.
- **Remembers what you listened to.** Permanent local listening history, with
  optional Last.fm and ListenBrainz scrobbling and Discord presence.
- **Shapes the sound.** A parametric equalizer that imports and exports
  Equalizer APO profiles. ReplayGain from tags.
- **Finds music.** A smart-playlist query language, library timelines, and an
  experimental Gravity Queue that builds a journey out from any track.
- **Reaches other machines.** Plays a library over a single SSH connection
  with nothing installed on the far side, and reviews a staging inbox before
  new music joins the permanent library.
- **Shares one session.** Every open terminal is the same player. MPRIS and a
  control socket for keybinds and status bars.

Linux on x86_64 and aarch64, and macOS on Apple Silicon.

## Read more

The [documentation](docs/README.md) has a page for each of those. The ones most
people want first:

- [Keys and mouse](docs/keys-and-mouse.md)
- [Configuration](docs/configuration.md)
- [Album art](docs/album-art.md)
- [Listening history, scrobbling and Discord](docs/listening.md)
- [A library on another machine](docs/remote.md)
- [Embedding the player](docs/embed.md)
- [If something is wrong](docs/troubleshooting.md)

[CONTRIBUTING.md](CONTRIBUTING.md) is for building and changing it,
[CHANGELOG.md](CHANGELOG.md) for what each release holds, and
[SECURITY.md](SECURITY.md) for reporting a vulnerability.

## Credits

staramp links [FFmpeg](https://ffmpeg.org) for the formats symphonia does not
decode; it is linked dynamically, and the AppImage carries Debian's
build beside the binary. See [NOTICE](NOTICE). The visualizer, the text
effects and the equalizer are staramp's own. Colour schemes come from
Catppuccin, Dracula, Nord, Rosé Pine, Tokyo Night and others, each MIT.

Not affiliated with, or endorsed by, Winamp or its rights holders. It is a
tribute to a program a lot of us grew up with.

## License

MIT. See [LICENSE](LICENSE).
