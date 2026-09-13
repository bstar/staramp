# Playback

STAR/AMP decodes everything a lossless collection tends to contain, opens the
device at the file's own rate, and treats a cue-sheet album as the single file
it is.

## Formats

| Decoder | Formats |
| --- | --- |
| [symphonia](https://github.com/pdeljanov/Symphonia), pure Rust | FLAC, MP3, Vorbis, AAC, ALAC |
| libavformat / libavcodec, linked in-process | Monkey's Audio, WavPack, Musepack, DSD, Opus, WMA |

Monkey's Audio, WavPack, Musepack and DSD are common in lossless collections
and are missing from nearly every pure-Rust or pure-Go player. STAR/AMP links
libav **in-process** for them. Not a subprocess: real seeking, no respawn, no
`ffmpeg` on `PATH` at runtime.

Two diagnostics tell you what a file gets and what it decodes to. Both are
described in [the command line](cli.md):

```sh
staramp probe  <file>              # codec, rate, depth, channels, length
staramp decode <file> -o out.wav   # the samples, without touching the device
```

## CUE sheets

Single-file album rips with a `.cue` are the norm for EAC and vinyl rips. In
the reference library, **27% of curated playlist entries** are cue virtual
tracks.

- Each track in a sheet is a first-class track in the playlist and the index.
- MPD's `Album/rip.cue/track0007` URI form is read and written, so playlists
  stay compatible.
- A cue album plays as one linear read of one file, which is both perfectly
  gapless and the fastest access pattern there is.
- `[cue] pregap` in the [configuration](configuration.md) decides which track
  a pregap belongs to.

## Bit-perfect output

The device is opened at the file's own sample rate, so anything the device can
take is played untouched. The header says `bit-perfect` when that is the case.

Where the device cannot follow the file, the conversion happens on the way
into the output ring and the header says so:

| Situation | What happens |
| --- | --- |
| the device cannot do the file's rate | resampled to a rate it can |
| the file has fewer channels than the device | copied across the device's channels at unity gain |
| `[output] mode = "fixed"` | everything is converted to `fixed_rate` and the device is never reopened |

> [!NOTE]
> On a typical Linux desktop cpal opens ALSA's `default`, whose `plug` layer
> advertises every rate and converts underneath, so native mode almost never
> needs to convert. It matters wherever the device is reached directly: a
> `hw:` device, a plugless `.asoundrc`, and always on CoreAudio.

## Gapless albums

While album sorting is active, adjacent tracks from the same release are
opened and their first PCM block is decoded ten seconds early. The prepared
samples join the existing output ring without reopening the device.

Shuffle, play-next and ordinary playlist order do not trigger this work.
Native output still has to reopen when an album changes sample rate or
channel count; fixed-rate output stays continuous across those changes by
converting them.

## Crossfade

`F`, or a click on `x FADE` in the footer, blends the last five seconds of a
track into the next one. The choice is saved as `[playlist] crossfade`.

## ReplayGain

| Setting | Does |
| --- | --- |
| `[replaygain] mode = "album"` | levels records against each other using the gain tags already in your files |
| `[replaygain] mode = "track"` | levels every track on its own, which is what you want under shuffle |
| `[replaygain] mode = "off"` | the default: plays everything as it is |
| `preamp` | adds a fixed amount back on top, in dB |
| `prevent_clipping` | pulls the gain down when a track's stored peak says the result would clip |

> [!IMPORTANT]
> It is **off by default**, and that is a deliberate refusal rather than an
> oversight. Turning it on changes what you hear, and in a library where only
> some files carry the tags it would turn half the collection down and leave
> the rest alone. In the reference library that is 228 tracks out of 31,928.

The gain is applied on the decode thread, at track boundaries, so a change of
setting lands on an exact sample rather than stepping the level underneath a
playing track. Running with `-v` reports the scalar as each track opens, which
is the only way to tell "no tags" apart from "no effect" by ear.

Nothing here computes gain. An EBU R128 scanner is not built yet, so files
without tags stay as they are.
