# Classic skin implementation status

## Selected design

The user selected **01 — Classic stacked rack · parametric EQ** on October 9,
2026, requiring bitmap precision and fluid layout. The original SVG and PNG in
`treatments/winamp-01.*` are unchanged; `selected-design.json` records their
hashes. Design selection is separate from acceptance of the application.

## Production implementation

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

Theme mode uses the selected palette directly, without carrying Classic's
channel offsets into pane backgrounds, controls or active states. Panes match
the theme background, wells sit 6% darker, and raised controls/title strips use
restrained neutral highlights. Kanagawa's graphical panes use its raised
background (#2a2a37) instead of its terminal base (#1f1f28), with the queue
slightly darker (#272734). Kanagawa uses its blue accent; Everforest keeps
its own teal accent. The Classic study retains its original artwork colors.
Album names now use 14px text at every pane width instead of 18–22px. The
release checks passed 879 unit tests, 11 embedding tests and the all-theme
graphical integration fixture; Kanagawa, Everforest and Classic captures were
also inspected for this treatment.

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

Theme-mode queue interiors now sit just 6% darker than the surrounding pane
surface, keeping Kanagawa consistent with Album, Activity and EQ instead of
using the much darker analyzer well. The queue background is flat to its edges,
with no recessed dark frame around it. Album fold buttons draw centered plus/minus
geometry rather than font glyphs. Pane title strips and album sections have
subtle theme-derived highlight/shadow edges.

The focused native pane has a two logical pixel accent outline with unchanged
layout bounds. Pointer presses focus the containing pane even in blank space;
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
| Build gates | Rust formatting, strict Clippy, 878 unit tests (5 ignored), 11 embed tests, 1 graphical integration test passed | Cross-platform CI and live platform validation |

The gallery's production captures are generated by `staramp render-rack` using
isolated fixture data. They are not the independent `skin-proof` example.
They do not establish pixel-perfect acceptance, physical SSH performance, or
completion of every item in the original seven-step plan.
