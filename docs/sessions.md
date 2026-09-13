# Several terminals, one session

Every window is the same window. The first instance to start binds a control
socket and owns the audio device; later ones find it and join, and from there
they behave identically. Press play, pick a track, reorder the albums or fold
a record away in any of them and the rest follow.

```sh
staramp        # first: plays
staramp        # second: the same session, in another terminal
```

## What is shared

| Shared | Not shared |
| --- | --- |
| the queue and its order | the terminal's size |
| the track and position | its theme |
| volume, shuffle, repeat | whether it can draw pixels |
| album order and any arrangement made by hand | how wide a cell is |
| the cursor and the folded records | how far a window of a different height has scrolled |
| which panels are open, as intent | |

Panel visibility is shared as _intent_, so a window too small for the
playlist hides it there without closing it everywhere.

`[session] share = "playback"` narrows the sharing to the music alone, if you
would rather each window kept its own place in the list.

## Who owns playback

One window owns the audio device, records listening history, submits
scrobbles and publishes Discord presence. The others send their transport and
setting changes to it, which is what keeps a listen from being counted twice
and the equalizer from being applied twice.

**If the instance holding the session goes away**, another picks it up and
carries on from where it was, mid-track included. Nothing is reloaded: the
window taking over already had the queue and the view in front of it.

## Opening a playlist while something is playing

STAR/AMP asks what you meant, in the window you just opened, rather than
guessing or expecting you to have known to pass a flag:

```
╔═ ALREADY PLAYING ════════════════════════════════╗
║ 2003 was asked for, and a session is already pla…║
║  join the session, leave it playing              ║
║  load 2003 into the session                      ║
╚════════════════════════ enter change · esc close ╝
```

## Resuming

On the next start, STAR/AMP offers to pick up where you left off: the
playlist, the track and the position it was at. `session.toml` in the
[staramp directory](configuration.md#where-it-keeps-things) is where that is
remembered.
