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
| Shared KIT bitmap skin foundation | Bounded nine-slice, alpha masks, bitmap glyph metrics; tests pass | Shared asset manifest/cache transport and production integration |
| Native typography | Persistent compositor with explicit font family; outline/bitmap comparison | Final font selection, baseline refinement and size review |
| Independent fluid layout and density | 900/1352/1800 captures; independently composed 2x assets/text | Compact layout below 720 logical pixels |
| Kitty interaction | Isolated release runtime: mouse, seek drag, focus, density, resize, quit | Production controls bound to real AMP state/actions |
| Theme support | Existing AMP theme engine retained | Apply themes to skin artwork and verify all themes |
| Rounded/rigid treatment | Separate skin masters; interactive toggle | Wire to production configuration |
| Linux/Mac/SSH | Linux Kitty runtime verified | Mac Retina and authenticated SSH validation |
| Visual approval | Review gallery available on port 4000 | User acceptance of revised treatment |
| Complete standalone and embedded AMP | Existing UI remains installed | Migrate only after player proof approval; preserve all actions |
| EQ/Album/Activity/Playlist/dialogs | Original design and action checklist retained | Implement and verify each treatment against updated target |
| Other STAR apps | Generic APIs reside in KIT | FOLD migration later, as requested |

The plan is not complete. Do not replace its feature-parity checklist with
sample interaction checks, or claim production performance from the skin proof.
