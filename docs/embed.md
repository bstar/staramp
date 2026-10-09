# Embedding the player

`staramp embed --stdio` runs a small, independent player for a host terminal
application such as STAR/FOLD. It reads and writes one JSON object per line on
standard input and output. Protocol version 1 begins with a `hello` response;
the host should check `protocol == 1` before sending anything else. Standard
error may carry diagnostics, but standard output is reserved for protocol
messages.

This mode does not open the ordinary STAR/AMP window, control socket, library
index, listening history, scrobblers, Discord presence, or session files. It
does not save standalone settings. Only an explicitly selected embed profile
may save player presentation styles (described below). Ending the process ends
only its own playback. The host supplies absolute local media paths and handles
browsing.

Rendering and input geometry remain STAR/AMP-owned. The embed path paints the
ordinary `PlayerView` offscreen, uses STAR/AMP's transport rasterizer and
hit-testing geometry, and sends cells plus optional image pixels to the host.
The host only forwards playback input and paints those results; it does not
recreate player controls or decide where a transport button lives.

## Requests

Send `configure` before `play`. A new configuration can resize the player,
change its focus, or change the host palette. Its `generation` identifies the
current view; frames and play requests carry it so a host can discard stale
results after a resize or a new selection.

```json
{"type":"configure","generation":1,"width":64,"height":10,"focused":true,"theme":{"bg":[10,20,30],"fg":[230,230,230],"muted":[140,140,140],"accent":[200,120,60],"selected":[40,50,60],"border":[80,80,80],"error":[240,60,60]}}
{"type":"play","generation":1,"paths":["/mnt/music/01.flac","/mnt/music/02.flac"],"index":0}
{"type":"control","action":"toggle"}
{"type":"pointer","x":3,"y":8,"button":"left"}
{"type":"shutdown"}
```

Hosts with terminal image support may add
`"graphics":{"cell_width":8,"cell_height":16}` to `configure`, using the
terminal's actual pixel dimensions per cell. Omitting `graphics` preserves
text-only behavior. The optional `"transport_images"` capability is
advertised in `hello.capabilities`; hosts should check it before sending
`graphics`. Protocol version 1 is unchanged.

Hosts that see `"player_styles"` in `hello.capabilities` may add
`"profile":"starfold"` to `configure`. The profile name is fixed for a child
process and accepts only 1–64 ASCII letters, digits, hyphens, or underscores.
Without a profile, style changes last only for the child process. With one,
STAR/AMP loads `$STARAMP_CONFIG_DIR/embed/starfold.toml` (or the corresponding
default config root) once and saves it atomically only after an explicit style
change. Its version-1 TOML fields are `version = 1`, `visualizer = "peaks"`, and
`seek_style = "bar"`. Missing profiles inherit the read-only standalone
preferences; malformed profiles are left untouched, and the player continues
with an unsaved style and a nonfatal `notice`. Repeated `configure` requests
with the same profile preserve the current style. The standalone config,
history, and session files remain untouched.

`play` replaces the embedded queue and starts the item at the zero-based
`index`. Paths must be absolute UTF-8 filenames with supported audio file
extensions. The player rejects non-regular sources, including FIFOs, rather
than waiting for them to produce data. The `hello.extensions` list is the authoritative set to offer in
a file picker. A malformed or unsupported request receives an `error` response
and leaves the service available for another request.

`control.action` accepts `toggle`/`toggle_pause`, `play`/`resume`, `pause`,
`stop`, `next`, `previous`/`prev`, `repeat`, `seek`, `seek_by`, `volume`, and
`volume_by`. `seek` is an absolute position in seconds; `seek_by` is a signed
offset in seconds. `volume` is 0 to 1, and `volume_by` is a signed delta.
These four numeric actions require a `value` number. The host can map its
playback keys to these actions.

When `player_styles` is advertised, the host may also send `next_visualizer`,
`prev_visualizer`, or `next_seek_style`. STAR/AMP cycles the same native
visualizer and seek-style sequences as its standalone player; waveform, scope,
and fluid modes use its own audio analysis and `PlayerView` renderer.

`pointer` coordinates are zero-based cells within the configured body. `left`
activates transport, seek, and volume targets; `drag` adjusts only seek and
volume. `scroll_up` and
`scroll_down` change volume by 0.05 except over a visible visualizer, where
they cycle visualizer modes. A left click on the visible visualizer cycles to
the next mode; a right click on it or the seek row cycles the seek style.
Compact rows that show a synthetic clock instead of the visualizer are not
visualizer targets. Other buttons and clicks outside targets have no effect.
`shutdown` or closing standard input stops the embed process.

## Responses and drawing

The first response is
`{"type":"hello","protocol":1,"extensions":[...],"capabilities":["transport_images","player_styles"]}`.
The service then sends `status` when playback state or title changes. A status
contains `playing`, `paused`, `title`, and an optional absolute `path` for the
current track, which lets the host open that track externally after next or
previous. It sends `error`
for a failed request or playback problem, and `frame` while configured. A frame
has `generation`, `width`, `height`, and exactly `width * height` row-major
`cells`. Every cell has `symbol`, RGB `fg` and `bg` triples, and `modifiers`,
the ratatui modifier bitfield. Frames arrive at about 30 per second while the
process is active; the host should render the newest complete frame.
`notice` reports a nonfatal embed-profile load/save problem; it never stops
playback or suppresses later frames.
With graphics configured, a frame also carries up to five `images`, drawn
after the cells. Each image has body-relative cell `x`, `y`, `width`, `height`,
exact `pixel_width = width * cell_width` and `pixel_height = height * cell_height`,
and flat `rgba` bytes (four per pixel). They are the same rasterized transport
controls as the standalone player, colored for the current theme and play
state. Text controls remain under every image as a fallback; no terminal escape
sequences cross the protocol. At five body rows, buttons are rasterized into
their single visible row, aligned with pointer targets.
Embedded title and artist tags are read only for the current track, not for
every path in the queue.

