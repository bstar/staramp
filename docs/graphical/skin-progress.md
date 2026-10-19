# Classic skin implementation status

## Selected design

The user selected **01 — Classic stacked rack · parametric EQ** on October 9,
2026, requiring bitmap precision and fluid layout. The original SVG and PNG in
`treatments/winamp-01.*` are unchanged; `selected-design.json` records their
hashes. Design selection is separate from acceptance of the application.

## Production implementation

### Framework display recovery · October 15

KIT now retains each rendered frame's matching layout, restores the framebuffer
on focus return, and refreshes quiet image regions from cached encoded tiles.
Ctrl+L restores every graphics layer without recreating the rasterizer. This
prevents mixed frame/layout composition and adds bounded recovery from terminal
placement loss without changing playback, panel or queue state.

The old local AMP executable stayed blank after the private Kitty display was
erased. The updated release restored idle artwork automatically in under three seconds;
focus and Ctrl+L restoration completed in under a second. Help modal restoration
and cleanup passed. AMP's 906 unit tests, embedding, first-run, native rack
rendering and strict Clippy passed. The shared live-canvas proof also restored
quiet controls while two producers kept animating, then passed refresh, resize
and reattachment. This remains an untagged local KIT candidate.

### Unified Play/Pause · October 14

The graphical rack, terminal player and embedded player now have four transport
buttons: Previous, Play/Pause, Stop and Next. The middle button shows Pause while
playing and Play when paused or stopped; its hit target and keyboard focus stay
stable. Loading disables the toggle. Explicit playback protocol commands retain
their separate Play and Pause meanings. KIT's compact audio controls also use a
single state-dependent button.

Linux verification passed: 906 unit tests, 12 embedding tests, first-run and
native rack rendering, including pointer pause/resume and repeated keyboard
activation through both states. Formatting and strict Clippy passed for AMP
and KIT; KIT's all/minimal feature suites and the other three consumer candidate
checks passed. Production captures were visually reviewed; the Nix release
build and installed command verification passed. This remains a local untagged
KIT candidate.

### Collapsed module references · October 14

Collapsed rack modules now use KIT's node filtering to remap text decorations,
centered labels and hover references. Replacing the expanded frame also adjusts
the retained header's references. Composing modules preserves centered labels
and checks decoration indices before looking them up. Grouped playlist body
decorations are covered by a regression at both native densities.

Linux verification passed: 906 unit tests, embedding/first-run/native rendering
checks, strict Clippy and the Nix release build. A real frontend completed 12
playlist collapse/expand actions at 1x/2x, resizing and fallback round trips,
local hover, a 1500-report motion burst and orderly terminal restoration. The
installed command points at the tested local release; KIT remains an untagged
candidate and the portable release dependency lock is unchanged.

### Local seek-bar background · October 12

Local graphical playback now uses a denser checker across the seek-bar
background, beneath the solid played portion. The skin tiles original 1x/2x
assets with alternating logical pixels; compact and legacy surfaces use a
bounded checker. SSH audio keeps Fine dots over its buffered range. Presentation
route state selects the texture for both playback owners and SSH mirror windows.

Formatting, strict Clippy, 911 unit tests, 12 embedding tests, first-run and the
graphical integration passed. Local production captures were reviewed at both
densities; the existing SSH dot-grid and cache-coverage pixel tests still pass.
The Nix release build passed and the installed command was verified.

### Modal focus · October 12

Classic modals now receive the existing theme-colored focus outline immediately
when opened. The border follows the dialog bounds at 1x and 2x density, including
native settings/filter/resume/EQ dialogs and the cell-rendered fallback. Nested
EQ confirmations take focus from their parent dialog. Background panels lose
their outline while a modal is open, and regain it when the modal closes without
changing the remembered panel focus.

Formatting, strict Clippy, 901 unit tests, 12 embedding tests, first-run and the
native rack integration passed. Production filter and EQ profile captures were
reviewed, including the filter at both densities. The Nix release build passed;
the host player was restarted with a recovery snapshot and its executable inode
verified against the new build. Terminal overlays already use the shared focused
modal frame.

### SSH audio · October 12

Terminal and Classic now share standalone SSH audio with a single playback
owner. The source-preserving PCM route retains 32-bit integers and float64,
uses float64 DSP and requires explicit compatibility conversion when the local
device cannot preserve the source. Disconnect pauses; seeking and reattaching
replace queued audio generations. The output footer reports the local format.

Linux checks passed for generated PCM over real loopback SSH, float64 volume,
credits, pause/seek, disconnect/reattach, owner mirroring, device failure,
legacy frontend handling and final drain. A real `--ssh` cell frontend played
through an isolated CPAL/ALSA null device and advanced the remote clock with no
server sound device. The graphical renderer and real frontend mouse/resize
checks also passed. Physical listening and macOS audio hardware remain
unverified. See `../ssh-audio.md` for the published KIT v0.4.0 dependency and use.

### Local queue hover · October 11

Queue hover now uses negotiated native surface hover fills, painted by KIT's
frontend directly from physical pointer coordinates. Movement within the queue
no longer requires controller input, state polling, scene rebuilding or a new
scene transfer. Authoritative scene updates reapply the current local hover;
selection remains controller-owned. Selected rows keep their selection color.
Hover fills follow clipping/density transforms and disappear for folded panes
and modals. Older clients retain the controller-rendered hover path.

AMP pins KIT revision `a811b7c`. KIT's 493 full-feature unit tests, four media
integration tests, 305 minimal-feature unit tests and doc tests pass, as does
strict Clippy with the existing dead-code allowance. AMP's 893 unit tests
(5 ignored), 12 embedding tests and native rack integration pass. Native checks
cover adjacent pixel rows at both densities, unchanged base/selection, modal
suppression, bounded hover references and the legacy rendering path. A real
frontend PTY measured a queue hover repaint at 0.020 seconds while proving no
controller input was sent and selection did not change. Strict Clippy passes.
The Nix release build passes. Its combined PTY check covers seven resize
transitions, local hover repaint at 0.010 seconds without controller input,
1,500 motion reports and clean exit. The desktop frontend, relay and playback
owner use the rebuilt executable; the 29,615-track queue, stopped playback state,
view, theme and EQ/profile files were verified preserved.

### Queue pointer response · October 11

