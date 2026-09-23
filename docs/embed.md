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
