# SSH audio

Run the player on your music server and hear it on the machine in front of you:

```sh
staramp --ssh music-server
staramp --ssh music-server graphical
staramp --ssh music-server --session player --attach
```

Install matching builds of STAR/AMP on both machines. SSH uses your normal
keys, agent and configuration. `--ssh-config FILE` chooses another SSH config;
`--remote-executable /path/to/staramp` selects the server executable. An initial
folder or playlist belongs to the server: `staramp --ssh music-server ui /music`.

The same commands without `--ssh` work from an existing SSH shell when the
local Kitty STAR/KIT integration is installed. Its local `star-kit-terminal`
frontend must also use the matching KIT build. Without that bridge, the player
explains how to launch from the local machine. F9 switches terminal and Classic
presentation while retaining the same player. Plain `staramp --ssh HOST` restores
the client's saved F9 preference; a client without a saved choice starts graphical
when supported. The host's presentation preference does not override the client.
Explicit `graphical` or `ui` launches override the view for that launch.

The server owns decoding, the queue, DSP and listening history. PCM travels over
the existing SSH connection to a local device; the server does not open a sound
device for this route. A frontend attaches to an existing playback owner rather
than creating a second decoder. Disconnecting pauses the persistent session;
reattaching leaves it paused until you resume. Seeking and reattaching discard
stale queued audio. A new frontend takes the audio route from an older one.

The progress bar shows played audio in the accent color, locally received audio
in a muted silver color, and the remaining track in a charcoal groove. Matching
frontends negotiate whole-song caching: PCM downloads as fast as credits allow
into an anonymous temporary file on the listening machine. Credits return after
writing each block, independently of playback. The cache is discarded on seek,
track change, route close or disconnect; pause retains it and downloading
continues. Gain/EQ changes refresh processed audio from the audible position.
The buffered range uses received sample counts and is shared with mirroring
windows. A continuous thick bar shows played audio in the accent color and
cached audio ahead in a subtle, stationary Fine dots texture; when cached to
the end, the dotted fill reaches the right edge. The duration label also confirms “cached” or the cached
percentage. Local playback and credit refill run independently of terminal drawing,
so workspace changes do not need UI frames to keep audio supplied. Downloaded
EOF does not advance the queue until the local audio device finishes playback.

The default route preserves source sample rate, channel layout and PCM precision.
Integer sources through 32 bits and float32/float64 sources retain their values
when EQ, ReplayGain, volume and crossfade are transparent. Processing uses
float64 and sends processed blocks as float64. Integer output after processing
or precision reduction uses TPDF dither. The visualizer's reduced precision copy
never feeds playback. Device-driver and operating-system mixing can still alter
audio after the application hands it to the device.

If the local device cannot preserve the source, playback pauses with a reason.
Enable **SSH audio compatibility** in Player settings to permit rate/layout/
precision conversion, or run `staramp ctl audio-compatibility on` on the server.
The footer reports source and local output formats and labels conversion. This
choice is saved as `[output] ssh_audio_compatibility` on the server and survives
restarts. Device errors pause playback;
they never silently move sound to the server. Legacy frontends are asked to
update rather than silently receiving the old fixed 48 kHz/16-bit route.

DSD files are decoded to PCM at the decoder's original PCM rate. Native DSD and
DoP are outside this route. High rates and many channels require proportionally
more bandwidth and temporary disk space. Transport and playback memory remain
bounded; the temporary file can hold the whole current song. Older PCM frontends
retain their bounded playback credit window.

STAR/AMP pins the published STAR/KIT v0.5.0 release. Cargo fetches the shared
PCM and frontend implementation automatically; no adjacent KIT checkout is
needed. Remove any development `.cargo/config.toml` path override before
building this release.

On an Apple Silicon Mac with Nix installed:

```sh
git pull --ff-only
nix develop -c cargo build --release --locked
./target/release/staramp --ssh music-server
```

To update and build a remote checkout after pushing changes, run from this
checkout on the Mac:

```sh
python3 scripts/build-ssh-host.py bstar@192.168.86.57 \
  --repo /home/bstar/projects/star/staramp-graphical
```

The command uses SSH, fast-forwards a clean remote `main`, verifies it matches
the local commit, builds the locked release through Nix and removes compiler
intermediates while preserving the executable, then restarts the host player.
It needs Nix, Git and Python 3
on the server and the Rust build hygiene cleanup script locally (override with
`--cleanup-script PATH`). `--control-path PATH` reuses an authenticated SSH master.
The restart saves a recovery snapshot and verifies the new executable is running.
Reattach the Mac frontend and resume the saved session afterward. Pass
`--no-restart` to build without interrupting an existing player.

After rebuilding the Mac executable, restart the Mac frontend too. An existing
process continues running its old code even when SSH reconnects to an updated
server. The build command checks running Mac executable inodes and reports an
error if a frontend still uses the old build.

The native Cargo/Homebrew route, including Intel Macs, is documented in
[Installing](installing.md). Build and install this version on the music server
as well, or select its executable with `--remote-executable`.

`scripts/test-ssh-pcm.py --staramp target/debug/staramp` checks exact generated
32-bit PCM, float64 processing, credits, pause, seek, disconnect, reattach, both
presentations and the complete end-of-track drain without a sound device. Its
optional `--ssh-host` / `--ssh-config` arguments exercise a real SSH connection
to this machine. Physical listening and macOS audio hardware require a separate
check.

The cache texture also remains dotted in graphical compatibility layouts without
skin assets. Large legacy viewports bound the drawing work by increasing dot
spacing; ordinary player sizes use the selected fixed 3×3 grid.

The visualizer follows the local device's played-frame acknowledgements, rather
than the download cursor. The host retains a temporary mono visualization copy
on disk for the current cache epoch, read at the audible position. Completing a
whole-song download must not freeze the spectrum at the last decoded packet.
Paused/stopped playback and initial buffering clear bars, peaks, and waveform;
a device clock stationary for 350 ms also clears them. This grace period permits
ordinary SSH progress feedback gaps without flashing the spectrum off each frame.
The hardware callback remains lock-free and does no visualization file I/O.

Run `python3 scripts/test-ssh-visualizer.py target/release/staramp` to exercise
this through a real, isolated playback owner without hardware or an SSH server.
It drains an entire generated song through the relay before advancing the
simulated device clock, then verifies frequency changes, pause, resume and stalls.
