# Security

## Reporting

Use GitHub's private vulnerability reporting on this repository
(Security → Report a vulnerability). Please do not open a public issue for
something exploitable.

I work on this in my spare time, so expect a first reply within a week rather
than a day.

## What is worth reporting

staramp parses a lot of input it did not write, while linking libav:

- audio files and their tags, through lofty and libavformat
- CUE sheets, in whatever encoding they happen to be in
- Winamp `.wsz` skins, which are ZIP archives people download from the internet
- Equalizer APO profiles, including whatever an `Include` line points at
- a library index and audio files served from another machine over SFTP
- HTTP responses from MusicBrainz, the Cover Art Archive, Last.fm and
  ListenBrainz

A malformed file that crashes the player is plausible and worth a report. One
that gets code running, writes outside the staramp directory, or escapes a
`.wsz` archive into the filesystem is worth one urgently.

Nothing here listens on a network port. The control socket is a Unix socket
(abstract on Linux, a path under the staramp directory on macOS), reachable
only by the same user. A remote library runs the user's own `ssh` and never
installs or starts anything on the far machine. Discord presence talks only to
the local desktop client's socket, and scrobbling and cover lookups are off
until turned on.

## What is not a vulnerability

- Playing a file you asked it to play, with the tags it contains.
- `[art] fetch = true` sending an artist and album name to MusicBrainz. That is
  what the setting does, and it is off by default for exactly this reason.
- A scrobbling provider or Discord receiving the track you asked star/amp to
  play, once you have turned that provider on.
