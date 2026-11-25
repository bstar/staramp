# Omarchy shell widget

STAR/AMP includes an optional Quickshell bar widget for Omarchy. The player
still runs in its own terminal. The widget talks to STAR/AMP over MPRIS and
shows playback, cover art, seek, repeat, shuffle, player volume, queue
position, local recent listens, and Last.fm/ListenBrainz delivery state. Its
bar button and popup use the same Omarchy theme controls as other modules.
When STAR/AMP is not running,
the bar icon opens a short unavailable message.

Install STAR/AMP first and make sure `staramp` is on `PATH`. Then install the
plugin from this same repository:

```sh
omarchy plugin add https://github.com/bstar/staramp.git --enable
```

The plugin manifest is at the repository root because Omarchy requires it
there. Omarchy clones the source repository into its plugin directory; it does
not build or launch STAR/AMP as part of the plugin installation. To update,
update the STAR/AMP application first, then run:

```sh
omarchy plugin update bstar.staramp
```

The manifest, widget, and application are released together under one
version, checked by `scripts/check-version.sh`. They don't have to be
*installed* at the same version, though. The plugin updates by git and the
player through your package manager, so either can arrive first. The widget
decides compatibility from the `protocol` number that `staramp ctl status`
and `staramp scrobble snapshot` report, not from the release version:

| Widget sees | What works |
| --- | --- |
| `staramp` not on `PATH` | The bar icon says STAR/AMP isn't installed, and the popup links to [installing](installing.md) |
| Installed but not running | The popup says STAR/AMP isn't running |
| Same protocol | Everything |
| Older or newer protocol, or a player too old to report one | Basic MPRIS controls. Repeat, queue position and history wait, and the popup says which side to update |

The protocol number goes up only when a field the widget reads changes
meaning or is removed. New fields don't change it.
Queue details are queried with `staramp ctl status` only while the popup is
open. They disappear if the CLI is missing or reports a different track.
Listening details come from `staramp scrobble snapshot`, which reads STAR/AMP's
local history and provider state without exposing credentials. It also runs
only while the popup is open. Last.fm and ListenBrainz must be authenticated
through STAR/AMP to show as connected; the old CLIAMP Last.fm configuration
is separate.
