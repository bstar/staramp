# Gravity Queue (experimental)

Gravity Queue takes one track as a seed and builds a new list out from it,
drawn from the entire indexed library. The same track menu also builds
artist playlists and library timelines, which this page covers too.

> [!NOTE]
> Experimental. It works from a local spectral analysis rather than a learned
> model, and the results are only as good as that analysis. Everything it
> produces is an ordinary unsaved playlist, so nothing it does is hard to
> undo.

## Building a journey

1. Play a track, or select one in the playlist.
2. Click **gravity** on the playlist header, or right-click the track and
   choose **create Gravity list from this**.
3. Choose a journey and press enter.

| Journey | Does |
| --- | --- |
| **Rediscover** | brings back music you played often but have not heard lately |
| **Climb** | builds an energy arc |
| **Wander** | favors unfamiliar music connected by smooth sonic transitions, kept near the seed's genre family and guitar texture rather than rewarding random stylistic outliers |
| **Bridge to selected** | makes the selected track a destination and finds a path toward it |

The seed becomes the first row, unrelated tracks are filtered out, and the
related tracks are shaped into the journey. The source playlist is never
overwritten implicitly: the result is an unsaved list that `ctrl+s` saves
under a new name.

### The knobs

All of them are editable in the gravity panel, and their defaults live under
`[journey]` in the [configuration](configuration.md).

| Knob | Does |
| --- | --- |
| **intensity** | how far the journey is allowed to travel from the seed |
| **artist spacing** | prevents close repeats of one artist |
| **artist variety** | keeps prolific artists from dominating a large queue |
| **minimum match** | filters weak seed relationships before ordering. Defaults to 70% |
| **seed** | the deterministic random seed, so the same settings give the same list |

### Why is this track here?

Every accepted track carries a **why** explanation, beginning with its
measured match quality. **Why is this track here?** in the track menu shows
it for an active result.

Logical tracks from a CUE sheet are scored independently, just like tracks
stored as separate files.

### What it will not do

- change which tracks are in the queue behind your back
- disturb the part of the queue that has already played
- split consecutive tracks from the same CUE image
- replan silently while music is playing. Replanning is something you ask for

## Artist playlists

Right-click a track and choose **create playlist for this artist** to build a
list of everything by that artist from the full indexed library. The clicked
track is first, track-artist and album-artist credits are both recognized,
and the result is an unsaved list like Gravity.

## Library timelines

Right-click a track and choose **create library timeline…** to lay the whole
indexed library out in date order, oldest album first.

1. Choose the date assigned to each album:

   | Choice | Uses |
   | --- | --- |
   | **filesystem date** | file creation time, with modification time as the fallback |
   | **first indexed** | the date star/amp first indexed each file |
   | **release year from tags** | the tagged year |

2. Choose **all years**, or one year for just that slice of the collection.

Albums stay intact, with discs and tracks in their proper playback order. An
album's position comes from the median date of its tracks, so one replaced or
re-tagged file does not drag an otherwise older album out of place. The
result is unsaved until you save it as a playlist.

The timeline always works from the full indexed library, not from whatever
playlist or filter was open when you asked for it.

## The sonic index

Audio features are computed locally, and stored in `index.sqlite` beside the
library index. The player analyzes one pending track at a time while stopped
and prioritizes a bounded batch when a journey is requested.

To inspect or fill it directly:

```sh
staramp analyze --status        # coverage, without decoding anything
staramp analyze                 # the next 25 pending tracks
staramp analyze --all           # everything pending
```

Once a track is analyzed, `energy`, `tempo`/`bpm`, `brightness`, `dynamics`,
`distortion`/`guitar` and the boolean `analyzed` are available to
[smart playlists](smart-playlists.md).

Some details worth knowing:

- The distortion value is a local, deterministic likelihood derived from
  midrange harmonic density, sustain, and compression. It is useful for
  ranking and filtering, but it is not a claim that every distorted sound is
  a guitar.
- When the analyzer version changes, existing rows become pending and
  `staramp analyze --all` refreshes them in place.
- A remote library is analyzed on the machine that owns it. No whole audio
  file is pulled across the link just to fill a local cache.
- The storage format is model-versioned, but no third-party ML weights are
  bundled until their checkpoint license explicitly permits commercial
  redistribution. Code licensing alone is not treated as permission to ship
  weights.