An explicit transport stop (a `stop` control or click on the stop button)
also sends `{"type":"stopped"}`. Hosts can use this to close the embedded
player and restore ordinary previews. A paused track or decoder failure is
not a `stopped` event.

The frame is only STAR/AMP's main player body. At 10 rows it includes the
clock, analyzer, title, metadata, seek bar, transport and volume from the
normal player panel. Shorter heights keep a compact version; five rows still
show a readable clock, title, metadata, seek and transport with volume. The
host paints its own panel border and title. Artwork, library, playlist and
listening history are outside this embed interface.

The service accepts at most 240 columns by 20 rows, 2,048 queue paths, and
1 MiB per input line. Responses are limited to 2 MiB. A host should validate
frame dimensions and `generation` before drawing.
Graphics cells must be 1–64 pixels wide and 1–128 pixels high. An image is
capped at 65,536 pixels, and all RGBA images at 262,144 bytes per frame. When
a terminal cell size exceeds an image budget, the image is omitted and its
text control remains usable.

### Native surfaces (experimental)

A helper advertising `native_surface_v1` accepts `native_surface: true` alongside
`graphics` in Configure. Frames then contain an empty `cells` array and a
`surface` using STAR/KIT's bounded pixel primitives. The surface owns its title,
clock, spectrum, transport, seek and volume layout. STAR/FOLD only places it.
Pointer coordinates remain terminal cells: STAR/AMP maps each cell's center to
its own surface hit regions. This does not claim pixel-precise terminal input.

`visible: false` suspends frame publication while playback/status continue.
Resizing, theme and focus updates reuse Configure and its generation guard.
Local frame cadence is 30 FPS; an SSH environment uses 15 FPS. Hosts coalesce
frames into the existing single latest-frame slot. Omitting the extension keeps
the original cell/transport-image protocol. No web engine is involved.

## Nix audio runtime on Linux

The Nix package includes ALSA's dynamic PipeWire and PulseAudio plugins. Its
wrapper supplies `ALSA_PLUGIN_DIR` unless the caller already set one. A raw
Cargo executable outside the dev shell needs the same plugins:

```sh
nix build .#alsa-plugins --out-link target/alsa-plugins
nix develop -c cargo build --release
scripts/run-local.sh embed --stdio
```

`scripts/run-local.sh` is a generic launcher for this checkout, so embedding
hosts can put a symlink to it on their private PATH. On macOS, audio uses
CoreAudio and the plugin package is not needed.

An optional real-device test exercises the native clock, pointer transport and
seeking with silent audio and isolated settings:

```sh
python3 scripts/test-native-playback.py --binary scripts/run-local.sh
```

This requires a working local audio session and complements the device-free CI
tests. It does not prove audible output or a remote machine's audio setup.

## SSH audio relay (experimental)

`audio_relay_v1` advertises an alternative output, selected with Control
`audio_relay` and value `1` (client) or `0` (host). The player keeps its queue,
position, pause state, DSP and volume; client mode opens no host sound device.
It outputs 48 kHz stereo signed 16-bit PCM after DSP. This mode is not
bit-perfect playback. Closing the embedded child stops its relay output.

An `audio_open` response contains an opaque `epoch`. Each `audio` response
contains the same epoch and `samples`, an interleaved signed-integer array
with at most 1920 samples (20 ms). Initially no audio is sent: the host grants
up to eight blocks with Control `audio_credit`. Each emitted block consumes
one credit; queued and granted credits are bounded. The host returns credits
only as its client consumes data. Controls and visual frames continue when
credits stop, so pause, seek and shutdown stay responsive on a stalled link.

Seeking and selecting the client route again create a fresh epoch. The host
must discard prior blocks and forward the new open before granting credits.
Re-selecting client output lets a reattached frontend replace an old stalled
stream. Older hosts must not send these controls without the advertised
capability. Ordinary `staramp`, cell embedding and host output are unaffected.

## Render-only video transport

`staramp embed --stdio --transport` renders the same native transport buttons
and volume slider for playback owned by a video host. It opens no player or
sound device and writes no user files. Each bounded JSON line contains pixel
`width`/`height`, the usual `theme` palette, `playing`, `paused`, `volume` (0–1),
and optional `pointer: [x, y]`. A rendering response contains `surface`; a
pointer response contains `action` and an optional normalized volume `value`.
Actions are `previous`, `play`, `pause`, `stop`, `next`, and `volume`. The host
assigns previous/next semantics (FOLD seeks five seconds). EOF shuts it down.
AMP owns both transport geometry and pointer hit testing.

### Cached native player skin

A helper built with terminal graphics advertises `native_skin_v1`. A supporting
host may add `native_skins: true` to Configure only when its own frontend
advertises KIT's `native_skins` capability and `native_surface` is also enabled.
The helper returns the same AMP-owned player component as standalone playback,
with content-addressed PNG assets and Sprite primitives inside its Surface.
Hosts validate/place that surface and forward pointer input; they do not draw
AMP controls. KIT transfers artwork reliably and caches it locally over SSH.

The helper includes payloads in its latest-frame protocol so discarding an
obsolete helper frame cannot lose an asset. KIT strips repeated payloads at the
network session boundary. Old hosts omit the flag and receive ordinary native
primitives. Text is shaped at the chosen physical density; finished frames and
text are not resampled. Short/narrow embedding regions use the existing compact
presentation. Configuration changes clear the cached input surface, and pointer
hit testing uses the last surface actually emitted by the helper.
