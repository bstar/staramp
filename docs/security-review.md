# Security review, September 2026

star/amp parses a great deal of input it did not write and did not ask for.
That is not incidental to what it is: a music player for a local collection
reads tags from files of unknown provenance, cue sheets in whatever encoding
the ripper used, playlists handed over with an album, skins downloaded from
fan sites, presets swapped in forums. Given a remote library it also reads an
SQLite index and a stream of packets from another machine, and it launches
`ssh`, an image viewer and a browser.

This is what a review of that surface found, what was changed, and what was
deliberately left alone. It covers version 0.1.0 as released, and every fix
named here is on `main` with a regression test.

## Method

Three independent passes over the code, one per boundary group: the parsers of
untrusted input; the filesystem, subprocess and socket boundaries; and the
network clients, build and dependencies. Each finding was verified against the
source before it was accepted, and each proposed fix was checked against the
actual APIs of the locked crate versions before it was written. Findings that
turned out to be wrong are recorded below along with the rest.

## Trust boundaries

| Boundary | What the attacker controls |
| --- | --- |
| Music files | tags, embedded images, audio bitstreams |
| Sidecar text | cue sheets, `.m3u` playlists |
| Downloaded artifacts | `.wsz` skins, Equalizer APO presets |
| A remote library | SFTP packets, directory listings, the whole `index.sqlite` |
| Web services | MusicBrainz, Cover Art Archive, Last.fm, ListenBrainz responses |
| Other local users | the control socket, the Discord socket, directory modes |
| The build | what the release workflow downloads and what CI runs |

## What was found and fixed

### The control socket was open to every local user

On Linux the socket is bound in the abstract namespace, which is scoped to the
network namespace rather than to a user, has no permission bits, and took a
name derived from a path whose only variable is the uid. Any account on the
machine could connect, drive playback, and read the queue back with every file
path in it. `SECURITY.md` claimed the opposite.

`SO_PEERCRED` is now checked on accept and a connection from another uid is
dropped. On macOS, where the socket is a real file, its directory is 0700 and
the socket 0600. The data directories are 0700 as well: the index lists every
path in the library and the activity database is a complete listening history,
and both were world-readable.

### A preset could hand the output device full-scale noise

`Preamp: 400` is a legal line in an Equalizer APO preset, and nothing checked
it beyond being finite. It compiled to a hundred-billion-fold multiplier whose
first stop was the audio device. Four things were missing and each is fixed:
the range is refused where the preset is read; it is clamped again at compile
time; the ten-band preamp, which multiplies all ten of its already-clamped
bands, was never clamped at all; and coefficients are checked to be finite,
because a NaN one is not a wrong sound but a permanent one.

A soft limiter now runs before the output ring, but only where something could
have pushed the signal past full scale. A transparent chain at or below unity
gain is untouched, so the bit-perfect path stays bit-perfect. There is a test
that says so.

### `Include` and `.wsz` were unbounded

An APO `Include` took any path, so a preset could read a file elsewhere on the
machine and return its lines through parse errors; its size limit was applied
after the read, so `Include: /dev/zero` never returned; and because a diamond
is not a cycle, thirty-two files that each include the next twice is four
billion parses. Includes are confined to the preset's own directory, measured
before reading, and parsed once each.

Skin archives read three entries with no size limit at all, so a ten-megabyte
`.wsz` whose `VISCOLOR.TXT` inflates a thousandfold was ten gigabytes resident.
Declared sizes are checked, reads are capped in case the header lies, and the
refusal is a visible warning rather than a silent fallback. Every image decode
now goes through one helper with explicit dimension limits.

### Library URIs could leave the library

A URI in the index or a playlist is library-root-relative by definition, and
the one function that turned it into a path honoured absolute paths and never
looked at `..`. A line of `/etc/shadow` in an `.m3u` was opened and handed to
the decoder. URIs are now confined; files named on the command line are not,
because those were chosen by the person typing them. An entry that cannot play
is still preserved byte-for-byte on save, because silently dropping somebody's
playlist line is its own bug.

