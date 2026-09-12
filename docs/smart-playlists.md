# Smart playlists

`staramp query` runs an expression against the index and prints what matches.
The same expression compiles to SQL, so it stays fast at library scale.

```sh
staramp query 'genre ~ "power metal" and year >= 2015 sort added desc limit 20'
staramp query 'lossless and duration > 600' --count
staramp query 'artist ~ opeth and not cue' --explain    # show the SQL
```

> [!NOTE]
> There is no rule builder in the TUI yet, which is the half of this that is
> not finished. The language and its compiler are done and work from the
> command line.

## Fields

| Group | Fields |
| --- | --- |
| tags | `artist`, `albumartist`, `album`, `title`, `genre`, `composer`, `year`, `trackno`, `discno` |
| file | `codec`, `bitrate`, `samplerate`, `bitdepth`, `duration`, `path`, `filesize`, `added` |
| listening history | `playcount`, `skipcount`, `lastplayed`, `rating`, `loved` |
| sonic analysis | `energy`, `tempo`/`bpm`, `brightness`, `dynamics`, `distortion`/`guitar`, `analyzed` |

Most have short aliases: `ar`, `al`, `ti`, `g`, `y`.

The listening-history fields read the permanent activity database, so
rescanning or replacing a downloaded remote index cannot erase them.

The sonic fields are available where the sonic index has analyzed a track;
see [Gravity Queue](gravity-queue.md) for how it is filled.

```sh
staramp query 'analyzed and energy > 0.7 sort tempo desc limit 50'
staramp query 'genre ~ metal and guitar > 0.65 sort distortion desc'
```

## Operators

| Kind | Forms |
| --- | --- |
| comparison | `=`, `!=`, `>`, `<`, `>=`, `<=` |
| text | `~` (contains), `!~` |
| logic | `and`, `or`, `not`, parentheses |
| standalone | `lossless`, `cue`, `loved`, `unloved`, `never` |
| dates | `today`, `yesterday`, `thisweek`, `thismonth`, `thisyear`, or a number of days |
| ordering | `sort <field> [asc\|desc]`, `limit <n>` |

Errors point at the character that caused them and suggest the field you
probably meant.
