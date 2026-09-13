# Security

## Reporting

Use GitHub's private vulnerability reporting on this repository
(Security → Report a vulnerability). Please do not open a public issue for
something exploitable.

I work on this in my spare time, so expect a first reply within a week rather
than a day.

## Threat model

Who the attacker is, at each place staramp takes input from somewhere else.

| Boundary | In scope |
| --- | --- |
| **Files in your library** | Yes. Tags, cue sheets, embedded images and audio bitstreams all come from files you did not write. A malformed one that crashes the player is worth reporting; one that gets code running is worth reporting urgently. |
| **Things you downloaded** | Yes. Winamp `.wsz` skins and Equalizer APO presets are files people swap. They may not read outside their own directory, exhaust memory, or reach the audio device with a value that has no musical meaning. |
| **A remote library** | Yes. The machine at the other end of `staramp remote` is not trusted: its index is a database this program parses, and its `sftp` replies are packets this program decodes. |
| **Web services** | Yes, as far as they can reach. MusicBrainz, the Cover Art Archive, Last.fm and ListenBrainz can return anything; a MusicBrainz URL relationship is editable by anyone with an account, and one of those becomes a link you may click. |
| **Other local users** | Yes on a shared machine. The control socket is checked against your uid, and staramp's own directories are private to you. |
| **The person running staramp** | No. A path you type, a playlist you open and a library root you configure are your own authority, and the control socket confers exactly that authority and no more. |
| **The build** | Yes. What the release workflow downloads is pinned and checksummed, and what CI runs is pinned to commits. |

A dated review of this surface, including what was found, what was fixed and
what was deliberately accepted, is in
[docs/security-review.md](docs/security-review.md).

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

Nothing here listens on a network port. The control socket is a Unix socket --
abstract on Linux, a path under the staramp directory on macOS -- and is
reachable only by the user running staramp: connections are checked against
that uid on Linux, and the socket's directory is private on macOS. It confers
the authority that user already has, which is the reason it can open any
playlist path they name. A remote library runs the user's own `ssh` and never
installs or starts anything on the far machine. Discord presence talks only to
the local desktop client's socket, and scrobbling and cover lookups are off
until turned on.

## What is not a vulnerability

- Playing a file you asked it to play, with the tags it contains.
- `[art] fetch = true` sending an artist and album name to MusicBrainz. That is
  what the setting does, and it is off by default for exactly this reason.
- A scrobbling provider or Discord receiving the track you asked STAR/AMP to
  play, once you have turned that provider on.