KIT now forwards the latest position in each queued hover burst instead of
replaying every intermediate mouse position. The bounded drain yields to
frame presentation and preserves every click, drag, wheel, key and resize
event in order. A 1,500-report PTY burst reproduced input queue overflow in
the previous frontend. KIT revision `ff8c17e` introduced the correction.

Clicking a queue row now focuses the pane and selects the row on the same
first left press. The next press can complete the normal double-click action;
selection does not wait for another press after switching from a different
pane. Slider and EQ graph focus behavior stays unchanged.

KIT's full/minimal tests and strict Clippy pass. AMP's 893 unit tests (5
ignored), 12 embedding tests and native rack integration pass, with first-click
selection painted on the next frame at both densities without starting playback.
The real frontend PTY passes seven resize transitions followed by 1,500 hover
reports and a close click in 0.274 seconds. The sender uses small paced writes
and concurrent output draining to avoid saturating the PTY itself; the previous
release fails this same paced check. Strict Clippy passes.
The Nix release build and the same resize/burst PTY check against that binary
pass. The desktop frontend, relay and playback owner use the rebuilt executable;
the 29,615-track queue order, stopped playback state, view, theme and EQ/profile
files were verified preserved.

### Terminal resize repaint · October 11

KIT's presentation caches now include the viewport generation. A rapid
shrink/grow sequence can finish at the original dimensions after the terminal
has clipped or moved its image placements; matching pixels and dimensions are
insufficient to reuse those placements. Every resize now replaces all image
regions, retires the old IDs and invalidates the cell diff buffer. KIT revision
`76ed6aa` introduced this correction.

The rack clips scrolling modules above the footer, retains only the visible
portion of bitmap corners in physical source pixels at 1x/2x, and cancels
pointer captures tied to the previous geometry. This prevents footer overlap
and failures when a pane is reduced to a fragment shorter than its corners.

The regression first failed against the previous image cache. KIT's full and
minimal feature tests and strict Clippy pass. AMP's 893 unit tests (5 ignored),
12 embedding tests and native integration pass, including 110 render cases
through partial pane corners at both densities. An isolated real frontend PTY
verifies complete image replacement after unchanged-size, odd-pixel, growing,
shrinking, density and native/cell-layout fallback transitions, plus clean exit
and pixel mouse restoration. Strict Clippy, the Nix release build and the same
PTY check against the release executable pass. The desktop frontend, relay and
playback owner now use that executable; queue order, paused track/position,
shared view, theme and EQ/profile files were verified unchanged. Physical
macOS/SSH resize behavior remains unverified.

### Clickable footer theme · October 11

Clicking the footer's theme label cycles to the next available theme, wrapping
at the end, through the existing theme action. Native clicks activate on the
first press regardless of pane focus and leave reference palette mode so the
selected theme appears and persists normally. The cell footer has the same
click action.

Formatting, strict Clippy, the native rack check at both densities and the Nix
release build passed. An isolated real frontend PTY confirmed a footer click
changes the theme and saves it on clean exit. The desktop uses the new release;
its detached older playback owner was replaced, with queue order, paused track
and position, shared view, theme and EQ files preserved.

### Contextual EQ details · October 11

The native EQ no longer has a bottom strip of every band's parameters,
previous/next links or drag instructions. Clicking a curve point selects the
band and shows its editable details beside the graph. The panel is 36 logical
pixels shorter; adding filters remains available through EQ Settings and the
existing keyboard action.

Formatting, strict Clippy, the native rack integration check and the Nix release
build passed. The 1x production EQ capture was visually inspected. The desktop
frontend, relay and playback owner were refreshed on the new executable with
the queue, paused track/position, shared view, theme and EQ profile files retained.

### Track loading feedback · October 11

Follow-up: the first implementation missed paused-track resumes and storage
reads that blocked an already-running decoder. Playback requests now publish
pending work before reaching the worker; the independent audio callback also
reports starvation and recovery. A failed command send rolls back only its own
pending count. The loading display retains the current position during resume
and buffering, and resets stale timing when opening a different track.

Native transport buttons now activate on the first click from another pane.
Play and Pause send explicit commands; repeated Play presses during a wait
cannot queue opposing toggles. Slider/graph clicks retain their existing
focus-before-drag behavior.

Regression checks exercise Resume queued behind a real blocked CUE read, a
full command channel, empty/refilled/finished output, the actual bounded PCM
relay starving without decoder progress, and first-click transport from other
panes with repeated Play clicks through the production compositor. The full
suite passes: 893 unit tests (5 ignored), 12 embed tests and one graphical
integration test. Physical USB wake and macOS remain unverified.

The follow-up Nix release, formatting and strict Clippy passed. An isolated
real-frontend PTY with a silent WAV and the local audio service confirmed
first-click Play/Pause/Stop after switching pane focus, five rapid Play presses
remaining Playing, zero-coordinate input and main-X clean exit. The live Kitty
frontend, relay and playback owner were restarted on the new release and their
executable inodes verified. The paused position, exact 29,615-track queue order,
shared view, theme, EQ settings and saved/draft profile hashes were preserved.

Foreground track opens now publish a loading flag before any filesystem access,
including the stat/open that can wait for a sleeping USB drive. It stays set
through initial audio buffering, then clears when the output is primed; failed
opens, decoder errors, Stop and end-of-queue clear it as well. Background album
preloads and crossfades do not replace ongoing playback with a loading message.

The native player replaces its analyzer with **LOADING TRACK…** and an animated
indeterminate bar, retains the selected track below it, and explains the wait
on the technical line. Clock/format/meter values from the preceding track are
hidden, and seeking stays unavailable until the track is ready. Compact and
cell presentations also name the wait. The owner/mirror protocol and embedded
status carry a backward-compatible `loading` boolean; older owners omit it.
Loading animation remains visible when the music visualizer/effects are off.

A delayed real CUE read verifies feedback while the playback worker blocks and
cleanup on failure, without changing disk power settings. A real embedded
silent-WAV process verifies ready/stop transitions and PCM delivery without a
sound device. Formatting, strict Clippy, 890 unit tests (5 ignored), 12 embed
tests and the graphical integration check pass. `render-rack` produces separate
`loading-track-1x.png` and `loading-track-2x.png` fixtures through the production
compositor. Their loading display was visually inspected. A physical USB wake
cycle and macOS remain unverified.

