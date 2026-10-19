# Reviewing new music before it joins the library

`staramp import` is a dedicated review mode for music that has not earned a
place in the permanent library yet. You listen to an inbox in the ordinary
player, and import or reject whole albums from there.

## Setting it up

Configure the two roots once:

```toml
[imports]
staging_root = "/path/to/music-inbox"
quarantine_root = "/path/to/rejected-music"
```

Or name the inbox on the command line:

```sh
staramp import ~/Downloads/ALBUMS
staramp import ~/Downloads/ALBUMS --quarantine /somewhere/else
```

To review the same inbox in the native Classic graphical rack, run this in
Kitty (or a terminal with the STAR/KIT graphics integration):

```sh
staramp import --graphical ~/Downloads/ALBUMS
```

Close the existing STAR/AMP player first: import review owns playback and must
not silently join the permanent-library queue. The graphical reviewer uses a
separate import session and the same staging index, duplicate decisions, and
verified import transactions as the terminal reviewer. Right-click an album
heading or one of its tracks to import or reject the whole album, including
when its heading is folded. The graphical flag currently reviews local staging
directories; it does not start an SSH import session.

When no quarantine is configured, the first form uses the sibling directory
`~/Downloads/ALBUMS-rejected`.

> [!IMPORTANT]
> The staging, library and quarantine directories must be three separate
> places. None may sit inside another.

## Reviewing

The command opens the normal player on a separately indexed, album-grouped
view of the inbox, oldest first. Navigate, filter, fold, and play it as you
would anything else.

In graphical mode, select an album and use the visible **IMPORT** or **REJECT**
buttons at the bottom of the playlist. Right-clicking an album heading or track
also opens its album actions, even when another pane has focus. Album headings
remain usable when folded.

The artist-directory form supports arrow keys, Home/End, Delete/Backspace,
pasting and clicking to place the caret. Its **Use** and **Back** buttons let
you complete the choice with the mouse. Destination and confirmation rows
highlight on hover and support the mouse wheel.

The two decisions are:

| Action | Does |
| --- | --- |
| **import whole album** | files it into the permanent library |
| **reject whole album** | moves it to quarantine |

An import preserves the album folder name and puts it below the album-artist
directory. Existing and similarly spelled artist directories are offered
before a new one is made.

If an apparent copy of the album already exists, STAR/AMP stops and offers to
keep both versions or replace the old one. Replaced albums are quarantined.

## What an import guarantees

Every import is a filesystem transaction:

1. Every file is copied to a hidden temporary directory beside the
   destination.
2. Each copy is verified with BLAKE3.
3. The album is installed at its final path atomically.
4. The permanent index has to find the new album before the staging copy is
   removed.

A failed copy or scan leaves the source intact and nothing half-installed at
the destination. Tags are never changed to obtain a directory layout.

The inbox's own index is `imports.sqlite`, which is rebuildable and never
confused with the permanent one. After an import or rejection, its refresh runs
in the background. The review retains the selected remaining track, viewport,
folded albums and playback position instead of rebuilding the queue from the top.
