# Keys and mouse

`?` or `F1` opens the key list inside the player, where it is generated from
the same table the keys are dispatched from and so cannot drift from what the
program does. This page is the same list, with the explanations.

> [!NOTE]
> `esc` never quits. Terminals encode `alt`+key as escape followed by the key,
> so binding `esc` to quit would put every `alt` binding one dropped byte away
> from closing the player. `esc` only closes whatever is open on top; `q`
> quits.

## Transport

| Key | Does |
| --- | --- |
| `space` / `c` / `x` | play or pause |
| `v` | stop |
| `z` / `b` | previous / next track |
| `ctrl+up` / `ctrl+down` | volume |
| `s` | shuffle on or off |
| `S` | reshuffle now |
| `r` | repeat off / all / one |
| `F` | crossfade on or off |

## Progress bar

| Key | Does |
| --- | --- |
| `left` / `right` | seek 5 seconds |
| `shift+left` / `shift+right` | seek 30 seconds |
| `d` | seek bar style |
| `o` | transport button style: pictures or text |

## Playlist

| Key | Does |
| --- | --- |
| `up`/`k`, `down`/`j` | move |
| `shift+up` / `shift+down` | move ten rows |
| `pgup` / `pgdn` | page |
| `home`/`gg`, `end`/`G` | first / last track |
| `enter` | play what is selected |
| `f` | order the playlist |
| `/` | filter the playlist |
| `alt+up` / `alt+down` | move a whole record up or down |
| `ctrl+s` | save the playlist |

### Filtering with `/`

`/` narrows the playlist to the rows that match what you type. Every word has
to be found somewhere in the artist, title, album, year or file path, in any
order and any case.

- `enter` applies it; `esc` keeps whatever was in force; `enter` on an empty
  box clears it.
- Pressing `/` again opens the box with the current words in it, so a filter
  is edited rather than retyped.
- It works from any panel and brings the playlist up.
- A newly applied filter starts at its first match, wherever the playing track
  or old viewport sat.
- The music is not filtered. A hidden track still plays when its turn comes;
  only what the list shows changes, and the title says what it is showing.

### Tagging rows

Tagging is how a queue gets rearranged in bulk: mark rows anywhere in the
list, including across playlists, then copy, move or delete them in one go.
While any row is tagged the available commands are shown across the top of
the player, where they do not scroll away with the list.

| Key | Does |
| --- | --- |
| `t` | tag this row |
| `T` | clear every tag |
| `y` | copy the tagged rows |
| `u` | put them here |
| `m` | move them here |
| `del` / `D` | remove them |

## Library browser

| Key | Does |
| --- | --- |
| `l` | open the browser (`esc` closes it) |
| `left`/`h`, `right`/`l` | change column |
| `/` | search it |
| `space` | add the selection to the queue |
| `a` | add the whole record |

## Windows and focus

| Key | Does |
| --- | --- |
| `p` | playlist on or off |
| `i` | album window on or off |
| `alt+g` | equalizer panel on or off |
| `alt+s` | Activity panel on or off |
| `alt+e` | choose a playlist |
| `alt+i` | choose a cover |
| `alt+r` | look the cover up again |
| `alt+1` … `alt+5` | focus player, playlist, equalizer, album, activity |
| `tab` / `shift+tab` | next / previous pane |

Each panel has its own keyboard once it has focus, and the focused panel's
whole border is drawn in the highlight colour so it is clear which one is
listening.

### Album panel

| Key | Does |
| --- | --- |
| `left` / `right` | previous / next cover candidate |
| `o` / `enter` | open the full-size original |
| `c` | open the chooser |
| `r` | look the cover up again |

### Equalizer panel

| Key | Does |
| --- | --- |
| `up`/`down`, `j`/`k` | select a filter |
| `left` / `right` | gain by 1 dB |
| `shift+left` / `shift+right` | gain by 10 dB |
| `enter` | bypass the selected filter |
| `a` | add a peaking filter |
| `d` | remove the selected filter |

The panel's `settings` menu has exact pasted values, filter type and channel
selection, reordering and duplication, managed-profile creation, rename and
delete, and APO import and export. See [Equalizer](equalizer.md).

### Activity panel

`j`/`k`, the arrow keys, page keys and `home`/`end` scroll the recent-listen
log instead of moving the playlist cursor. The panel shows five listens at a
time and retains the latest 100 for navigation.

## Equalizer, appearance, general

| Key | Does |
| --- | --- |
| `e` | equalizer on or off |
| `[` / `]` | previous / next preset |
| `w` / `W` | next / previous visualizer |
| `+` / `-` | bar width |
| `alt+t` | next theme |
| `a` | animations on or off |
| `esc` | close whatever is open |
| `?` / `F1` | help |
| `q` | quit |

## Mouse

The pointer works everywhere the keyboard does.

### Playlist

| Gesture | Does |
| --- | --- |
| wheel | scroll the list |
| click | select a track |
| double click | play it |
| right click | track actions |
| click `sorting` | order the playlist |
| click `gravity` | create a related list |

Right-clicking a row opens its track actions:

- add it to another playlist
- remove it from this one
- create a Gravity list seeded from it
- create a playlist of everything by its artist
- build a library timeline

Adding to another playlist writes that M3U immediately. A starred destination
already contains either the same library entry or a likely alternate encoding
of it; STAR/AMP warns before adding a duplicate and can replace an alternate
version at the same playlist position. Removing a track from the current
playlist is an unsaved queue edit, and `ctrl+s` makes it permanent.

### Player

| Gesture | Does |
| --- | --- |
| click a transport button | that button |
| click `SHUF` / `REP` / `x FADE` in the footer | toggle it |
| click the visualizer name in the footer | next visualization |
| click or drag the bar | seek |
| wheel over the bar | seek 5 seconds |
| click or drag `VOL` | set the volume |
| wheel over `VOL` | volume by 5% |
| click the analyzer | next visualization |
| right click anywhere | play or pause |

### Equalizer

| Gesture | Does |
| --- | --- |
| click `[ON ]` | enable or bypass |
| click the chevrons | change preset |
| click a filter | select it |
| wheel over the panel | previous / next filter |

### Album

| Gesture | Does |
| --- | --- |
| click the cover | open the full original |
| click `<` / `>` | previous / next candidate |
| wheel over the cover | previous / next candidate |
| click album / artist | open its music database page |
| click `retry` | look the cover up again |

### Overlays

| Gesture | Does |
| --- | --- |
| click outside | close the picker |
| click `close` on a header | close that panel |
| click `settings` on a header | what that panel controls |