The Nix release build and installed-command check passed. The release also
passed the isolated real-frontend PTY check (zero mouse coordinates, main X,
pixel-mode restoration). The live Kitty frontend, relay and playback controller
now use the new executable. Detached old controllers were replaced so the
playback owner can publish loading state. Its 29,615-track queue, paused track
and position, theme, saved/draft EQ files and shared view were retained. The
existing takeover path reset the IPC view to defaults; the saved view was
republished and verified afterward. This is runtime migration evidence, not a
claim that a physical USB sleep/wake cycle was exercised.

### Main module close control · October 11

The STAR/AMP header now has the same close X as the secondary modules, with
settings moved left and a separate hit target. Clicking it uses the normal Quit
action and closes the application on the first click even when another pane has
focus. The native rack integration check covers the close action and adjacent
settings target at both bitmap densities and with artwork disabled. Formatting,
the integration check and strict Clippy passed.

A reported startup panic at Crossterm's SGR mouse parser was traced to its
unchecked subtraction from zero coordinates. The root registry patch now pins
upstream fix `d4e9929`, keeping KIT and Ratatui on one Crossterm instance. A real
frontend PTY check sends zero coordinates during startup and at both edges,
then clicks the main X and verifies clean frontend exit and pixel mouse mode
restoration. The full unit/embed/native checks, Clippy and dependency source,
license and duplicate checks passed with the pinned fix.
The previous release reproduced the reported panic at `parse.rs:737`; the new
Nix release passed the same PTY packets and close-X check. The installed wrapper
still targets this checkout. The user's Kitty window was reopened on the new
executable and survived the zero-coordinate reports, with its paused position,
29,615-track queue, shared view, theme and saved/draft EQ files preserved. Native
Silver gallery captures now include the main header X.


### Reference EQ and profile management · October 10

Player readouts now use original 11px/13px bitmap lettering with cached ink bounds
instead of estimated terminal-font advances. Progress has 24px clear space at
both text ends, volume has 18px, and timestamps sit within 16px outer insets. The
elapsed field has fixed digit advances, keeping the bar stationary as the clock
ticks; volume stays fixed across one-, two- and three-digit values. A raster
regression measures actual alpha coverage at 1x/2x, checks equal gaps and original
one-to-one source pixels, and verifies independence from terminal font choice.
Silver transport buttons have square one-pixel bevels and a narrower metallic
gradient. Playlist number, album-index and duration columns use muted blue, warm
stone and blue-gray ink; titles retain the primary ink. Album track-count fields
have additional room for the presenting terminal's glyph advances.
Formatting, strict Clippy, 887 unit tests (5 ignored), 11 embed tests and the
native rack integration check passed with these changes.
The Nix graphical release was rebuilt and the desktop frontend, relay and
controller verified against its executable inode. A stale reconnecting frontend
needed explicit detach before reopening. Live Kitty inspection confirmed balanced
visible progress/volume gaps, the sharper silver faces, column colors and full
12 tracks / 14 hidden / 11 tracks labels. The 29,615-track queue, paused position,
EQ settings and all saved profile/draft hashes were preserved. Native treatment
captures were refreshed; the frozen reference SVG/PNG hashes remain unchanged.

The initial balanced-track spacing used a nominal 0.6em label width, which was
too narrow after KIT applied the presenting terminal's font scale: VOL, 100 and
the elapsed/total readouts became ellipses. Short readout boxes temporarily used a
0.75em advance plus two pixels of raster slack, retaining equal 14px track gaps.
A production-surface regression compares bounded labels with unrestricted
shaping in Liberation Mono and JetBrains Mono Nerd Font at the desktop's 12/11
scale, at both densities, including a duration over 100 minutes.
Playlist selection now blends its theme color into the actual playlist well at
50% opacity, softening the row fill while preserving text and active-track ink.
The decorative dash before each album's expand/collapse control is removed.
Winamp Silver uses its brighter silver ink for focused-pane outlines and the
playlist scrollbar thumb instead of green.
Formatting, strict Clippy, 887 unit tests (5 ignored), 11 embed tests and the
native rack integration check passed for these corrections.
The graphical release was rebuilt and the desktop frontend/relay/controller
reloaded from that executable. A live Kitty capture verified complete elapsed,
total, VOL and 100 readouts, the 50% row fill, silver outline/scrollbar and no
decorative heading dash. The existing playback owner retained its 29,615-track
queue and paused position; EQ settings and every saved profile/draft hash matched.
Native treatment captures were refreshed.

EQ action labels now centre their actual bitmap ink within the Minimal hover
face, rather than centring an atlas rectangle with extra transparent padding.
Save and the other profile actions use the same 11px bitmap lettering as Save As,
Import and Export. Bounds are cached from each density's alpha coverage; source
and destination pixels remain one-to-one. A regression checks horizontal and
vertical ink centring for all EQ/profile actions at 1x/2x. Formatting, strict
Clippy, 886 unit tests (5 ignored), 11 embed tests and the native integration check
passed. The graphical release was rebuilt and the running desktop restarted on
that executable, preserving playback, all 29,615 queued tracks, view/folds, EQ
settings and saved profiles/drafts. Native EQ preview captures were refreshed.

The initial relative-drag change did not remove character-cell stepping in the
frontend: KIT was synthesizing `Input::Pointer.pixel` from cell centres. KIT
revision f569f00 enables SGR 1016 on known Kitty terminals with measured geometry,
passes exact mouse pixels through the relay, and computes logical coordinates
separately. The optional `pixel_mouse` capability leaves legacy reports readable;
other terminals/multiplexers keep their cell path. Normal/panic restoration turns
1016 off. A real-frontend PTY harness delivered four distinct one-pixel drag events
inside the same logical cell and checked mode restoration. KIT strict Clippy and
its all-feature/no-default-feature suites passed (490/305 unit tests).

The native player's timestamp widths now follow their text, with a shared 14px
gap between each end label and the progress bar. Volume uses the same gap before
and after its track, with a reserved three-digit value field to keep the bar
stationary while volume changes. Playback button artwork is unchanged.
AMP formatting, strict Clippy, 885 unit tests (5 ignored), 11 embed tests and the
native integration check passed. Updated 1x/2x gallery captures were inspected.
The Nix release was reopened and the desktop frontend reported `pixel_mouse:
true` with measured Kitty geometry. Saved profiles, recovery drafts, EQ settings,
29,615-track queue/order, paused position and shared view/folds were preserved.

