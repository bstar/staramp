# Configuration

Everything star/amp keeps lives under one directory, and everything it can be
told lives in one file there. This page covers the file, the themes, the
directory layout, and the terminal-side choices about fonts and drawing.

## The config file

`~/.local/staramp/config.toml`, written with comments on first run.

Settings changed from inside the player are written back here. The write is a
line edit rather than a re-serialisation, so comments, ordering and any key
star/amp does not know about survive it byte for byte.

### Library and playlists

| Key | Does |
| --- | --- |
| `library_root` | where the music is. The one setting with no default; `staramp scan <dir>` sets it |
| `playlist_dir` | `.m3u` directory, read and written in place. Point it at MPD's to share one set |
| `[playlist] crossfade` | blend the last five seconds of each track into the next. `F` toggles it |
| `[imports] staging_root` / `quarantine_root` | the inbox `staramp import` reviews, and where its rejects go. See [Reviewing new music](importing.md) |
| `[remote] host` / `root` | the SSH destination and far-side library root `staramp remote` uses when given no arguments. See [A library on another machine](remote.md) |
| `[session] share` | `"view"` (default) shares the queue and what is looked at; `"playback"` shares only the music. See [Sessions](sessions.md) |

### Sound

| Key | Does |
| --- | --- |
| `volume` | 0.0 to 1.0 |
| `[output] mode` | `"native"` for bit-perfect at the file's rate, `"fixed"` to pin the device to one rate |
| `[output] fixed_rate` | the rate `"fixed"` pins |
| `[cue] pregap` | which track a pregap belongs to |
| `[eq] enabled` / `preset` | enable the equalizer and select a built-in or managed profile. See [Equalizer](equalizer.md) |
| `[replaygain] mode` | `off` (default), `track`, or `album`. See [ReplayGain](playback.md#replaygain) |

### Look

| Key | Does |
| --- | --- |
| `theme` | `"system"` to follow the desktop, or a name. See [Theming](#theming) |
| `[ui] seek_style` | how the seek bar is drawn: `ansi` (default), `bar`, `thin`, `blocks`. See [Fonts](#fonts) |
| `[ui] graphics` | how covers are drawn: `auto`, `kitty`, `blocks`, `off`. See [Album art](album-art.md) |
| `[ui] buttons` | transport buttons: `auto` for pictures where the terminal can, `text` for ASCII. `o` toggles |
| `[ui] padding_x` / `padding_y` | blank columns and rows around the window |
| `[ui] show_equalizer` | draw the EQ panel. Visibility does not enable or bypass the EQ |
| `[ui] show_scrobbler` | draw the Activity panel. The key name is historical, and it never affects a provider |
| `[vis] mode` | `bars` `leds` `peaks` `dots` `wave` `scope` `fluid` `off` |
| `[vis] gain_db` | shifts the analyzer range if your music sits quiet or loud |
| `[vis] smoothing` | how long the `fluid` mode's bars take to fall, 0 to 1. Lower snaps to the music |
| `[vis] bar_width` / `bar_gap` | cells per bar, and columns between bars |
| `[fx]` | track-change effects, including `reduced_motion` to disable them |

### Services

All of these are off until you turn them on.

| Key | Does |
| --- | --- |
| `[art] fetch` | look covers up on the Cover Art Archive when the files have none |
| `[art] viewer` | optional image-viewer argv list; the full-resolution image path is appended |
| `[scrobble] lastfm` / `listenbrainz` | submit eligible tagged listens to that provider. See [Listening](listening.md) |
| `[discord] enabled` / `client_id` | publish the playing track to Discord Rich Presence. See [Listening](listening.md#discord-rich-presence) |
| `[journey]` | Gravity Queue defaults: preset, intensity, artist spacing and variety, minimum match, seed. See [Gravity Queue](gravity-queue.md) |

## Theming

`theme = "system"` follows the desktop. star/amp reads Stylix's
`~/.config/stylix/palette.json`, so whatever base16 scheme the rest of the
desktop is set to, the player matches it, analyzer ramp included.

```sh
staramp theme list          # what is available, and what the system is set to
staramp theme show cosmic   # swatches and measured contrast
```

Sixteen themes ship built in, and `alt+t` cycles them live:

`winamp-classic` · `cosmic` · `catppuccin-mocha` · `catppuccin-latte` ·
`gruvbox-dark` · `nord` · `tokyo-night` · `dracula` · `rose-pine` ·
`everforest` · `solarized-dark` · `one-dark` · `kanagawa` · `ayu-dark` ·
`matte-black` · `terminal`

Import your own:

```sh
staramp theme import-base16 scheme.yaml   # any base16 scheme
staramp theme import base.wsz             # a classic Winamp skin
```

Every built-in is checked against WCAG AA in the test suite: body text,
selected rows and dim text all have to clear 4.5:1.

## Where it keeps things

Everything lives under one directory, so a whole star/amp setup can be backed
up, moved, or deleted by moving one folder:

```
~/.local/staramp/
├── config.toml        your settings
├── credentials.toml   Last.fm/ListenBrainz secrets (0600)
├── session.toml       what was playing, for the resume offer
├── index.sqlite       the library index, and the sonic analysis. Rebuildable
├── imports.sqlite     the staging inbox's index. Rebuildable
├── activity.sqlite    permanent plays, skips, pending scrobbles, Gravity feedback
├── playlists/         .m3u files, read and written in place
├── equalizers/        managed and imported Equalizer APO profiles
├── themes/            your own themes, and imported Winamp skins
└── cache/             album art, logs. Safe to delete
```

- `$STARAMP_DIR` relocates all of it; `$STARAMP_CONFIG_DIR` moves just the
  config.
- An older install under `~/.config/staramp` and `~/.local/share/staramp` is
  migrated automatically on first run, and nothing already present at the
  destination is overwritten.

### Playlists and MPD

Point `playlist_dir` at MPD's own playlist directory if you want the two to
share one set. star/amp writes the same URI form MPD does, including
`Album/rip.cue/track0007`, so they stay in sync.

Saving from the player (`ctrl+s`) writes the order you are looking at, not the
internal one, so a queue you have grouped, shuffled or arranged by hand saves
the way it reads on screen. It writes bare library-relative paths, which is
what MPD reads and writes. A line star/amp could not resolve is copied through
exactly as it was found rather than being dropped or rewritten.

## Fonts

star/amp cannot choose the font it is drawn in. That is your terminal's
setting, and no terminal program can override it. Three things follow from
that.

### The transport buttons are pictures

The buttons do not go through the font. Each is rasterised at the terminal's
real cell size, in the theme's colours, and put on the screen the way cover
art is: over the kitty, sixel or iTerm2 graphics protocol. They are the
Material Design transport shapes, and they are identical on every terminal
that can show them. Whether to draw them is decided separately from
`[ui] graphics`: turning covers off leaves the buttons alone.

Where the terminal has no graphics protocol the buttons are drawn as text, in
ASCII: `<<`, `|>`, `||`, `[]`, `>>`, which every font there has ever been can
draw. A font with ligatures draws `|>` as one triangle; one without draws the
two characters. Both are two cells, so nothing moves.

`o` switches between the two, and `[ui] buttons = "text"` makes the choice
stick.

> [!TIP]
> Force text where a terminal claims a protocol it does not really have.
> Inside a multiplexer that does not forward graphics, most often, the
> pictures are transmitted and simply never appear.

### The seek bar has four styles

`[ui] seek_style`, or `d` to cycle them:

| Style | Draws |
| --- | --- |
| `ansi` (default) | `[====----]` from plain characters. Any terminal can manage it, but it moves a whole cell at a time. Its highlight goes bold as well as bright, so the sweep shows on a terminal that renders the two colours alike |
| `bar` | a double-stroke box-drawing rule on the cell's middle, level with the clock digits either side |
| `thin` | the same, single stroke |
| `blocks` | fills the cell, by eighths |

`bar` and `thin` sit level with the digits because a block element would sit
above or below them, since blocks anchor to a cell edge. Both shade their
leading cell between the groove and the fill, so the bar moves smoothly
without taking a full row of height.

Everything else star/amp draws (box drawing, block elements, braille) is
covered by any font shipped as a terminal font. The packages install a
suitable one where the distribution has it (`fonts-noto-core` on Debian,
`noto-fonts` on Arch). Installing it is all a package can do; selecting it in
your terminal is still up to you.

### Why the visualizer's gap is a whole column

A bar's tip is a part-height block, and nothing in Unicode is both part-height
and part-width, so a sub-cell separator can only be drawn on the _body_ of a
bar. It disappears along the top edge, which is the part you actually look at.

The gap is therefore a real column. Widen `bar_width` to make it a smaller
fraction of the bar, or set `bar_gap = 0` for a solid spectrum.
