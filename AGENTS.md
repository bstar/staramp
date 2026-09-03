# Working notes

Context that is not derivable from the code or the history, kept here rather
than in any one machine's notes because this is developed on both Linux and
macOS, with more than one assistant, and the repository is the only thing all
of them see.

`CLAUDE.md` is a symlink to this file, so there is one copy to keep true.

## Where the macOS work lives

Neither branch is merged.

| branch | what it carries |
| --- | --- |
| `main` | Linux only. `flake.nix` names `x86_64-linux` and `aarch64-linux` and rejects darwin explicitly. |
| `remote-libraries-over-ssh` | The SSH remote-library feature, and with it the whole macOS port: `mpris_stub.rs`, target-gated zbus, `cfg`-gated `ipc.rs`, `aarch64-darwin` in the flake, the `macos-14` CI job. |
| `darwin-output-adaptation` | Off the above. The fixes that make the port correct rather than merely compiling. |

Start darwin work from `darwin-output-adaptation`, not `main`.

## Building on Linux

Use the Nix flake for all builds and checks. Do not assume `cargo` or the
system audio dependencies are available in the ambient shell.

```sh
nix develop -c cargo build --release
```

Run other Cargo commands through the flake in the same way, for example
`nix develop -c cargo test --all`.

## Listening history is user data

`activity.sqlite` is deliberately separate from `index.sqlite`. The index is
rebuildable and a remote index is replaced when it is downloaded; plays,
skips, and queued scrobbles must survive both. Library connections attach the
activity database so smart-playlist fields can join `activity.track_stat`.

Only the window that owns playback records activity. Mirroring windows send
transport and scrobble-setting changes to the owner over IPC; otherwise every
open TUI would record and submit the same listen. Local history is always on.
Last.fm and ListenBrainz are optional, independent providers, and credentials
belong in the mode-0600 `credentials.toml`, never in the ordinary config.

## Building on macOS

Both routes are verified on `aarch64-darwin`, against Homebrew's ffmpeg 8.1.2
and nixpkgs' 9.0.1.

```sh
brew install ffmpeg pkg-config
export LIBCLANG_PATH="$(dirname "$(xcrun --find clang)")/../lib"
cargo build --release
```

`LIBCLANG_PATH` is asked of `xcrun`, not `xcode-select -p`. Where full Xcode is
selected -- the CI runners, and any machine with Xcode installed --
`$(xcode-select -p)/usr/lib` holds no libclang at all; it lives under
`Toolchains/XcodeDefault`.

```sh
nix develop -c cargo build --release
```

`x86_64-darwin` is deliberately absent: nixpkgs 26.11 dropped it, and naming it
fails evaluation rather than merely failing to build. An Intel Mac needs the
plain `cargo` route.

## CI does not run on branch pushes

`ci.yml` triggers on `push` to `main` and on `pull_request` only, so a branch
push matches neither. The `macos-14` job has therefore never executed, which is
how three darwin defects reached a branch that already contained the job meant
to catch them: a `cfg(target_os = "linux")` function called from an ungated
test, a `sun_path` limit measured against Linux's 108 bytes where darwin's is
104, and a `LIBCLANG_PATH` that resolves to a directory containing no libclang.

**This is deliberate and is not to be changed yet.** CI work on the macOS job
starts once the macOS build has stabilised *and reached parity with Linux*.
Parity is the gate, not "it compiles". Until then the local loop is the check:

```sh
cargo clippy --all-targets -- -D warnings -A dead_code
cargo test --all
```

Those are exactly the two steps the `macos-14` job runs.

## Not yet verified

- **Remote cue tracks.** The decode thread takes its index through
  `vfs.index_path()` rather than always opening the local one. Reasoned from
  the code -- `ui/app.rs` was already fixed the same way and this call site was
  missed -- but never exercised. Needs a live SSH host serving a cue album.
- **The transport buttons on a Mac terminal.** They are pictures drawn with
  the same `ratatui-image` path the cover art uses, so wherever the cover
  shows as a picture the buttons should too -- iTerm2, Ghostty, WezTerm,
  kitty -- and Terminal.app gets the ASCII text. Exercised in kitty on Linux
  only. Sixel terminals transmit the whole image every frame rather than
  once, so five buttons may cost there.

## Where macOS is not yet at parity

- No MPRIS equivalent. It is compiled out, not stubbed at runtime. macOS Now
  Playing is reachable (`MPNowPlayingInfoCenter`, and `objc2` is already in the
  graph via cpal) but wants an `NSApplication` run loop on the main thread,
  which the TUI owns.
