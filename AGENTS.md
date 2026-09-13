# Working notes

Context that is not derivable from the code or the history, kept here rather
than in any one machine's notes because this is developed on both Linux and
macOS, with more than one assistant, and the repository is the only thing all
of them see.

This file is the only assistant-facing notes file in the repository. Do not
add a tool-specific notes file or directory beside it; every assistant reads
`AGENTS.md`.

## One branch, one history

Everything lives on `main`, which builds on `x86_64-linux`, `aarch64-linux`
and `aarch64-darwin`. The SSH remote-library feature, the macOS port, the
darwin output fixes, Gravity Queue, staging imports and the rest were
developed on feature branches and folded in for the 0.1.0 release on
2026-09-11, when the history was flattened to a single commit. The
pre-flatten commits exist only on local `backup/*` branches on the machine
that did it; nothing on GitHub refers to them. Start new work from `main`.

## STAR/KIT

Everything that is not about playing music lives in a separate crate,
[starkit](https://github.com/bstar/starkit), checked out beside this one: the
theme engine and the sixteen theme files, the directory rule, file logging,
terminal graphics and the picture cache, the panel frame and the settings
list, the key table and the help overlay, atomic and mode-0600 file writes,
marquee and truncation, the HTTP agent defaults, and the layout engine and
text field written there for STAR/CORD. It also re-exports `ratatui`,
`crossterm`, `ratatui_image` and `image`, which is why none of those are
dependencies here any more and why every import spells them
`starkit::ratatui::` and so on. There is one copy of each in the build, and a
widget written in one repository fits a signature declared in the other.

It exists because of STAR/CORD, a terminal Discord client on the same
foundation. **That is the rule to remember: every public item in starkit has
two callers, and a change to one of its signatures has to be checked against
both.** They are `grep -rn "starkit::" ../staramp/src ../starcord/src`. A
change that is awkward at one and impossible at the other is a new function,
not a refactor.

The dependency is a git tag, and a local starkit change is invisible here
until it is tagged. To try one before it is:

```toml
# .cargo/config.toml -- untracked, and in .gitignore
[patch."https://github.com/bstar/starkit"]
starkit = { path = "../starkit" }
```

Delete it once the tag exists and this repository has moved to it. Moving to
a new tag is its own commit, "Take starkit 0.Y", carrying nothing but the
`Cargo.toml` bump and the `Cargo.lock` change, so that what came with the new
version is one diff to read rather than a line buried in a feature.

`Graphics::probe` must run **before** `term::init` enables raw mode: it writes
capability queries to the terminal and reads the replies off stdin, and after
raw mode it reads the user's keystrokes instead. starkit debug-asserts the
ordering across the crate boundary; a release build degrades to
`Graphics::disabled()` rather than corrupting the session.

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

Panel visibility is presentation, never service state. Closing the history
panel must not stop a provider enabled under `[scrobble]`, and closing the EQ
panel must not bypass an EQ enabled under `[eq]`. `ui.show_scrobbler` and
`ui.show_equalizer` only remember whether those panels are drawn. The former
name is retained for config and shared-view compatibility; the user-facing
module is named Activity. Internal `history` identifiers remain compatible.

Discord Rich Presence follows the same boundary. `[discord] enabled` is an
opt-in service setting, unrelated to Activity panel visibility, and only the
window that owns playback may publish it. It uses the local Discord desktop
IPC socket under the application name `STAR/AMP`; it has no bot token and
must clear the activity on stop, disable, and shutdown.
Keep the RPC socket open for the lifetime of the activity: Legcord/arRPC clears
the activity as soon as the socket closes. Send the application name in every
payload rather than relying on Legcord's asynchronous application lookup.
Discord cannot consume a local cover file; album artwork must be a public HTTPS
URL, resolved from MusicBrainz/Cover Art Archive independently of every
scrobbling provider. Discord presence must never require Last.fm credentials.

## Gravity Queue data boundaries

Gravity Queue reshapes only the unplayed suffix of the current queue. It must
not change membership, disturb the played prefix, split consecutive tracks
from the same CUE image, or silently replan while music is playing. Replanning
is an explicit user action. The resulting order is an ordinary queue edit and
can be saved as an ordinary playlist.

`sonic_feature` belongs in `index.sqlite`: analysis is derived, versioned by
both analyzer and model, invalidated by file identity, and safe to rebuild.
Explicit more-like/less-like signals belong in `activity.sqlite` alongside
listening history because they are user data and must survive rescans. Remote
libraries are analyzed on the machine that owns their index; do not pull whole
remote audio files merely to populate a local cache.

Do not add model weights based only on the license of their inference code.
The checkpoint itself must have documented commercial redistribution rights.
Until one does, keep the transparent spectral fallback and its distinct model
version so licensed embeddings can replace those rows later.

## Album artwork and catalog links

Clicking album artwork opens the full original externally; cycling belongs to
the visible previous/next controls, the wheel, and Album-focused keys. Embedded
and remote originals are materialized lazily in the rebuildable art cache.
`[art] viewer` is an argv list and must never be passed through a shell.

Music catalog links are explicit user actions. Prefer an exact Metal Archives
URL exposed by a MusicBrainz relationship, then a Metal Archives search for a
locally tagged metal genre; use exact MusicBrainz identifiers or MusicBrainz
search for everything else. Do not scrape Metal Archives or make catalog
access depend on scrobbling credentials.

## Library timeline semantics

The library-timeline creator always works from the canonical full-library
model, not the active playlist or its filter. Its default direction is oldest
album first. Filesystem chronology means creation time when the platform
provides it and modification time otherwise; first-indexed chronology is the
track's durable `added_at` value. Use the median track date for an album so one
replaced file does not move the whole record. Timeline dates are contextual
queue metadata and must not overwrite tagged release years. Within every
album, retain canonical disc, track, CUE, and natural filename ordering.

## Staging imports are filesystem transactions

`imports.sqlite` is a rebuildable index rooted at `[imports] staging_root`; it
must never be confused with the permanent index. An import preserves the
physical album folder and files it beneath the unmodified album-artist name.
Never rewrite tags to obtain a preferred directory layout. Copy into a hidden
temporary sibling, verify every file, atomically install it, and confirm a
permanent-library scan indexed it before removing the staging source.

Staging, library, and quarantine roots must be disjoint canonical paths.
Rejects and replaced permanent versions go to quarantine, and recursive
operations must not follow symlinks out of their configured root. A failed
copy or scan leaves the source intact and must not leave a partial album at the
final destination.

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
push matches neither. Before the history was flattened this is how three
darwin defects reached a branch that already contained the `macos-14` job
meant to catch them: a `cfg(target_os = "linux")` function called from an
ungated test, a `sun_path` limit measured against Linux's 108 bytes where
darwin's is 104, and a `LIBCLANG_PATH` that resolves to a directory containing
no libclang. All three are fixed on `main`, and every push to `main` and every
tag now runs the job.

**This is deliberate and is not to be changed yet.** Widening the trigger to
branches waits until the macOS build has stabilised *and reached parity with
Linux*. Parity is the gate, not "it compiles". Until then the local loop is
the check before a push to `main`:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings -A dead_code
cargo test --all
```

Those are the steps the `test` and `macos-14` jobs run.

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
