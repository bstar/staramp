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
presentation while retaining the same player.

The server owns decoding, the queue, DSP and listening history. PCM travels over
the existing SSH connection to a local device; the server does not open a sound
device for this route. A frontend attaches to an existing playback owner rather
than creating a second decoder. Disconnecting pauses the persistent session;
reattaching leaves it paused until you resume. Seeking and reattaching discard
stale queued audio. A new frontend takes the audio route from an older one.

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
choice lasts for the running playback session. Device errors pause playback;
they never silently move sound to the server. Legacy frontends are asked to
update rather than silently receiving the old fixed 48 kHz/16-bit route.

DSD files are decoded to PCM at the decoder's original PCM rate. Native DSD and
DoP are outside this route. High rates and many channels require proportionally
more bandwidth; transport and playback queues are bounded and stop accepting
audio when the local frontend stops consuming it.

STAR/AMP pins the published STAR/KIT v0.4.0 release. Cargo fetches the shared
PCM and frontend implementation automatically; no adjacent KIT checkout is
needed. Remove any development `.cargo/config.toml` path override before
building this release.

On an Apple Silicon Mac with Nix installed:

```sh
git pull --ff-only
nix develop -c cargo build --release --locked
./target/release/staramp --ssh music-server
```

The native Cargo/Homebrew route, including Intel Macs, is documented in
[Installing](installing.md). Build and install this version on the music server
as well, or select its executable with `--remote-executable`.

`scripts/test-ssh-pcm.py --staramp target/debug/staramp` checks exact generated
32-bit PCM, float64 processing, credits, pause, seek, disconnect, reattach, both
presentations and the complete end-of-track drain without a sound device. Its
optional `--ssh-host` / `--ssh-config` arguments exercise a real SSH connection
to this machine. Physical listening and macOS audio hardware require a separate
check.
