# Native graphical upgrade

## Design review

### Border alternatives

[Border treatments](treatments/borders.html) compare Previous bevel, Fine bevel,
Soft contour and Thin outline after the user rejected the latest stepped edge.
Only the outer frames vary around a fixed native capture. Player and paired
Album/Activity contexts retain the muted green, silver transport controls and
blue-gray panel shading. The page has fit/1:1 modes, 3x corner details, a latest
version comparison, focused option links and self-contained SVG/1x/2x PNG
exports. The user selected **D — Thin outline**, now applied to the native Silver
frames. All module headers share EQ styling. Regenerate the original studies with
`python3 scripts/design/border_treatments.py`; the capture is kept separately in
`treatments/borders-native-rack.png` so later production captures cannot change
the review. Chromium checks cover desktop/mobile bounds, matching comparison
size, context/filter controls, keyboard navigation, hash links and all exports.

### Album and Activity alternatives

[Paired pane comparisons](treatments/panes.html) show Refined classic, Compact
ledger, Artwork first and Split rail layouts. Each uses the current palette,
Minimal action buttons and the same sample metadata/service states. Panes are
shown at 592×320 logical pixels, with separate Album/Activity views, keyboard
focus, sample artwork-counter interaction and original 1x/2x exports.
The user selected **B — Compact ledger**, then requested larger artwork. Native
Album uses a 160px artwork slot with the image selector beside it, a three-column
facts row and minimal footer actions. Activity uses a service strip and six-row
ledger. Playback transport controls retain their previous treatment. Regenerate with
`python3 scripts/design/pane_treatments.py --cover /path/to/cover.jpg`; omitting
the cover uses a placeholder. Desktop/mobile layout, text bounds, controls and
export links were checked in Chromium.

### Button alternatives

[Interactive button comparisons](treatments/buttons.html) show Compact Classic,
Quiet bevel, Thin outline and Minimal treatments in the user's Album pane,
alongside Player, Playlist and EQ controls and interaction states. All four
use 22px button height and 11px labels, compared with the current 29px controls.
The page includes the supplied current-pane reference and 1x/2x PNG and SVG
exports. The user selected **D — Minimal** on October 10, with the five playback
transport buttons explicitly kept unchanged. Native action buttons now use
transparent faces, compact lettering and interaction framing. The gallery
retains the four original studies for comparison.

Regenerate with `python3 scripts/design/button_treatments.py`. Optional
`--cover /path/to/cover.jpg` and `--reference /path/to/current-pane.png` retain
the review's original cover and current-pane comparison; without them the
generator uses an artwork placeholder and omits the current screenshot.

### Revised direction: Winamp Classic

The original modern treatments were rejected as insufficiently Winamp-like.
The selected faithful direction is **Classic stacked rack**, with square
transport buttons (equal width/height and rigid button corners), including
the matching FOLD embed. Panel corner configuration remains independent.
The selected rack's EQ deliberately deviates from the original ten-slider
study: use a logarithmic response plot, draggable numbered filter handles,
exact frequency/gain/Q/type/channel editing, bypass/delete/add, preamp, and
visible profile selection/import/export/save-as. Plot the compiled chain's
total response including preamp, not an independently approximated DSP curve.
The study's curve is illustrative; production must use the existing response
calculation. All actions must also be accessible in ASCII and by keyboard.

The revised native EQ uses the reference's broad recessed dark-green graph,
subtle log-frequency grid, numbered handles, right-hand band inspector and
inline filter summary. Winamp Silver retains the selected Thin Outline chrome,
shared headers and Minimal action buttons. Band numbering excludes preamp.
The response is still calculated from the actual compiled DSP chain and follows
the device sample rate automatically at startup and across output-format changes.

Click **Profile** to open the searchable EQ library. Select a row and choose
**Load** (or Enter) to apply it. **Save** updates the current custom profile;
**Save As** asks for a new name and keeps the original. Built-in profiles use
Save As. The library also offers New, Rename, confirmed Delete, confirmed Revert,
Import and Export. Save and Save As always act on the current loaded curve;
Rename/Delete/Revert act on the selected row. Ctrl+S saves within the library,
Ctrl+Shift+S opens Save As, Ctrl+N creates a profile, and Ctrl+R renames the
selected custom profile. Type to search, Ctrl+U clears, and Esc closes.

