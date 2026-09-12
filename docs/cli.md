# On the command line

`staramp` with no arguments opens the player. Everything else is a subcommand,
and `staramp --help` lists them all; each has its own `--help`.

## The library

```sh
staramp scan ~/Music                        # build or refresh the index
staramp stats                               # what the index holds
staramp search "black sabbath"              # full-text, from the index
staramp query 'year >= 2015 sort added desc' # smart-playlist expressions
staramp playlists ~/.config/mpd/playlists   # what resolves, what does not
staramp cue-report ~/Music                  # how your cue sheets classify
```

| Command | Notes |
| --- | --- |
| `scan [dir] [--full]` | remembers the directory, so later runs need no argument. `--full` re-reads every tag, ignoring change detection |
| `query <expr> [--count] [--explain]` | see [Smart playlists](smart-playlists.md) |
| `playlists <dir> [--verbose] [--roundtrip]` | `--roundtrip` verifies each playlist rewrites byte-for-byte |
| `cue-report [dir] [--verbose]` | `--verbose` lists every sheet that is not indexable, with the reason |

## Opening things

```sh
staramp ui "/path/to/an/album"              # a folder, with no index
staramp ui some-playlist.m3u                # a playlist
staramp remote music-server                 # a library over SSH
staramp import ~/Music-Inbox                # review a staging inbox
staramp play "album/01 - track.flac"        # one file, no UI
```

`remote` is described in [A library on another machine](remote.md), and
`import` in [Reviewing new music](importing.md).

## Diagnostics

```sh
staramp probe  "album/01 - track.flac"      # codec, rate, depth, channels, length
staramp decode "album/01 - track.flac" -o out.wav --start 30 --duration 10
```

`decode` writes the samples to a WAV without involving the audio device. It
exists to prove sample accuracy against a known-good reference, and it is the
first thing to reach for if a file sounds wrong:

```sh
staramp decode in.flac -o mine.wav
ffmpeg -i in.flac -f wav -acodec pcm_f32le theirs.wav
cmp mine.wav theirs.wav
```

## Maintenance

```sh
staramp theme list                          # what is available
staramp theme show cosmic                   # swatches and measured contrast
staramp theme import ~/skins/base.wsz       # a real Winamp skin
staramp theme import-base16 scheme.yaml     # any base16 scheme
staramp art retry                           # forget every "no cover found"
staramp analyze --status                    # sonic-index coverage
staramp analyze --all                       # analyze everything pending
staramp scrobble auth lastfm                # store a provider's credentials
staramp scrobble status                     # providers and queued submissions
staramp scrobble logout lastfm              # forget them and disable it
```

## Controlling a running instance

For keybinds and status bars:

```sh
staramp ctl toggle
staramp ctl next
staramp ctl prev
staramp ctl stop
staramp ctl seek 30          # relative, in seconds
staramp ctl position 90      # absolute
staramp ctl volume 0.5
staramp ctl shuffle          # toggles, prints the new state
staramp ctl repeat           # off, all, one
staramp ctl set-scrobble lastfm on
staramp ctl status           # JSON
```

On Linux the same is reachable over MPRIS:

```sh
playerctl -p staramp next
```

## Logging

`staramp -v` logs at debug level. Logs go to a file under
`~/.local/staramp/cache/`, never to the terminal, because stdout would
corrupt the screen. `STARAMP_LOG` takes a full tracing filter:

```sh
STARAMP_LOG=staramp::audio=debug staramp
```
