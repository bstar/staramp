# Installing

Built packages for the current release are on the
[releases page](https://github.com/bstar/staramp/releases/latest). Pick the
route that matches your machine.

| Route | For | Needs on the machine |
| --- | --- | --- |
| [AppImage](#appimage) | any desktop Linux | ALSA, which every desktop already has |
| [Nix](#nix-and-nixos) | NixOS, or Nix on Linux or macOS | Nix with flakes |
| [macOS](#macos) | Apple Silicon | release archive or Nix; FFmpeg for STAR/AMP |
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



## macOS

Download `staramp-<version>-aarch64-apple-darwin.tar.gz` from the releases page,
extract it, and run the enclosed `staramp` executable. The archive is unsigned.
Install its audio runtime with `brew install ffmpeg`.


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