Curve edits are recoverable drafts in `equalizers/working-edits.json`, retained
when switching profiles or restarting, and marked `*` / Unsaved edits. They do
not replace named profile files until Save. Save As uses exclusive publication;
renaming, deleting and saving check the saved curve for external changes. An
externally changed file requires Save As to preserve both versions. Naming
errors remain in the form so they can be corrected. Native captures and the
profile workflow are shown in `treatments/equalizer.html`.

Album refinement: larger artwork, clear album/artist hierarchy, year/count/
duration/format/genre, artwork provenance and candidate navigation, chooser,
retry, original-art opening, and explicit artist/album catalog links. Omit
unavailable metadata rather than inventing values. Artwork cycling remains
separate from clicking the cover to open its original.

Activity refinement: provider enable/configuration/account summaries, explicit
local-history state, readable two-line recent listens, listened duration,
playing/sent/queued/error delivery states, pending count, and retry action.
Use existing activity data and service actions; configuration is not proof of
network connectivity. Closing the panel must not stop recording or submission.
Sample error/queue rows in the study demonstrate states, not real user history.

APO/AutoEQ text import already exists in AMP. Direct EasyEffects JSON import
requires an additional compatibility adapter: its presets may include other
processors and filter algorithms, not just EQ values. Do not silently treat
the full processing preset as an equivalent AMP EQ. Report unsupported modes
and non-EQ processing before activation, and keep the current profile intact
on parsing/validation failure. Import gains, channel masks, filter order,
bypass, and Q only when their semantics can be represented accurately.
Format reference: https://github.com/wwmm/easyeffects/blob/master/src/equalizer_preset.cpp
The current gallery (`treatments/index.html`) presents six revised Classic
studies, generated by `python3 scripts/design/winamp.py`:

1. Classic stacked rack
2. Classic docked windows
3. Classic rounded dock
4. Classic media library
5. Classic amber rack
6. Classic amber library

Use beveled frames and transport buttons, inset green/amber displays,
seven-segment clocks, LED analyzers, narrow title strips, and dense playlist
rows. Rounded/rigid configuration remains required; Classic styling does not
remove that option. The media-library experiment also uses Classic chrome.
Each study includes the same AMP-owned FOLD player treatment.

EQ sliders illustrate a graphic-profile presentation. They must not replace
the existing ordered parametric filter editor, exact values, response curve,
or import/export functionality. All previous parity requirements still apply.
The original studies remain available in `treatments/modern-reference.html`
for comparison, but are no longer the implementation targets.

These are **design studies**, not screenshots of a working graphical release.
Production implementation and verification notes are below.

See the [ASCII reference](ascii-reference.png) and the
[action-by-action acceptance checklist](acceptance-checklist.md).

| Option | Direction | Treatment |
| --- | --- | --- |
| [01](treatments/01-stacked.png) | Faithful | Stacked reference: preserve the current panel hierarchy |
| [02](treatments/02-balanced.png) | Faithful | Balanced panels: album and activity share a row, giving the queue more space |
| [03](treatments/03-studio-light.png) | Faithful | Studio panels: compact side panels, larger queue; light theme example |
| [04](treatments/04-library-desk.png) | Experimental | Library desk: persistent navigation, artwork, and queue |
| [05](treatments/05-listening-room.png) | Experimental | Listening room: artwork-led layout and full-height queue |
| [06](treatments/06-queue-workbench.png) | Experimental | Queue workbench: queue, equalizer, and activity visible together |

Each study has a matching FOLD now-playing embed below the standalone view.
Artwork is an original geometric placeholder. Identical content, dimensions,
terminal typography, and palettes make the layouts directly comparable.
Panels shown are examples of visibility settings, not limits on features.
The studies consolidate play/pause into one control; both underlying actions
and existing key bindings must remain available.

