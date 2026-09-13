# Album art

`i` opens the album window: the cover, the record, and where the cover came
from. This page covers where art is looked for, how to pick a different
picture, the catalog links, and the optional online lookup.

## Where the cover comes from

Art is looked for in this order, stopping at the first hit:

1. a picture in the audio file's own tags
2. an image beside it named like a front cover
3. any other image in the folder
4. one level down into `Covers/`, `Scans/` or `Artwork/`

The ranking is not a guess. Counting every image beside the audio in the
reference library gives:

| Preferred | Count | | Rejected | Count |
| --- | --- | --- | --- | --- |
| `cover` | 695 | | `back` | 139 |
| `front` | 204 | | `cd` | 132 |
| `folder` | 175 | | `inlay` | 83 |
| | | | `obi` | 37 |
| | | | `booklet-N` | ~200 |

Taking the first image in the directory picks a disc label or the back of the
case about a third of the time, so there is a rejection list as well as a
preference list, and neither is decoration.

## Choosing a different picture

The panel names the file it chose, and how many others it found:
`folder · Front 2.jpg  3/22`.

| Do | With |
| --- | --- |
| move through the candidates | the `<` and `>` controls, the mouse wheel, or `left`/`right` with the Album panel focused |
| open the full original in the desktop viewer | click the cover, or `o` / `enter` |
| see everything that was considered | `alt+i`, or `c` with the panel focused |
| look the cover up again | click `retry`, or `alt+r` |

Nothing is discarded to reach the candidate list. Backs, disc scans and
booklet pages are all still offered, just ranked last, because the ranking
decides what shows by default rather than what you are allowed to see.

A choice is remembered against the album, not its path, so a rescan or a
remount keeps it, and it applies the next time the record comes round.

### The chooser

`alt+i` opens a chooser over the album window listing everything that was
considered: the images on disk, and the releases the archive offered with how
alike their titles are. It is there because a highly relevant search result is
not the same as the right record. Searching Cinderella's `Monster Ballads`
returns `Best Ballads`, which MusicBrainz rates highly and which is the wrong
album. Anything that close but not certain is offered rather than taken.

### The external viewer

The system default opener is used unless `[art] viewer` supplies a command
and arguments. The image path is appended as one literal final argument,
without a shell, so spaces and punctuation remain safe:

```toml
[art]
viewer = ["imv", "--fullscreen"]
```

Embedded images and artwork from an SSH library are copied at their original
resolution into the rebuildable art cache only when opened. A local sidecar
or an already cached archive image is opened directly.

## Catalog links

The underlined album and artist names are database links, and they are
followed only when you click them.

| The record is | Opens |
| --- | --- |
| tagged with an exact MusicBrainz relationship to Encyclopaedia Metallum | that Metal Archives page |
| tagged with a metal genre | a Metal Archives search |
| tagged with a MusicBrainz identifier | its exact MusicBrainz entry |
| anything else | a pre-filled MusicBrainz search |

The metal detection recognises subgenres and common family names such as
NWOBHM, djent, thrash, doom, sludge, grindcore and deathcore, including
compound and Cyrillic tags; plain classic rock and hard rock remain
non-metal. When identifiers or genre are missing, STAR/AMP uses the
artist-and-album combination to discover a curated Metal Archives
relationship through MusicBrainz before falling back.

Metal Archives pages are never scraped, and none of this needs a scrobbling
account.

## Fetching covers online

`[art] fetch = true` looks up whatever is left on the Cover Art Archive, by
way of a MusicBrainz release search.

> [!IMPORTANT]
> It is **off by default**, because it means sending an artist and an album
> name to a third party, and that is your decision rather than a favour to be
> done.

How it behaves once on:

- Results are cached under `cache/art/`, keyed on the album rather than the
  path, so a rescan or a remount keeps them.
- Albums with no art there are remembered as such for a week, so a coverless
  library does not re-ask on every track change.
- Requests are spaced at one a second and back off when asked to.

### When the album cannot be placed

A box-set disc, a compilation, a rip whose album tag names something no
catalogue has heard of: the album itself cannot be found, but the artist and
the song title usually still can be. STAR/AMP asks what record the _song_
originally came from and uses that cover. Playing `To Be With You` off a
`Monster Ballads` compilation shows _Lean Into It_; `High Enough` shows _Damn
Yankees_.

Only official studio albums count for this. Live records, compilations and
bootlegs are excluded, which matters because bootlegs are often dated ahead
of the record they were taken from.

### Retrying

A "no cover found" is remembered for a week. When that is the wrong answer,
because a tag has been fixed or the archive was having a bad hour:

- the panel's own `retry` is clickable and `alt+r` does the same thing. It
  clears what was remembered about the album _and_ the backoff, because being
  told to try now means now.
- `staramp art retry` clears the lot at once, for after a batch of tags.

## How it is drawn

| Terminal | Cover |
| --- | --- |
| kitty, and anything else that speaks its graphics protocol | real pixels |
| everything else | half blocks, two pixels a cell. Coarse but correct |

`[ui] graphics` overrides the detection:

| Value | Does |
| --- | --- |
| `auto` | detect |
| `kitty` | force pixels on, for ssh or a multiplexer where the outer terminal cannot be seen from in here |
| `blocks` | force pixels off |
| `off` | drop the picture entirely and give the text the whole panel |

Nothing about art happens on the drawing path. Lookups, decoding and any
network request run on their own thread with their own read-only handle to
the index, because the library may be on a removable volume and a `stat` on a
dead mount blocks for as long as the kernel likes. A frame must never be able
to wait on one.
