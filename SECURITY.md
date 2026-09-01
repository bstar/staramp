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
- HTTP responses from MusicBrainz and the Cover Art Archive

A malformed file that crashes the player is plausible and worth a report. One
that gets code running, writes outside the staramp directory, or escapes a
`.wsz` archive into the filesystem is worth one urgently.

Nothing here listens on a network port. The control socket is an abstract Unix
socket, reachable only by the same user.

## What is not a vulnerability

- Playing a file you asked it to play, with the tags it contains.
- `[art] fetch = true` sending an artist and album name to MusicBrainz. That is
  what the setting does, and it is off by default for exactly this reason.
