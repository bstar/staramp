# Skin migration progress

## Current direction

2026-10-09 feedback asks for more of FOLD's quieter treatment while retaining
the stacked rack organization. The flat proof is an experiment for review;
the preserved Classic target has not been replaced or accepted anew.
Reduce chrome and analyzer noise. Font-size direction remains to be clarified.
Resolve the proposed adjustments against the preserved reference before claiming fidelity.

## Evidence and remaining work

| Requirement | Current evidence | Remaining |
| --- | --- | --- |
| Shared KIT bitmap skin foundation | Bounded nine-slice, masks, atlas sprites, explicit density and reliable cached asset transport; full KIT tests pass | Physical SSH measurements and long-session cache review |
| Native typography | Persistent compositor with explicit font family; outline/bitmap comparison | Final font selection, baseline refinement and size review |
| Independent fluid layout and density | 900/1352/1800 captures; independently composed 2x assets/text | Compact layout below 720 logical pixels |
| Kitty interaction | Isolated release runtime: mouse, seek drag, focus, density, resize, quit | Real player bindings now exist; verify the production terminal path |
| Theme support | Mask layers + all 16 built-in captures; active-control contrast checks | Production/user-theme integration |
| Rounded/rigid treatment | Separate skin masters; interactive toggle | Wire to production configuration |
| Linux/Mac/SSH | Linux Kitty runtime verified | Mac Retina and authenticated SSH validation |
| Visual approval | Review gallery available on port 4000 | User acceptance of revised treatment |
| Complete standalone and embedded AMP | Shared AMP-owned skinned player uses real state/actions in standalone and negotiated embed | Release/runtime review; remaining modules follow visual approval; preserve all actions |
| EQ/Album/Activity/Playlist/dialogs | Original design and action checklist retained | Implement and verify each treatment against updated target |
| Other STAR apps | Generic APIs reside in KIT | FOLD migration later, as requested |

The plan is not complete. Do not replace its feature-parity checklist with
sample interaction checks, or claim production performance from the skin proof.

### Layout simplification following review

The production graphical rack now starts EQ, Album and Activity folded into
compact clickable headers. Playback and Playlist remain expanded; unfolding a
section reveals its existing controls without changing playback or service state.
Secondary sections stack vertically, and album metadata uses the regular body
size rather than enlarged title typography. This is a usability revision, not
visual acceptance of the skin proof or completion of the remaining parity gates.

### Production player and negotiated transport

`src/embed/skin.rs` bundles the original artwork masks and recolors them with
the current palette. `native::player_surface` supplies the real player state
and actions to both standalone and embedded views. Artwork IDs use PNG content
hashes; KIT's reliable Asset channel transfers each once per connection, while
replaceable scenes contain references and changing primitives. Old frontends
and small hosts retain the ordinary player surface. `native_skin_v1` and
`native_skins` are separately negotiated by AMP, its host, and the frontend.

Unit and child-process checks cover live clock/analyzer state, stable seek and
volume geometry, cached artwork, Unicode, explicit 1x/2x density, theme/corner
variants, and legacy fallback. This does not establish visual acceptance,
physical SSH latency, audible playback or macOS fidelity. The production player
still needs measured comparison with the preserved reference; the remaining
modules have not been migrated to this skin.

### Production keyboard button focus

The standalone native rack now cycles visible player buttons with Alt+Left/Right
and activates them with Enter, using the emitted hit geometry and ordinary AMP
actions. Tab retains module navigation; leaving Player clears button focus.
Dialogs and cell mode keep their existing routing. Narrow compatibility surfaces
receive an accent outline rather than losing the focus indication. Graphical
fixture checks cover activation, seek-shortcut preservation and overlay isolation;
this does not establish embedded-host keyboard parity or physical-terminal testing.
