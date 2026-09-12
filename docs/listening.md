# Listening history, scrobbling and Discord

Local history is always on and needs nothing. Last.fm, ListenBrainz and
Discord are separate, optional, and off until you turn each one on.

## Local listening history

Every play and skip is recorded locally. It does not require an account or a
network connection.

| What happened | Recorded as |
| --- | --- |
| the track ended naturally | played |
| half of it, up to four minutes, actually played | played |
| next, previous, stop, or a seek before that point | skipped |
| a crash, decode failure, or quit | interrupted, not skipped |

Paused time does not count.

The history lives in `activity.sqlite`, separate from the rebuildable index,
so a rescan or a replaced remote index cannot erase it. Smart playlists read
it through `playcount`, `skipcount`, `lastplayed` and `never`; see
[Smart playlists](smart-playlists.md).

Only the window that owns playback records a listen. Other windows in the
same session send their transport and setting changes to it, so a listen is
never counted twice.

## Scrobbling

The network providers are independent and off by default.

### Turning a provider on

From the Activity panel:

1. `alt+s` opens the panel.
2. `enter` opens its settings.
3. Choose a provider's authenticate row and paste the credentials it asks
   for. Last.fm asks for its API key and shared secret in turn, then opens
   the browser authorization page. ListenBrainz accepts its single user
   token.

Or from the command line:

```sh
staramp scrobble auth lastfm
staramp scrobble auth listenbrainz
staramp scrobble status
staramp scrobble logout lastfm
```

Last.fm wants an API key and shared secret from your own Last.fm API
application. ListenBrainz wants the user token from your profile.

### What gets submitted

- Eligible listens need real artist and title tags and more than 30 seconds
  of duration. Filenames are never invented as metadata.
- Failed submissions stay queued with bounded exponential backoff, survive
  restarts, and can be retried from the panel.
- Credentials are stored separately in `credentials.toml` with mode `0600` on
  Unix, never in the ordinary config.

> [!NOTE]
> The `[scrobble]` switches keep working when the Activity panel is hidden.
> `alt+s` changes only whether the panel is drawn.

## Discord Rich Presence

Discord presence is native, local, and opt-in: star/amp talks directly to the
running Discord desktop client's IPC socket. It is not a bot, needs no bot
token, and does not join or read any server.

### Setting it up

1. Create an application named `STAR/AMP` in the
   [Discord Developer Portal](https://discord.com/developers/applications).
2. Copy its Application ID.
3. Set:

```toml
[discord]
enabled = true
client_id = "your-numeric-application-id"
# Optional button on the activity:
lastfm_username = "your-lastfm-name"
```

You can turn an already configured presence on and off from the Activity
panel's settings.

### How it behaves

- It keeps running when the Activity panel is hidden.
- It publishes only from the window that owns playback.
- It shows play/pause and elapsed time, and clears the activity on stop or
  exit.
- The Discord desktop client must be running. Discord's browser and mobile
  clients do not expose the local socket.
- Discord's local RPC presents third-party applications as `Playing`, so the
  visible heading is `Playing STAR/AMP`. The Spotify-only `Listening`
  treatment cannot be selected by an ordinary RPC client.

### Album art on the card

Album art is sent as a public HTTPS asset, which is the only image form the
Discord RPC bridge can publish; it cannot read star/amp's local cover files.
star/amp uses a tagged MusicBrainz release ID when one exists, then falls back
to an independent MusicBrainz album lookup.

Discord presence never requires Last.fm configuration or authentication.