EQ dragging now anchors the band's original frequency/gain and pointer position.
It applies continuous logarithmic frequency and dB deltas instead of setting the
band's gain to the total response under the pointer. Preamp, neighboring filters
and off-center pickup no longer cause an initial jump. Parameters retain full
precision; returning to the pickup restores them exactly. Release, pointer
cancellation and presentation changes end capture. The native workflow fixture
checks stationary pickup, one-pixel motion, dragging beyond the handle, repainting
during capture, return to origin and release/cancel at 1x and 2x. Formatting,
strict Clippy, 885 unit tests (5 ignored), 11 embed tests and the graphical
integration check passed. The Nix release was reopened on the desktop; EQ
settings, saved profiles and recovery drafts, queue order, paused position and
shared view/folds were preserved.


A restored profile's graph previously retained coefficients compiled at the
startup fallback of 8 kHz after the audio worker rebuilt for playback. This
particularly distorted high-frequency bands until manually loading again.
Compiled EQ settings now retain their actual device rate, and the display
refreshes when that rate changes. Startup, edits and IPC loads use the device
rate, including resampled output, and retain it while stopped. The profile field
uses a separate raster dropdown chevron rather than appending a letter to its
name. The bit-perfect indicator, selected profile name and band-field values
use smaller 10px text; field labels retain their existing size. A Bob-like
regression compares low/mid/high responses to audio settings
at 44.1/48/96 kHz, checks that reapplying cannot change the restored graph, and
verifies the saved file is unchanged. The IPC test covers source/output rate
mismatch as well. Formatting, strict Clippy, 885 unit tests (5 ignored),
11 embed tests and the graphical integration check passed after these changes.
The Nix release was reopened on the desktop with Bob enabled, all saved profile
bytes unchanged, and playback/position plus the 29,615-track queue and shared
view/folds preserved. Native gallery captures include the smaller text.


The EQ now follows the selected rack's geometry more closely: a broad recessed
#101813 graph, subtle logarithmic grid, dB labels padded inside the dark well, muted green
numbered handles, right-hand
band inspector, matching dark-green value fields, and inline clickable filter
summaries. Preamp is separate from band numbering and remains visible at 900px.
The curve comes from the compiled audio chain. Thin Outline frames, common EQ
headers and Minimal action buttons remain the selected theme treatments.

Profile selection opens a searchable native library with Load, Save, named Save
As, New, Rename, confirmed Delete/Revert, Import and Export. Saved custom profiles
sort before built-ins. Selecting a row does not alter playback until Load.
Built-ins use Save As; naming collisions remain visible in the form. Explicit
Save replaces the former per-edit overwriting of managed profile files. Working
curves are private atomic recovery drafts, retained across profile switches and
restart, with saved baselines kept for Revert and external-change checks.
Save As publishes without replacing existing files. Existing modified files are
protected from Save/Rename/Delete. The lifecycle is shared with cell presentation;
native exact-value/name forms consume navigation keys and block background input.

Formatting, strict Clippy, 885 unit tests (5 ignored), 11 embed tests and the
production graphical integration check passed. The fixture covers named saving,
duplicate files, edits not changing saved bytes, restart draft recovery, switching,
inactive rename, delete/revert cancellation and confirmation, external-change
protection, keyboard input ownership, and native modal hit targets at 1x/2x.
Wide/narrow EQ, library and Save As captures are in `treatments/equalizer.html`.
The LAN preview and all linked assets return HTTP 200 on port 3000.
The optimized release was built through Nix and reopened using the existing
ALSA-aware `~/.local/bin/staramp` launcher. The graphical client, relay and server
all resolve to the new executable. Refresh preserved the 29,615-track queue and
order, paused state and position, shared view/folds, Winamp Silver theme, EQ
configuration and every existing managed profile's bytes. Physical listening
and macOS presentation remain separate checks.


After reviewing the stronger stepped border, the user requested another HTML
review rather than accepting that version. `treatments/borders.html` offers four
outer-frame treatments over unchanged native content: Previous bevel, Fine bevel,
Soft contour and Thin outline. Player and Album/Activity contexts, 1:1 sizing,
3x corner details and the latest production frame can be compared. Chromium
checks passed for context/filter controls, keyboard/hash navigation, responsive
bounds, equal comparison sizing and 24 self-contained SVG/PNG exports. These
remain the original review alternatives. The user selected **D — Thin outline**
after reviewing Fine Bevel with silver outlines. Native Silver frames now use
one muted silver #60657c outline with no bevel strokes; the blue-gray body
gradient reaches the outline. The common EQ header treatment is retained.
Player, EQ, Album, Activity and Playlist share a single header painter and
canonical chrome palette; Album/Activity content colors no longer alter their
header background. Production raster checks compare all five header strips,
including highlight and shadow, across all themes at 1x and Silver at 2x.

Winamp Silver is an additional AMP theme, inspired by the user's original
Winamp reference. It has three-stop blue-gray panel shading, original six-state
silver transport plates exported independently at 1x/2x, and muted #a4d791
display accents. Minimal action buttons and Compact Ledger geometry are kept.
The original Winamp Classic palette and selected reference files are unchanged.
The user preferred the earlier sage green after reviewing the neon variant;
text, clock, visualizer and control indicators now use that muted treatment.
The playlist restores Classic’s #101813 dark well, with Silver chrome retained.
Silver panel frames use the selected Thin Outline. Original rounded/rigid
1x/2x frame exports keep fixed corners; raster checks cover the single outline
on all four sides and uninterrupted body shading immediately inside it.
Artwork padding and transparent pixels follow the panel gradient, including
fractional cell grids. Theme selection uses the existing registry and Alt+T.
Native fixtures cover all 17 themes, the new full rack at 1x/2x, button states,
gradient pixels and artwork outline/background continuity. Formatting, strict
Clippy, 881 unit tests (5 ignored), 11 embed tests and the production graphical
integration test passed. Native captures are in `treatments/winamp-silver.html`.
The optimized Linux build was installed through the existing ALSA-aware local
launcher; client, relay and server were reopened on that executable. Alt+T
activated and persisted Winamp Silver. The 29,615-track queue, current playback
position, playing state, pane visibility and folded records were preserved.