### The remote index was parsed without precautions

It is a SQLite database downloaded from another machine and parsed in-process.
Read-only connections now run with `DEFENSIVE` on, `trusted_schema` off,
triggers and views disabled, and `cell_size_check` on; none of that costs
anything here, since the schema uses no views or triggers and FTS5 marks its
own virtual table innocuous. The schema version is checked, and both downloads
are bounded by the size the far end advertised.

### Smaller things

- The Metal Archives host check ended the authority at the first `/`, so
  `https://evil.example?x.metal-archives.com` passed it and opened in the
  user's browser. That URL comes from a MusicBrainz relationship anyone can
  edit.
- All three HTTP agents are https-only with bounded redirects.
- The command line printed tag text, cue titles and playlist lines with no
  control-character filtering. The TUI was never affected: ratatui drops them.
- `staramp query 'é!'` panicked, because the lexer recorded character offsets
  in spans documented as byte offsets. The parser also recursed without bound.
- Integer overflow now panics in release rather than wrapping silently.
- The import staging directory was created in a way that could be pre-empted,
  and copied file modes wholesale.
- The AppImage runtime was downloaded from a rolling tag with no checksum; it
  is the first code that runs when anyone opens an AppImage. It is pinned and
  verified.
- Every CI action is pinned to a commit rather than a tag.

## What was already right

Worth recording, because a review that only lists problems misrepresents the
code it reviewed.

The SFTP wire parser bounds packet length before allocating, uses checked
arithmetic and non-panicking slicing throughout, cross-checks DATA framing,
and drains replies it has refused. The custom ID3 reader bounds every index and
cannot loop on a zero-size frame. A cue sheet's `FILE` reference is reduced to
its basename before any filesystem use, so traversal was never possible. Skin
archives never write an entry to disk, which makes zip-slip structurally
impossible. The smart-playlist compiler binds every value as a parameter,
whitelists fields and sort keys through a closed enum, and escapes `LIKE`
patterns. The `ssh` invocation puts `--` before every host, sets
`BatchMode=yes`, and neutralises forwarding and remote commands. Imports
validate containment by canonical path and refuse symlinks at every level of
the copy. Identifiers are UUID-checked before they are interpolated into any
URL, and URL parameters go through a strict allowlist encoder. Credentials are
written 0600 before the first byte reaches the file, and no secret is logged or
placed in a query string. There are three `unsafe` blocks in sixty thousand
lines and all three are sound.

## Accepted risks

- **The control socket confers the user's own authority.** With the peer check
  in place, a client is the same user, and that user can already read their own
  files. `load-playlist` therefore still accepts any readable path, because
  confining it would break `staramp ui /somewhere/else.m3u` joining a running
  session, which is a documented flow.
- **TLS roots are compiled in** rather than taken from the system store, so
  enterprise CAs and OS-level revocation do not apply. For four fixed public
  endpoints this is a reasonable trade, but it means refreshing roots needs a
  release.
- **MD5 in the Last.fm signature** is what that protocol specifies.
- **The AppImage bundles an ffmpeg frozen at the end of Debian bullseye's LTS**
  (August 2026) and will not receive further security fixes for it, while
  decoding untrusted media is exactly what it does. The `.deb`, Arch and Nix
  packages all use the system ffmpeg and are the better choice for anyone who
  cares about that.
- **A local user can still observe that staramp is running** and see socket
  names. Nothing hides that, and nothing tries to.

## Out of scope

The internals of libav, symphonia and SQLite. staramp links all three and
feeds them untrusted bytes by design; their own hardening is theirs. The
decode path is where a malformed audio file is most likely to cause trouble,
and the mitigation there is the same as everyone else's: keep the libraries
current, which the system packages do and the AppImage does not.

## Reporting

See [SECURITY.md](../SECURITY.md).
