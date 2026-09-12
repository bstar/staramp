# A library on another machine

The music is on the machine under the desk; you are on the laptop. star/amp
opens one SSH connection and plays the files through it.

```sh
staramp remote music-server
```

That is the whole setup.

## What it needs

- `ssh music-server` already works without asking you anything. Use a key or
  an agent.
- star/amp is installed and has been scanned on the far machine, so there is
  an index to copy:

  ```sh
  # on the machine with the music, once
  staramp scan ~/Music
  ```

There is **nothing to install or leave running** on the far machine beyond
star/amp itself: no daemon, no listening port, no streaming server. `sshd`
and its `sftp` subsystem do all of the work, and your own `~/.ssh/config`,
keys and agent do all of the deciding. star/amp runs `ssh`; it does not
reimplement it.

Write the host down and `staramp remote` takes no arguments:

```toml
[remote]
host = "music-server"
root = "~/Music"
```

## What crosses the link

Two things, treated completely differently.

| | The index | The audio |
| --- | --- | --- |
| copied? | once, then re-fetched when the far copy changes | never |
| size | small: 31 MB describes the 1.1 TB reference library | read a window at a time, as it plays |
| after that | every browse, search and smart playlist runs against local SQLite at full speed | roughly forty-five seconds of the compressed file is kept ahead of the decoder |

The read-ahead is what a link that hiccups is ridden out on: a blip shorter
than the buffer is inaudible. Seeking inside what is already buffered, which
is most scrubbing, costs no network traffic at all.

Nothing but the index and album art thumbnails is written to the laptop's
disk. Your listening history stays local to the laptop and survives the index
being replaced.

## Both directions

A Mac can be either end: the machine playing, or the machine holding the
files. The index records paths exactly as the filesystem gave them, byte for
byte and never normalised, which is what lets an index built on APFS, where
filenames are commonly NFD, resolve correctly when it is served to Linux, and
the reverse.

## What it does not do

- **Transcode.** It never will. The bytes on the far disk are the bytes the
  decoder sees, so a WavPack cue album played over SSH is the same
  bit-perfect read it would be locally.
- **Ask for a password.** There is a full-screen UI on the terminal and
  nowhere to put a prompt. If the host key is unknown, star/amp says so and
  tells you to run `ssh <host>` once by hand.
- **Analyze audio for Gravity on the laptop.** A remote library is analyzed
  on the machine that owns it, so no whole file is pulled across the link to
  fill a cache.