Choose one faithful treatment and one experimental treatment before implementing
the presentation adapters. Existing uncommitted work in the original `staramp`
checkout is preserved; this branch starts at main `9acbe60`.

Regenerate the editable SVGs and PNGs with:

```sh
python3 scripts/design/treatments.py
```

This requires Python 3, `rsvg-convert`, and JetBrains Mono for the reference font.
The production renderer must use the terminal's actual font and metrics.

## Source evaluation

The standalone application currently renders its panels through the cell UI.
The cell embed renders the ordinary `PlayerView` offscreen and adapts its body
rows to the host's height. The graphical embed instead constructs a separate
surface in `src/embed/native.rs`; this is the main source of presentation drift.

Specific gaps found in the native embed:

- `PlayerRenderState` supplies format/technical metadata, repeat, bit-perfect,
  underruns, and visualizer layout information that the surface does not fully
  display or honor.
- The native waveform path groups waveform-like modes together. Its spectrum
  paths approximate the ASCII renderers instead of preserving each mode's
  distinct presentation and geometry.
- The cell player has shared rendering/input geometry; the native surface has
  its own layout calculations. Extending both independently invites more drift.
- Native transport primitives are also used by FOLD video playback. Replacing
  them requires video regression checks, not just music screenshots.
- Standalone startup owns history, presence, IPC, background jobs, and audio;
  introducing graphical sessions must not initialize a second playback owner
  or suppress those services when panels are closed.

### Ownership and integration

- AMP owns presentation models, music visualizers, player surfaces, transport
  actions, and standalone layouts. FOLD only supplies bounds/theme/input and
  displays the returned surface.
- KIT owns generic panels, typography, corner clipping, pointer/hover behavior,
  overlays, and the terminal graphical frontend/SSH transport. Prefer additive
  APIs and check AMP, FOLD, and CORD before changing shared signatures.
- Extract the existing application update/input cycle behind a controller;
  retain existing action handlers and service lifecycle. ASCII and native
  rendering consume the same state. The desktop experiment is a separately
  selectable layout, not a second player or forked action implementation.
- Embed remains an independent helper without standalone history/IPC services.
  Its existing cell protocol remains compatible, with native capabilities
  negotiated. Embed presentation preferences remain separate from standalone.
- Match FOLD's configurable `rounded`/`rigid` treatment and default 9 px panel
  radius. Theme colors and terminal metrics remain authoritative; do not bake
  the study's palette or font size into production.

## Parity acceptance matrix

“Required” describes implementation acceptance, not a claim of current support.
Audit keyboard actions, mouse actions, settings, and overlays independently;
the action enum alone does not cover every command.

| Capability | ASCII baseline | Faithful native | Desktop experiment | FOLD embed |
| --- | --- | --- | --- | --- |
| Play/pause/stop, previous/next, seek, volume | Existing | Required | Required | Required |
| Title, artist/album, technical data, elapsed/duration | Existing | Required | Required | Required |
| Repeat, shuffle, crossfade, output/bit-perfect/error status | Existing | Required | Required | Applicable player state |
| All seven visualizers plus off; bar widths/peaks/waveforms | Existing | Required | Required | Required |
| Seek styles, animations, transport styles | Existing | Required | Required | Existing embed-supported styles |
| Queue selection, tagging, bulk copy/move/remove, album reorder | Existing | Required | Required | Standalone only |
| Filter, sort/group, save/overwrite/save-as, duplicate handling | Existing | Required | Required | Standalone only |
| Artist/album/track library, search, add/replace/append | Existing | Required | Required | Standalone only |
| File browser, staging audition/import/quarantine | Existing | Required | Required | Standalone only |
| Gravity editor, explicit apply, explanations, queue boundaries | Existing | Required | Required | Standalone only |
| Album artwork, candidates, chooser, retry, original/catalog links | Existing | Required | Required | Standalone only |
| EQ response, filters, bypass, exact values, profiles, APO import/export | Existing | Required | Required | Standalone only |
| Activity, provider auth/retry, Discord presence settings | Existing | Required | Required | Standalone only |
| Resume, session/queue choices, existing ownership semantics | Existing | Required | Required | Independent embed lifecycle |
| Help, settings, menus, text entry, focus and status footer | Existing | Required | Required | Host-scoped actions/status |
| Themes, Alt+T footer feedback, font metrics, panel visibility | Existing | Required | Required | Host theme and bounds |
| Pointer, row hover, scrollbars, drag/click geometry | Existing | Required | Required | Required on interactive regions |
| Rounded/rigid corners, resize without clipping or layout jumps | Cell fallback | Required | Required | Host and AMP agree |
| Local/SSH playback and negotiated frontend audio relay | Existing paths | Required | Required | Required |

