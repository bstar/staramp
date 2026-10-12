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

The manifest, widget, and application share one release version, checked by
`scripts/check-version.sh`. Basic MPRIS controls still work with an older
installed player; the popup identifies a version mismatch and waits for a
matching version before enabling repeat and queue details.
Queue details are queried with `staramp ctl status` only while the popup is
open. They disappear if the CLI is missing or reports a different track.
Listening details come from `staramp scrobble snapshot`, which reads STAR/AMP's
local history and provider state without exposing credentials. It also runs
only while the popup is open. Last.fm and ListenBrainz must be authenticated
through STAR/AMP to show as connected; the old CLIAMP Last.fm configuration
is separate.
