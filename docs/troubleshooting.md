# If something is wrong

The usual suspects, in the order people hit them.

## It says it does not know where the music is

Nothing has been indexed yet, or `library_root` is unset.

```sh
staramp scan /path/to/music
```

Run once; it sets the setting for you.

## A track will not play, or sounds wrong

Two commands split the problem in half:

```sh
staramp probe  <file>              # what star/amp sees in it
staramp decode <file> -o out.wav   # the samples, without the audio device
```

| `decode` is | Playback is | The problem is |
| --- | --- | --- |
| right | wrong | the output |
| wrong | wrong | the decoder. Attach that output to a bug report |

## No sound, but everything looks like it is playing

The audio device is opened at the file's own sample rate for bit-perfect
output. If something else holds the device exclusively, that fails.

```toml
[output]
mode = "fixed"
```

pins one rate instead.

## The transport buttons are text rather than pictures

The terminal has no graphics protocol, or the detection cannot see through
ssh or a multiplexer.

```toml
[ui]
graphics = "kitty"
```

forces it. See [Fonts](configuration.md#fonts).

## The album art is a blocky mess

Same cause, same fix: `[ui] graphics = "kitty"`.

## `staramp remote` stops before the UI opens

It says why. Usually `ssh <host>` still wants a password or a host-key
confirmation, which the player has no way to show. Run `ssh <host>` once by
hand, then try again.

## Where the logs are

Under `~/.local/staramp/cache/`.

```sh
staramp -v                                  # much more in them
STARAMP_LOG=staramp::audio=debug staramp    # a full tracing filter
```