Album/Activity layout studies are available in `treatments/panes.html`: Refined
classic, Compact ledger, Artwork first and Split rail. They keep the palette
and Minimal action buttons, use sample history/provider states, and expose
paired or individual panes at 1:1 size plus 1x/2x PNG/SVG exports. Chromium
checks cover text bounds, artwork navigation, hover/focus, pane filtering,
desktop/mobile layouts and export links. The user selected **B — Compact ledger**
and requested larger art. Album now reserves a 160px slot, with selection arrows
and counter beside it, aligned release/count/duration facts and footer actions.
Artwork retains its fitted aspect ratio and two-pixel outline. Activity shows
real cached local/provider state, six compact listened/track/delivery rows and
pending/error context. Unconfigured providers say Set up rather than Off or
Enabled. Native scrolling accounts for six visible rows; ordinary cell clients
retain their existing geometry. Original playback button artwork is unchanged.
Unit (879), embed (11), production graphical integration and strict Clippy checks
pass. Production captures verify artwork geometry and populated/empty Activity
at 1x/2x, including provider state and scroll clamping.
The optimized release was reopened through the existing local launcher. Client,
relay and server use the new executable; playback resumed with all 29,615 queue
entries in order, the user's shared view/folds and EQ visibility preserved.

Four alternative button studies are available in `treatments/buttons.html`:
Compact Classic, Quiet bevel, Thin outline and Minimal. They compare 22px
controls with the current 29px buttons using the supplied Album reference and
matching Player/Playlist/EQ examples. Desktop/mobile browser checks cover the
gallery layout, artwork-counter demo, hover, keyboard focus and transport-state
demo. On October 10 the user selected **D — Minimal**, then explicitly kept the
previous/play/pause/stop/next controls unchanged. Production action buttons use
transparent idle faces, 22px visual height, 11px lettering from fresh 1x/2x
atlases, subdued hover/pressed framing and accent outlines for focus/active
states. Existing larger pointer targets remain intact. Album, EQ, Activity,
Playlist, Resume and Filter share the treatment; album fold signs lose their
idle box. Playback transport faces, symbols, state colors and geometry retain
their previous assets.
All 879 unit checks, 11 embed checks, graphical integration, formatting and
strict Clippy pass. Production captures cover the rack and dialogs at 1x/2x;
palette tests check action-button readability and original transport assets.
The optimized release was reopened through the existing local launcher with
29,615 queue entries in order, the shared view/folds and paused position
preserved (within 0.06 seconds of the decoder seek boundary).

Pane focus uses the accent border alone. Focused title strips retain their
ordinary theme color, without the previous green accent tint.
Existing 1x/2x focus raster and pointer checks, formatting and strict Clippy
pass. The optimized build was reopened with the paused session and shared
view preserved.

The earlier Playlist divider cleanup flattened its title strip; the later
common-header request restores the EQ title relief. The track-list background begins at the first row, removing the exposed dark strip
that combined with the header shadow into an extra horizontal divider.
Production preview, graphical integration and strict Clippy checks pass. The
release was reopened with paused position, queue order and shared view intact.

Large instrument displays use a recessed glass treatment: a soft theme-derived
vertical gradient, quiet scan lines, a four-pixel top/left inner shadow and a
thin bottom/right reflected edge. The treatment is drawn behind the player's
contents and EQ plot inside the existing bitmap bevel, scales at 1x/2x and
keeps a bounded primitive count for large embedded players.
All 879 unit tests, 11 embed checks and the production graphical integration
check pass with this treatment. Palette checks cover the new recessed tones
and preservation of Kanagawa's blue hue.
The optimized build was reopened in the user's Kitty window; paused position,
all 29,615 queue entries in order, cursor, open panels and album folds were
verified after restoration.

Playlist album expand/collapse controls now draw at 14x14 logical pixels rather
than 18x18, with proportionally smaller signs centred in the heading row. Their
18x18 pointer targets remain intact. Existing 1x/2x production rendering and
album-fold interaction checks pass.
The release build was reopened in the user's Kitty window, preserving all
29,615 queue entries in order, album folds and paused playback. Restoring
the position landed within 0.06 seconds, at the decoder's seek boundary.

Resume now uses a native dialog with wrapped artist/title metadata, queue and
position details, saved age, folded-record context and an unavailable-playlist
message. Resume and Start fresh are explicit pointer and keyboard controls;
long content scrolls without accepting or declining the offer. Both choices
still call the ordinary application's resume actions. Playlist availability is
captured when the session is offered rather than queried during drawing.
Production captures cover the dialog at 1x/2x, including long titles, saved
position, folded records and a missing playlist. Regression checks exercise
keyboard selection, both pointer actions, session restoration and scrolling a
long Unicode title without answering the prompt.
Resume buttons reserve extra text width for the presenting terminal's font
metrics. A raster comparison at a larger glyph size checks both full labels
against unrestricted text at 1x/2x; the generic button's tight text estimate had
ellipsized Start fresh in the user's Kitty window despite passing the default
reference capture.
The optimized Linux build was installed through the existing local launcher and
the running graphical window reopened. Client, relay and server use the new
executable; the pending saved-session file remained byte-for-byte unchanged.

The native Album pane now requests artwork when it opens, including when joining
an already paused playback owner. Previously only a track-revision change or the
text renderer started that lookup, leaving native artwork empty until Retry.
The request still runs through the existing background worker and is deduplicated
per track; a closed pane remains idle.

Loaded album artwork is centered at its own aspect ratio on the pane color,
with a two-logical-pixel border following the image edges. The empty dark well
and NO COVER placeholder are drawn only while there is no image. The transported
raster accounts for terminal-cell rounding and uses one-to-one presentation so
the border stays sharp at fractional cell sizes. Resizing, density and theme
changes invalidate the artwork raster and its transport ID.

Clicking an unfocused pane now only selects it. The consumed press cannot
activate a button, move a playlist selection, open a context menu, or start a
slider/scrollbar drag. Subsequent clicks in the selected pane operate normally.
This applies to both the native rack and text presentation; modal controls keep
their existing routing. Regression checks exercise settings in all five panes
at 1x/2x density, playlist selection/right-clicks, and volume selection/dragging
in the native and text views.