## Verification gates

1. Capture live ASCII baselines with isolated config and a reproducible library;
   include small/large windows, all overlays, and local/remote playback. Source
   inspection and the repository screenshot are the current baseline evidence;
   live performance measurements have **not** yet been collected for this branch.
2. Exercise each matrix row in both native layouts, through keyboard and mouse
   where supported. Check mode changes preserve position, queue, focus, tags,
   settings, and service ownership. Modal clicks must never reach underlying rows.
3. Verify every visualizer with identical analysis inputs, seeking while playing,
   art changes, resizing, long Unicode titles, empty/error/loading states, light
   and dark themes, and rounded/rigid corners.
4. Cache static artwork/geometry. Bound animated updates and queued transport
   frames; compare scroll/input latency, frame cadence, bandwidth, CPU, and audio
   underruns with the recorded ASCII and existing FOLD baselines.
5. Check Linux and macOS locally and through SSH. Actual audible playback and
   physical interactions are separate gates from compilation/protocol tests.
6. Run the repositories' Nix format, clippy, and test gates; rebuild the matching
   AMP helper and verify FOLD music and video transports together.

Current delivery: source audit, parity matrix, six reproducible visual studies.
Implementation follows visual selection; no runtime feature parity is claimed.

## Running the native rack

```sh
staramp graphical
staramp graphical /path/to/playlist.m3u
staramp graphical --ssh user@host
```

Use Kitty for the native graphical frontend. F9 switches between native and
cell rendering within the same player session. Plain `staramp` retains the
ordinary terminal entry point. Existing shortcuts and panel settings remain
available, including Alt+T for themes. SSH renders locally but standalone AMP
audio plays on the host; this command does not enable FOLD's embedded PCM relay.

```toml
[ui]
layout = "classic-rack"
corners = "rounded" # or "rigid"; transport buttons remain square
graphical_palette = "classic" # selected design colors; "theme" uses ui.theme
```

**Winamp Silver** (`theme = "winamp-silver"` at the configuration root,
`ui.graphical_palette = "theme"`) adds a dark–light–dark blue-gray chassis,
original silver gradient transport plates, and muted sage green text, clock and
visualizer accents. Alt+T cycles to it immediately after Winamp Classic.
The Minimal action buttons and Compact Ledger geometry remain in place.
[Native production captures](treatments/winamp-silver.html) cover full and narrow
racks plus 2x density. Winamp Classic remains a separate choice.

The player stays fixed while the lower modules scroll. EQ handles drag frequency
and gain continuously from their pickup position, with no grid rounding or
jump to the combined response under the pointer. Kitty sessions with measured
geometry now use SGR pixel mouse reporting, avoiding character-cell steps;
other terminals retain cell input. The wheel changes Q. Existing
exact-value dialogs remain available.
APO/AutoEQ text and EasyEffects JSON imports share validation and managed-profile
saving. EasyEffects imports support IIR APO (DR), x1 filters only; unsupported
algorithms, global processing, or decramping are rejected. Other active processors
require explicit confirmation that they are excluded from the imported EQ.

Production compositor references are generated with an isolated fixture:

```sh
STARAMP_DIR=/tmp/amp-reference/data STARAMP_CONFIG_DIR=/tmp/amp-reference/config \
  staramp render-rack docs/graphical/rendered
```

Automated coverage includes all sixteen built-in palettes, full/compact layouts,
fixed player geometry while scrolling, surface bounds, existing embed controls,
and import/DSP response regressions. This is not a claim that every acceptance
checklist item has been exercised manually. macOS, audible SSH playback, and
physical mouse/terminal behavior still need live validation.

