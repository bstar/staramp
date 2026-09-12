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

When no quarantine is configured, the first form uses the sibling directory
`~/Downloads/ALBUMS-rejected`.

> [!IMPORTANT]
> The staging, library and quarantine directories must be three separate
> places. None may sit inside another.

## Reviewing

The command opens the normal player on a separately indexed, album-grouped
view of the inbox, oldest first. Navigate, filter, fold, and play it as you
would anything else.

Right-click an album heading for the two decisions:

| Action | Does |
| --- | --- |
| **import whole album** | files it into the permanent library |
| **reject whole album** | moves it to quarantine |

An import preserves the album folder name and puts it below the album-artist
directory. Existing and similarly spelled artist directories are offered
before a new one is made.

If an apparent copy of the album already exists, star/amp stops and offers to
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
confused with the permanent one.
