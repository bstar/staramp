# Contributing

Thanks for looking. This is a small project with a maintainer who works on it
in the evenings, so the most useful thing you can do before writing code is
open an issue and say what you have in mind.

## Getting it to build

The one-command path, which sets everything below for you:

```sh
nix develop
cargo build
```

Without Nix you need a Rust toolchain (1.90 or newer) and:

```
pkg-config  clang  libclang-dev
libasound2-dev  libdbus-1-dev
libavformat-dev  libavcodec-dev  libavutil-dev  libswresample-dev
```

Not libavfilter, libavdevice or libswscale. Those are video plumbing, and
`ffmpeg-next` is pinned to the decode and resample features so they are never
linked.

**The thing that makes a first build fail** is `LIBCLANG_PATH`. `ffmpeg-next`
runs bindgen, which loads libclang at build time and does not search for it:

```sh
export LIBCLANG_PATH=/usr/lib             # Arch
export LIBCLANG_PATH=/usr/lib/llvm-16/lib # Debian, Ubuntu; match your version
```

If the ffmpeg headers are somewhere unusual, bindgen needs pointing at them
too:

```sh
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/ffmpeg/include"
```

The build also fetches [starkit](https://github.com/bstar/starkit) from git --
it is this project's own shared foundation, pinned to a tag rather than
published to crates.io, and it brings ratatui, crossterm, ratatui-image and
image with it. So the first build needs network, and `nix build` needs it too:
the flake sets `cargoLock.allowBuiltinFetchGit`, which fetches outside the
sandbox rather than carrying a hash that would have to be recomputed on every
starkit tag.

To work on starkit and this at the same time, check it out beside this
repository and point the build at it with an untracked `.cargo/config.toml`:

```toml
[patch."https://github.com/bstar/starkit"]
starkit = { path = "../starkit" }
```

It is in `.gitignore`, because a committed one points CI at a path that does
not exist. Delete it once the change is tagged and this repository has taken
the new tag.

## What CI will run

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings -A dead_code
cargo test --all
./scripts/check-version.sh
```

`dead_code` is allowed because a few modules land complete and tested ahead of
the UI that will reach them. Every other lint is an error.

Tests that need a real music library are gated behind `STARAMP_TEST_LIBRARY`
and skip cleanly when it is unset, so `cargo test` works on a machine with no
music on it.

## Two rules that are not visible from the type system

**Nothing allocates, locks or does I/O in the audio callback.** The output
callback runs on a real-time thread with a hard deadline; a `malloc` that
happens to take a slow path there is an audible glitch. Everything it needs is
pre-allocated and reaches it through a lock-free ring or an atomic. If a change
puts a `Vec::push`, a `Mutex::lock` or a `println!` inside it, that is a bug
however well it tests.

**Nothing waits on the filesystem in a frame.** The library may be on a
removable disk, and a `stat` on a dead mount blocks for as long as the kernel
feels like. Art lookups, scanning and network requests run on their own threads
with their own read-only handles to the index.

## Packaging

Release targets are Linux Nix, Linux AppImage (x86_64), and a native macOS
Apple Silicon archive. `scripts/build-dist.sh nix` builds the Nix package;
`./scripts/build-dist.sh appimage` uses Docker or Podman with an isolated
old-glibc build directory; `./scripts/build-dist.sh macos` runs on a Mac.
Debian, Arch and standalone Linux tarballs are no longer release targets.

## Taking a new starkit

Its own commit, "Take starkit 0.Y", and nothing else in it:

1. Change the `tag` and the `version` requirement on the `starkit` dependency
   in `Cargo.toml`.
2. `cargo update -p starkit` so `Cargo.lock` records the new revision.
3. Run the checks above. starkit's own CI has a job that builds both of its
   consumers against the tip of the library, so a break should have been
   caught there first, but the version this repository actually pins is the
   one that matters.

Keeping it separate is the point: what arrived with the new version is then
one diff to read rather than a line buried in a feature commit. starkit is
0.x, so a minor bump may change an API and a patch bump may not.

## Releasing

1. Bump `version` in `Cargo.toml` and refresh `Cargo.lock` through the Nix
   devshell; run `./scripts/check-version.sh` there too.
2. Add the release to `CHANGELOG.md` and run the checks above.
3. Commit, tag `vX.Y.Z`, and push. The release workflow validates Nix, builds
   the AppImage and macOS archive, and verifies the AppImage across distributions.
4. Review the draft release with its checksums and provenance before publishing.

A manual dispatch on `main` creates downloadable workflow artifacts without a
release or a tag. A tag dispatch creates the same draft as a tag push.

## Commit messages

Present tense, plain prose, no conventional-commits prefix. What the commit
does and, where it is not obvious, why. The existing log is the style guide.

## Comments

The codebase explains decisions rather than mechanics, and usually says what
was measured. If you change something a comment justifies, change the comment.
If you leave a comment that says something is a certain way for a reason, make
sure the reason is true.