## Recovered skin implementation plan

Recovered verbatim on 2026-10-09 from the earlier conversation's seven-step
plan, following the request to create a proper plan and then start it.
This is the skin implementation plan; the earlier generic native rack above
is a prototype and does not establish visual acceptance.

The selected full-rack design is [Classic stacked rack](treatments/approved.html),
with its editable master in `treatments/winamp-01.svg`. The current proof is
separate from that reference. The later flat FOLD-style proof is an experiment;
it does not establish a replacement approved target.

### Bitmap precision and fluid sizing

The user's October 9 selection is explicitly **01 — Classic stacked rack ·
parametric EQ**. `treatments/selected-design.json` freezes the source assets and
their hashes. Design selection does not mean the implementation has passed
visual review.

Use original bitmap artwork at 1x and 2x for frame corners, bevels, control
states, transport symbols and clock digits. Preserve corner dimensions; tile
edges and expand the interiors. Window resizing changes layout constraints,
never the scale of a completed screenshot. Keep drawing and interaction bounds
in the same component. Theme masks retain existing color themes, and rounded
and rigid frames remain supported.

Clock atlases preserve the reference's fractional digit advances. Fixed-label
lettering must be measured against the source; track titles and other variable
Unicode content remain shaped text. All AMP artwork belongs to AMP. KIT owns
the generic compositor, density handling, caches and transport.

`render-rack` captures the real shared player at 900 and 1352 logical pixels,
at both densities, under `selected-player-*-*x.png`, plus the complete production
rack under `selected-rack*.png`. Compare these separately labeled captures
against the preserved reference. They are diagnostic fixture renders, not
evidence of macOS or SSH acceptance.

Implementation status: the complete production rack now uses the original-density
skin, including EQ, Album, Activity and Playlist. Bitmap labels, clock and transport
symbols use the study's artwork and source palette. First native launch shows all
modules with the reference LED analyzer; subsequent launches preserve panel visibility. Headers fold sections,
secondary modules stack at narrow sizes, and the player stays fixed while scrolling.
Alt+T selects an existing AMP theme and persists that palette choice. Isolated Linux
Kitty startup, keyboard/theme changes, F9 switching, clean quit and configuration
persistence passed. Visual acceptance, the full manual parity checklist, physical
SSH measurements and macOS validation remain open. See `skin-progress.md`.

The original plan follows without scope reduction:

## Goal

Reproduce the approved Classic stacked rack faithfully in native Rust inside Kitty, locally and over SSH.

**Build the rendering foundation in STAR/KIT so STAR/FOLD and other tools can adopt it later.** AMP is the first application; this phase won’t redesign the other apps.

## 1. Establish a measurable design specification

Turn the approved design into an executable specification:

- Panel dimensions, margins, padding, and alignment.
- Title strips, bevel thickness, corner radii, and recessed displays.
- Typography roles, sizes, weights, baselines, and spacing.
- Button dimensions and interaction states.
- Rules for resizing, scrolling, and module visibility.

Use matching content and colors for visual comparisons. Capture populated, empty, loading, and error states separately.

**Deliverable:** a reference sheet with annotated measurements and reference images.

## 2. Build a reusable skin system in KIT

Introduce reusable components rather than more application-specific rectangle drawing.

| KIT capability | Purpose |
|---|---|
| Nine-slice images | Preserve corners while panels resize |
| Tiled strips | Repeat borders, grooves, and LED patterns cleanly |
| Sprite atlases | Store icons, digits, and control states |
| Theme masks | Recolor assets through existing theme roles |
| Layout constraints | Define fixed, flexible, and minimum dimensions |
| Shared interaction geometry | Keep drawing, hover, clicking, and dragging aligned |
| Asset cache | Reuse decoded images and rendered components |

Assets need explicit dimensions, density, stretch regions, tint roles, and content insets. Rounded and rigid corners become skin variants.

Keep these APIs independent of music playback so FOLD can later use them for tabs, panes, menus, and preview controls.