Selecting **Winamp Classic** in the graphical theme picker now uses the selected
stacked-rack artwork palette, just like the initial Classic presentation. The
named theme previously recolored that artwork with the legacy terminal theme's
black panels, neon green displays and blue selection. The graphical theme now
keeps the study's slate beveled frames, recessed green displays, soft green
phosphor, raised buttons and muted green selection. Other named themes retain
their palette treatments. The Classic footer's enabled/disabled indicators also
use its phosphor/muted colors rather than the legacy terminal toggle colors.
Production raster verification compares the named
Winamp Classic rack with Classic reference mode at full and compact widths,
excluding only the footer's differing theme name.

The standalone application now composes the complete rack from original 1x/2x
artwork: Player, parametric EQ, Album, Activity, and Playlist. Bitmap clock
segments, transport symbols and fixed-label lettering come from the selected
study. Nine-slice frames retain their corners while interiors expand. Live
Unicode metadata remains shaped text. The reference palette is the graphical
default; Alt+T switches to existing AMP themes. Rounded/rigid corners remain
configurable. Original artwork and player layout belong to AMP; KIT owns generic
rendering, asset caching, density and terminal transport.

At wide sizes Album and Activity sit side by side; narrower layouts stack them.
Player geometry remains fixed while lower modules scroll. The playlist fills the
remaining height above the status footer, expanding when other modules are hidden
or folded. Playlist rows use smaller 11px text and 22px spacing. Short windows
retain a scrollable minimum playlist height. Headers fold modules,
close controls retain panel visibility, and Tab navigates modules. The first
native launch opens the complete rack with its LED analyzer; later launches preserve visibility.
ASCII presentation uses the existing application model and controls via F9.
The shared player artwork is also available through negotiated `native_skin_v1`
embedding. Small/older hosts retain the compatibility player.

## Verification

October 10 artwork framing fix: production raster checks cover a non-square
indexed cover in Winamp Classic and Kanagawa at 1x/2x, including a 2737×1801
viewport with fractional cells. They verify the pane-colored space above the
image, all four border edges, centering, and fresh transport IDs after resize or
theme changes. Nix formatting, all-target Clippy, 879 unit tests, 11 embed checks
and the graphical integration fixture passed.

October 10 artwork startup fix: the graphical integration fixture reproduced the
missing initial request before the fix. It now checks opening Album without a
playback revision, loading a real indexed cover without Retry, publishing its
image in the native scene, and reopening without repeating the lookup. Nix
formatting, all-target Clippy, 879 unit tests, 11 embed checks and the graphical
integration fixture passed.
The optimized build also joined the user's paused Black Sabbath session and
published its embedded cover as a native image on initial startup without any
Retry input. Both graphical views were reopened through the existing local
launcher; the playback owner's track, pause, position, volume and queue remained
unchanged. Remote-library startup and macOS were not exercised for this fix.

October 10 pane-selection update: Nix formatting, all-target Clippy, 879 unit
tests, 11 embed checks and the graphical integration fixture passed, including
the new selection-only mouse regressions.
The optimized build completed and both running graphical views were reopened;
the playback owner retained its paused track, position and volume.

October 10 Winamp Classic update: Nix formatting and all-target Clippy passed;
879 unit tests, 11 embed checks and the graphical integration fixture passed.
The footer refinement also passed a fresh graphical integration run. The
optimized build was installed through the existing local launcher, and both
graphical views were reopened with Winamp Classic selected. A live Kitty
capture confirmed panel #303340, display #101813, accent #a4d791 and selection
#303f3d. The playback owner retained its paused state, track and 134.55-second
position. This verifies the named graphical theme's treatment on Linux, not
acceptance of every layout detail against the study.

