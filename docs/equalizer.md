# Equalizer and APO profiles

The equalizer is an ordered processing chain, similar to the EQ portion of
EasyEffects. `alt+g` shows the panel, `e` turns the chain on and off, and `[`
and `]` step through presets.

## The chain

Each stage is one of:

- a preamp
- a biquad: peaking, low-pass, high-pass, band-pass, low-shelf, high-shelf,
  notch, or all-pass
- a GraphicEQ curve
- custom IIR coefficients

Every stage can be limited to a channel. Processing stays in `f64` throughout
the chain and converts to `f32` only at the audio-output boundary.
Coefficients are rebuilt for the output device's actual sample rate, and the
panel draws the chain's response curve.

## Editing it

With the panel focused:

| Key | Does |
| --- | --- |
| `up`/`down`, `j`/`k` | select a filter |
| `left` / `right` | gain by 1 dB; `shift` makes it 10 dB |
| `enter` | bypass the selected filter |
| `a` | add a peaking filter |
| `d` | remove the selected filter |

The panel's `settings` menu has the rest:

- exact pasted values for frequency, gain and Q
- filter type and channel selection
- reordering and duplication
- managed-profile creation, rename and delete
- APO import and export

## Equalizer APO profiles

The import browser accepts Equalizer APO `.txt` and `.apo` profiles.

| Supported | Rejected |
| --- | --- |
| `Preamp` | device routing |
| standard `Filter` forms | mixers |
| custom `IIR` | delays |
| `GraphicEQ` | convolution |
| `Channel` | other non-EQ commands |
| recursive `Include` | |

A rejected command is reported with a file and line number instead of being
silently ignored. Imports are transactional, so an invalid file never partly
replaces the live profile. Export writes a portable APO file.

Edited and imported profiles are kept under `equalizers/` in the
[staramp directory](configuration.md#where-it-keeps-things).

## Visibility is not state

> [!NOTE]
> EQ processing is independent of panel visibility. `alt+g` only shows or
> hides the controls; `e`, or `[eq] enabled`, is what enables or bypasses the
> chain. Closing the panel never changes what you hear.

The same holds when several STAR/AMP windows share a session: the window that
owns audio receives and applies EQ changes from follower windows.