**Deliverable:** a KIT component showcase rendered through the real terminal frontend.

## 3. Prototype typography before committing to custom fonts

Compare three treatments at actual display sizes:

1. A bundled outline font with carefully controlled metrics.
2. Custom bitmap lettering for fixed labels and branding.
3. A hybrid with bitmap controls and outline text for filenames, metadata, and Unicode.

Create custom clock digits and transport artwork immediately. Select the text approach through actual Linux and Mac captures.

Check:

- Small-size readability and consistent baselines.
- Retina and ordinary display densities.
- Accented characters, CJK, symbols, and fallback.
- Truncation and long labels.
- Font licensing and redistribution.

**Deliverable:** a typography comparison inside Kitty. A complete custom font family proceeds only if this demonstrates a benefit.

## 4. Prove the player module

Implement **only the player** using the new foundation:

- Skinned frame and title strip.
- Recessed display.
- Clock, analyzer, title, and technical metadata.
- Seek bar.
- Square transport controls.
- Shuffle, repeat, and volume.
- Hover, pressed, disabled, and keyboard focus states.

Use the same AMP-owned component for standalone and embedded playback.

### Visual acceptance gate

Compare the actual application against the approved reference using:

- Identical viewport, palette, and fixture data.
- Side-by-side images.
- Transparent overlays and difference images.
- Linux and Mac captures.
- Several window sizes and display densities.

Geometry, baseline, and spacing discrepancies must be resolved. Expected rasterization differences should be identified explicitly.

**No expansion to the remaining panels until this component passes your visual review.**

## 5. Make resizing and SSH part of the foundation

Separate **UI scale** from **window dimensions**:

- Wider windows expand content areas.
- Controls, lettering, corners, and bevels retain their intended proportions.
- Smaller windows use defined layouts and scrolling.
- Text is never silently shrunk to make a layout fit.

For SSH:

- Negotiate skin and asset capabilities.
- Cache assets on the local frontend.
- Send asset identifiers and changing state after initialization.
- Bound frame queues and prioritize interaction.
- Measure bandwidth, input latency, frame cadence, and CPU.

The current input path reports terminal-cell precision. Small controls therefore need usable hit areas, and any finer pointer support must be explicitly negotiated.

**Deliverable:** a responsive player with measured local and SSH behavior.

## 6. Complete AMP module by module

Apply the accepted foundation in this order:

1. **Parametric EQ:** response curve, handles, exact-value fields, profile controls, import/export.
2. **Album:** artwork, metadata hierarchy, provenance, navigation, catalog actions.
3. **Activity:** provider cards, recent listens, delivery states, retry controls.
4. **Playlist:** numbered rows, durations, grouping, selection, tagging, toolbar, scrolling.
5. **Menus and dialogs:** consistent skin, typography, keyboard and mouse behavior.

Each module gets its own visual comparison and functional checks before moving on.

Existing playback, history, library, and EQ logic remain the authoritative state shared with ASCII mode.

## 7. Validate parity and deliver

Complete the action-by-action parity checklist across:

- Keyboard and mouse.
- ASCII/native switching.
- Standalone and FOLD embedding.
- All existing themes.
- Rounded and rigid variants.
- Local and SSH operation.
- Linux and macOS.

Build and install the matching binaries, document the KIT interfaces, and retain reproducible reference captures.

## Completion standard

“Done” requires both:

- **Visual acceptance:** the actual application reproduces the approved treatment.
- **Functional acceptance:** existing capabilities remain accessible and behave correctly.

The first implementation milestone is the **KIT showcase plus one faithful player module**. That gives us concrete evidence that the approach works before we invest in the entire application again.

### Native player button focus

With the Player module focused, **Alt+Left/Right** cycles the visible transport,
shuffle, and repeat buttons; **Enter** activates the focused button. **Escape**
clears button focus. **Tab/Shift+Tab** retain module navigation and clear the
button focus when leaving Player. Ordinary Left/Right seeking and existing
transport shortcuts are unchanged. Dialogs keep their own keyboard handling.
This currently applies to the standalone native rack; embedded hosts continue
to own their keyboard routing.