Theme mode uses the selected palette directly, without carrying Classic's
channel offsets into pane backgrounds, controls or active states. Panes match
the theme background, wells sit 6% darker, and raised controls/title strips use
restrained neutral highlights. Kanagawa's graphical panes use its raised
background (#2a2a37) instead of its terminal base (#1f1f28). Its player display,
EQ graph and playlist use a darker #1b1b23 inset for clearer separation from
the frames. Album and Activity keep their existing surfaces. Kanagawa uses
its blue accent; Everforest keeps its own teal accent. The Classic study
retains its original artwork colors.
Album names now use 14px text at every pane width instead of 18–22px. The
release checks passed 879 unit tests, 11 embedding tests and the all-theme
graphical integration fixture; Kanagawa, Everforest and Classic captures were
also inspected for this treatment.

The darker Kanagawa inset update was checked through production `render-rack`
captures: Album and Activity match the previous build pixel for pixel, including
the artwork well and compact Album. The other 30 theme captures and Classic
remain unchanged. Legacy embedding palettes and the optional native inset
override both passed process protocol checks.

Native LED and dot analyzers now draw every segment once on a fixed grid,
choosing lit or unlit color per cell. Lit segments no longer originate at a
moving bar edge. A regression check sweeps silence through full level at 1x/2x
and requires unchanged cell geometry with increasing lit-cell counts.

The native footer uses the same stable status model as ASCII, with Help and the
active theme on the left and separately positioned visualizer, shuffle, repeat and crossfade
indicators on the right. Transient messages occupy the spare middle space and
never replace the theme or those indicators. Classic palette mode identifies
itself as Classic rather than naming the overridden theme. Pixel layout avoids clipping caused by flattening
a wide terminal row into a single native text label; the footer background
remains transparent.

Footer verification checks actual rendered glyph opacity, not just text-node
presence. KIT's glyph compositor now writes alpha for text on transparent
surfaces; previously the RGB existed in reference images but Kitty discarded
the invisible glyphs. Terminal gaps remain transparent.
The KIT renderer's 23 native tests and AMP's all-theme pixel-opacity checks
passed. Both running graphical clients were reloaded with the installed
binary; a live Kitty capture confirmed Help, the resolved theme name and all
right-hand status indicators were visible over the terminal background.

Theme-mode queue interiors use the theme's inset color. Kanagawa shares its
darker display well; other themes retain the 6% darker surface. The queue
background is flat to its edges,
with no recessed dark frame around it. Album fold buttons draw centered plus/minus
geometry rather than font glyphs. Pane title strips and album sections have
subtle theme-derived highlight/shadow edges.

The focused native pane has a one logical pixel bitmap accent outline matching
the pane's rounded or rigid contours, with unchanged layout bounds. Pointer
presses focus the containing pane even in blank space;
keyboard focus uses the same outline. Production raster checks exercise all
five panes at 1x/2x, including paired Album/Activity and their actual pointer
coordinates.

The playlist filter now has a native dialog with a full-width editable field,
live matching, a result count, and Apply/Clear/Cancel controls. Shared KIT text
editing handles pasted text, Unicode, caret navigation and deletion. Apply
commits the draft; Cancel or closing the dialog restores the previous filter.
Filtering preserves the playing track and suppresses underlying playlist clicks.
The ASCII filter uses the same editing and live matching model.

The scrollbar thumb has a 24 logical pixel minimum (48 physical pixels at 2x).
KIT uses the same minimum for painting, grabbing, track clicks and dragging.
Production fixtures cover 30,002 tracks, expanded and folded albums, the full
scroll range and stable release at both densities.

The native rack has no outer container inset. Pane gaps, rounded outer corners
and the status area are transparent, exposing the terminal background; pane
interiors keep their theme colors. KIT preserves straight-alpha antialiased
edges for terminal composition. Close marks are slightly smaller while their
larger hit targets remain unchanged.

Isolated Kitty sessions at 1x and 2x also verified the live filter and mouse
Apply/Clear/Cancel actions, followed by transport and pane-close controls.
A custom Kitty background was sampled between panes to confirm that the gaps
retain the terminal color. Formatting, the 878 unit tests, 11 embedding tests,
the graphical integration fixture and strict Clippy passed before release
installation. The launcher and FOLD helper use matching release binaries.

The graphical playlist now uses the ASCII view's album headings, fold counts,
artist/title fallbacks and disc/track numbering. Album dividers run through the
spare width, dated records follow the selected release-year direction, undated
records follow dated records, and tracks without an album remain last. The
playlist's KIT scrollbar counts rendered headings and folded rows, paints only
an inset content-sized thumb, and shares physical geometry with drag input at
both densities. Wheel scrolling stays in the playlist, and releasing a drag
preserves its viewport even when every album is folded.

Theme mode no longer retains Classic's green well and inactive-LED offsets.
Display fills, bitmap well interiors, clock segments and unlit analyzer LEDs
follow the chosen background hue; the selected Classic palette retains its
original artwork colors. Kanagawa has a raster regression for the actual
composed display interior. Rack headers, native popouts and embedded track
pickers share centered close icons with larger targets, independent of font
bearings; rack and popout close controls highlight on hover.

Production rendering checks the new album sections and scrollbar drags at
1x/2x, including release stability, folded lists, wheel routing and exact
agreement between painted and interactive thumb geometry. These supplement
the existing separate-owner queue and real Kitty pointer checks. Isolated X11
Kitty sessions at 1400×2000 (1x) and 2800×2400 (2x) additionally passed
Settings, sorting, track context removal, transport and close-button clicks;
closing Playlist and EQ persisted their visibility without stopping playback
services. The updated release is installed through the existing local launcher
and as FOLD's AMP helper, with matching executable hashes.


Graphical pointer dispatch now uses KIT's zero-based left/right/middle button
contract, including the cell-based modal and ASCII routes. Specific header
controls take priority over the fold strip; settings overlays share the padded
area used for mouse hit testing. The Player settings link now exposes existing
visualizer, shuffle, repeat, crossfade, EQ, and animation actions instead of an
empty settings list. Debug logging records activated native controls.

Regression checks cover all five Settings links, Sorting and Gravity, modal
selection with padding at 1x/2x, volume, EQ, and pause/resume/stop through the
playback worker. All 878 unit tests, 11 embed checks, the graphical integration
check and strict Clippy pass.

Isolated Linux Kitty sessions at 1400×2000 (1x) and 2800×2400 (2x)
also passed actual X11 mouse clicks for all five Settings links, Sorting,
Gravity, EQ, and Play/Pause/Previous/Next/Stop with a silent WAV fixture.
A Player setting changed by mouse was persisted on clean exit. Fresh isolated
startup intermittently encountered existing activity-database contention;
that separate startup race remains unresolved.

Native Settings and track-action popups now lay out their original rows directly
with STAR/KIT primitives. Labels, values and titles wrap within pixel bounds;
long menus scroll with the keyboard or wheel, and visible rows have matching
native mouse hit regions. The track context menu retains all six ASCII actions,
including destination flows and removal of the exact queue occurrence.

Selecting album order disables overriding track shuffle. Folder/file queues read
local tags without requiring a scan, preferring indexed metadata when their root
matches the configured library. Native album sections have fold controls. The
regression fixture checks interleaved albums, ascending/descending order, folding,
current-track preservation, right-click removal and the last row of a long menu
at both densities. A real tagged-file test covers folder and file launches.

Album and Activity share a balanced row from 1040 logical module pixels and stack
below that width. Compact Album keeps a 128-pixel artwork slot; regular artwork
remains 180 pixels. Activity omits the provider/status card strip, shows five recent
listens, and keeps scrobbling configuration in Settings. Responsive captures cover
both densities, including a 1100-pixel rack with paired modules.

The updated compositor additionally passed actual X11 left/right mouse clicks in
isolated Kitty at 1400×2000 (1x) and 2800×2400 (2x), using tagged silent FLACs.
The running owner's queue confirmed album grouping after clicking Sorting, and
confirmed one exact track was removed through the right-click menu. Settings,
Gravity and transport remained functional; both sessions exited cleanly.

Track actions now open as a compact context menu beside the pointer, with
aligned rows, grouped actions, separators, hover selection, a subtle shadow and
rounded or rigid theme corners. The menu flips inward at the viewport edges;
destination and confirmation flows retain their larger dialogs. Pointer anchoring,
hover without execution, outside dismissal and row actions are covered at 1x/2x.
Actual Kitty mouse clicks at both densities confirmed exact track removal.

Album-order validation now also uses a separate IPC playback owner with a
30,000-track queue. Followers install the received order unchanged, retain the
owner's revision for queue edits, retry failed queue fetches, and refresh the
open Sorting dialog after changes. Direction and manual-arrangement state are
published by new owners. Choosing year order disables shuffle and clears a
manual arrangement; remote requests must be acknowledged before success is
reported. The fixture checks ascending/descending changes, externally changed
order, shuffle, toggling grouping off, and preservation of the current track.
The actual user session was found to follow a September 27 playback owner;
its running queue has album and year metadata. That process was left intact.
An isolated copy of that older executable also passed actual graphical mouse
clicks for year ordering and reversing direction, verified against its queue.
The release build and installed STAR/AMP and FOLD helper binaries match.

The October 9 large-window startup failure was reproduced at 1400×2000:
the rack capped its canvas at 1800 pixels high, causing the host to stretch
fixed bitmap glyphs and reject the frame. The canvas now matches the physical
viewport, including one-pixel gutters at 2x density. KIT accepts surface
coordinates up to its existing terminal viewport budget; decoded asset limits
remain unchanged. Renderer failures also retain their underlying error in the
terminal and file log.

Regression rendering covers tall (1400×2000), odd-sized (2737×1801), and 4K
(3840×2160) viewports. The release additionally passed isolated Linux Kitty
sessions requested at 1400×2000 and 5121×2161, including themes, F9 round trips,
clean quit, and configuration persistence. These checks do not establish macOS
or physical SSH visual parity.

| Area | Evidence | Remaining |
| --- | --- | --- |
| Artwork and colors | Original-density atlas geometry; production raster checks for source palette and active/inactive button contrast | User visual review; variable font baselines across platforms |
| Full rack | Production compositor renders at 1x/2x; all 16 themes; wide/narrow, folding, close/reopen, scroll bounds and fixed transport checks | Physical Mac Retina and SSH comparisons |
| Interactions | Existing AMP actions retained; graphical checks for keyboard focus, Enter activation, overlays and F9; actual Kitty mouse clicks for Settings, sorting and transport at 1x/2x | Complete manual parity checklist and audible playback review |
| Kitty application | Isolated Linux Kitty launch, keyboard/theme changes, F9 round trip, clean quit and config persistence passed | Real user sessions on Linux/macOS and authenticated SSH |
| Startup | Fresh graphical configuration without a library index opens the empty player and Library/Playlist controls | Intermittent activity-database contention; decoder loading/error visual references |
| Shared embed | Existing child-process control checks and shared player state/surface tests pass | Audible FOLD embedding and physical macOS/SSH review |
| Build gates | Rust formatting, strict Clippy, 879 unit tests (5 ignored), 11 embed tests, 1 graphical integration test passed | Cross-platform CI and live platform validation |

The gallery's production captures are generated by `staramp render-rack` using
isolated fixture data. They are not the independent `skin-proof` example.
They do not establish pixel-perfect acceptance, physical SSH performance, or
completion of every item in the original seven-step plan.

October 12 cache-bar correction: the separate lower cache stripe and separator
made the seek bar appear split. The native player now uses one continuous
8-pixel bar (6 for Thin), with played, cached-ahead, and empty ranges using the
full stroke height. The duration label retains the cached percentage or
“cached” confirmation. Compact ASCII uses a narrower square-edged stroke so
all selectable styles remain distinct without moving their seek hit regions.
Rendered pixel tests at 1x/2x check every interior row for an uninterrupted
fill and confirm a full cache reaches the end. Formatting, strict Clippy,
900 unit tests (5 ignored), and 12 embed tests passed on macOS.

The user selected Fine dots from the October 12 browser comparison. Full and
compact graphical skins now tile a stationary 1-pixel dot on a 3×3 logical-pixel
grid over cached audio, using the comparison's muted 48% contrast. Played audio
remains solid and the cache label is retained. Original 1x/2x texture assets
keep the grid crisp and add one sprite instead of per-dot scene primitives;
frontends without skin negotiation also draw Fine dots through bounded scene primitives.
Pixel tests check grid pitch, density, uninterrupted stroke height, and cache
coverage through the final pixel. Strict Clippy, 900 unit tests (5 ignored),
12 embed tests, first-run checks, and the graphical integration test passed
on macOS.

Album facts now distinguish the original release year from the tagged edition
year. Explicit original-release tags take priority, then an album-folder year
(e.g. `1980 - Blizzard Of Ozz (2007)`). Known remasters with no usable original
year display an unknown date. CUE tracks use their backing audio file; metadata
is read on the album worker and cached without rewriting tags or reindexing.

Regression coverage includes real original-release/edition tags on a WAV file,
folder and box-set date rules, full/compact dotted cache rasterization at 1x/2x,
and actual spectral analysis of playback positions in a fully downloaded song
with a silent outro. Clock tests cover initial buffering, pause, stalls, ordinary
SSH acknowledgement gaps, and stale samples after cache epoch replacement.

Album heading dividers now follow the frontend's shaped glyph edge with a
constant 10px gap, including ellipsized titles and 2x density. The playlist
header adds lowercase jump using the same text style as its other links, with
equal gaps between the action labels. It reveals the
active track, falling back to the selection when no track is active. Reveal
opens its folded album, clears a hiding filter and releases the manual
scrollbar position without changing playback or queue order. Manual scrolling
retains the selection.

Starting a selected track preserves the playlist viewport. Row selection and
double-click playback retain the clicked row and pane focus after manual
scrolling; regression checks exercise the complete pointer path at 1x/2x.

`staramp import --graphical INBOX` now starts an import-aware Classic rack in
its own session. It uses the existing staging index and transactional album
import/reject logic, and refuses to contend with an already running playback
owner. Right-clicking an open or folded album heading opens its album actions.
Production-rack reference checks exercise those menus, artist selection,
duplicate decisions and reject confirmation at 1x/2x. A private real frontend
PTY check covers the public command, separate staging index, unchanged fixture
music, window close and terminal restoration. These checks do not import or
reject the user's real music.


Graphical import review now has visible Import/Reject toolbar actions,
first-press right-click context menus across pane focus, hover selection in
artist/conflict/reject dialogs, and a native artist-directory field with
mouse caret placement and Use/Back controls. The field uses KIT's existing
text editor for Unicode, paste and cursor/delete keys. Staging rescans run on
the album worker; incremental queue edits retain playback, selection, scroll
anchors, tags and folds after completed operations. Classic artwork is a
cached sprite in the rack surface, so opening a modal preserves the same
artwork beneath it. The graphical player and album facts now share the
terminal player's format string, including known kbps for lossy formats.
Production-rack regressions cover these pointer paths at 1x/2x, stable cover
assets through modal open/close, review state after removal, and visible
MP3/AAC/Vorbis/Opus bitrates. All fixtures use private data.
