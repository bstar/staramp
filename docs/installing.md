# Installing

Built packages for the current release are on the
[releases page](https://github.com/bstar/staramp/releases/latest). Pick the
route that matches your machine.

| Route | For | Needs on the machine |
| --- | --- | --- |
| [AppImage](#appimage) | any desktop Linux | ALSA, which every desktop already has |
| [`.deb`](#debian-and-ubuntu) | Debian 12, Debian 13, Ubuntu 24.04 | nothing else; dependencies are declared |
| [Arch](#arch) | Arch and derivatives | `makepkg` |
| [Nix](#nix-and-nixos) | NixOS, or Nix on Linux or macOS | Nix with flakes |
| [Portable tarball](#portable-tarball) | distributions without a package | `alsa-lib`, `ffmpeg`, `dbus` |
| [macOS](#macos) | Apple Silicon | Nix, or Homebrew and a Rust toolchain |
| [From source](#from-source) | anything else | a Rust toolchain and a few development packages |

## Nix and NixOS

```sh
nix run github:bstar/staramp
```

Declaratively, with the home-manager module:

```nix
{
  inputs.staramp.url = "github:bstar/staramp";

  # in your home-manager config:
  imports = [ inputs.staramp.homeManagerModules.staramp ];
  programs.staramp = {
    enable = true;
    libraryRoot = "/mnt/music";
    # Read and write MPD's own playlist directory. STAR/AMP writes the same
    # URI form, so both stay in sync.
    playlistDir = "${config.home.homeDirectory}/.config/mpd/playlists";
    stylix.enable = true;   # derive the theme from your base16 scheme
  };
}
```

## AppImage

The one that needs nothing installed. Download it, make it executable, run it:

```sh
chmod +x staramp-*-x86_64.AppImage
./staramp-*-x86_64.AppImage
```

It carries its own ffmpeg, which is the whole reason it exists: the libav
sonames differ on every distribution, so a plain binary built against one
refuses to start on the next.

It uses the system's ALSA rather than bundling it. ALSA is what loads the
plugin that reaches PipeWire or PulseAudio on the host, and a bundled copy
would find nothing to play through.

> [!TIP]
> On a distribution that no longer ships libfuse2, run it as
> `./staramp-*.AppImage --appimage-extract-and-run`.

## Debian and Ubuntu

A `.deb` per Debian generation is attached to each release, because the
dependency on libavcodec is version specific and one file cannot serve them
all.

| Package | Release |
| --- | --- |
| `staramp_0.1.0-1.bookworm_amd64.deb` | Debian 12 |
| `staramp_0.1.0-1.trixie_amd64.deb` | Debian 13 |
| `staramp_0.1.0-1.ubuntu24.04_amd64.deb` | Ubuntu 24.04 |

Or build your own:

```sh
sudo apt install pkg-config clang libclang-dev libasound2-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswresample-dev
cargo install cargo-deb && cargo deb
```

The Debian and Arch packages are both built and then installed from clean
containers in CI, so the dependency lists here are the ones that actually work
rather than the ones that ought to.

## Arch

```sh
cd packaging && makepkg -si
```

## Portable tarball

For distributions without a package:

```sh
tar xf staramp-*-x86_64-linux-gnu.tar.gz
cd staramp-* && ./staramp
```

- Needs `alsa-lib`, `ffmpeg` and `dbus` present.
- Built against glibc 2.31, so it runs on Debian 11 and later, Ubuntu 20.04
  and later, and RHEL 9 and later.
- There is no fully static musl build. ffmpeg's dependency graph does not
  cross-compile cleanly under `pkgsStatic`, and the AppImage covers the same
  ground properly.

## macOS

Apple Silicon, either way round. With Nix:

```sh
nix run github:bstar/staramp
```

Or from source with Homebrew, which needs no Nix on the Mac:

```sh
brew install ffmpeg pkg-config
cargo build --release
```

> [!NOTE]
> If the build stops in `ffmpeg-sys-next`, it is bindgen looking for libclang.
> The active Apple toolchain always carries one, and `xcrun` is what knows
> where. With full Xcode installed it is under `Toolchains/XcodeDefault`, not
> the `usr/lib` beside `xcode-select -p`:
>
> ```sh
> export LIBCLANG_PATH="$(dirname "$(xcrun --find clang)")/../lib"
> ```

What is different on a Mac:

- Output goes through CoreAudio. No ALSA.
- MPRIS is compiled out entirely rather than shipped as two megabytes that
  cannot run. No D-Bus.
- Everything else is the same program, and a Mac can be either end of a
  [remote library](remote.md): the machine playing, or the machine holding
  the files.

Intel Macs need the plain `cargo` build above. There is no Nix package for
them, because nixpkgs 26.11 dropped `x86_64-darwin`; a 26.05 nixpkgs still has
it if you would rather stay declarative.

## From source

You need:

- a Rust toolchain, 1.90 or newer
- `libclang`, for bindgen
- development packages for `alsa-lib` (Linux only)
- development packages for four ffmpeg libraries: libavformat, libavcodec,
  libavutil and libswresample

Not libavfilter, libavdevice or libswscale. Those are video plumbing, and
`ffmpeg-next` is pinned to the decode and resample features so they are never
linked.

```sh
nix develop -c cargo build --release   # or supply those yourself
```

[CONTRIBUTING.md](../CONTRIBUTING.md) has the rest, including the one
environment variable (`LIBCLANG_PATH`) that a first build outside Nix usually
needs.
