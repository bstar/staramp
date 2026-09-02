{
  description = "staramp — a Winamp-feel terminal music player";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      # Home-manager module, so staramp can be installed and configured
      # declaratively the way the rest of a NixOS setup is.
      hmModule = { config, lib, pkgs, ... }:
        let cfg = config.programs.staramp;
        in {
          options.programs.staramp = {
            enable = lib.mkEnableOption "staramp terminal music player";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "The staramp package to use.";
            };
            libraryRoot = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/mnt/music";
              description = "Where the music library lives.";
            };
            playlistDir = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Directory of .m3u playlists, read and written in place.
                Pointing this at MPD's own playlist directory is supported and
                intended: staramp writes the same URI form MPD does.
              '';
            };
            theme = lib.mkOption {
              type = lib.types.str;
              default = "winamp-classic";
            };
            stylix.enable = lib.mkEnableOption ''
              deriving a staramp theme from the active Stylix base16 scheme, so
              the player matches the rest of the desktop automatically
            '';
            settings = lib.mkOption {
              type = lib.types.attrs;
              default = { };
              description = "Extra config.toml settings, merged last.";
            };
          };

          config = lib.mkIf cfg.enable (lib.mkMerge [
            {
              home.packages = [ cfg.package ];
              # staramp keeps everything under one directory rather than
              # spreading it across the XDG roots, so this is not xdg.configFile.
              home.file.".local/staramp/config.toml".source =
                (pkgs.formats.toml { }).generate "staramp-config.toml" (
                  lib.filterAttrs (_: v: v != null) {
                    library_root = cfg.libraryRoot;
                    playlist_dir = cfg.playlistDir;
                    theme = if cfg.stylix.enable then "stylix" else cfg.theme;
                  } // cfg.settings
                );
            }
            (lib.mkIf cfg.stylix.enable {
              home.file.".local/staramp/themes/stylix.toml".text =
                let c = config.lib.stylix.colors;
                in ''
                  # Generated from the active Stylix scheme.
                  [meta]
                  name = "Stylix"
                  id = "stylix"
                  variant = "${config.stylix.polarity}"

                  [base16]
                '' + lib.concatMapStringsSep "\n"
                  (n: ''base${n} = "#${c."base${n}"}"'')
                  [ "00" "01" "02" "03" "04" "05" "06" "07"
                    "08" "09" "0A" "0B" "0C" "0D" "0E" "0F" ]
                  + "\n";
            })
          ]);
        };
    in
    {
      homeManagerModules.staramp = hmModule;
      homeManagerModules.default = hmModule;
      overlays.default = final: prev: {
        staramp = self.packages.${final.stdenv.hostPlatform.system}.default;
      };
    }
    # Explicit rather than eachDefaultSystem, which would also claim systems
    # nobody has built this on. ALSA and D-Bus are Linux-only and conditioned
    # out below; on darwin cpal reaches CoreAudio and there is no MPRIS.
    #
    # aarch64-darwin only. nixpkgs 26.11 dropped x86_64-darwin outright --
    # naming it here fails evaluation with a release note rather than a build
    # error, so an Intel Mac needs a 26.05 nixpkgs or the plain cargo build
    # documented in the README.
    // flake-utils.lib.eachSystem [
      "x86_64-linux"
      "aarch64-linux"
      "aarch64-darwin"
    ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # One version, read rather than repeated. scripts/check-version.sh
        # asserts the copies that cannot be derived (Cargo.lock, PKGBUILD).
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

        #   ffmpeg   : libavformat/libavcodec, linked in-process by ffmpeg-next
        #   alsa-lib : cpal's Linux backend (reaches PipeWire via pipewire-alsa)
        #   dbus     : MPRIS via zbus
        #
        # Neither of the last two exists on darwin: cpal goes to CoreAudio
        # through the SDK that stdenv already provides, and MPRIS is compiled
        # out entirely. `alsa-lib` is `platforms = linux`, so referring to it
        # unconditionally breaks *evaluation* there, not merely the build.
        linuxLibs = pkgsFor:
          pkgsFor.lib.optionals pkgsFor.stdenv.hostPlatform.isLinux [
            pkgsFor.alsa-lib
            pkgsFor.dbus
          ];
        runtimeLibs = [ pkgs.ffmpeg ] ++ linuxLibs pkgs;
        buildTools = with pkgs; [ pkg-config clang ];
        libclangPath = "${pkgs.llvmPackages.libclang.lib}/lib";

        # ffmpeg-next runs bindgen, which needs the headers of what it binds.
        bindgenArgs = pkgsFor:
          "-I${pkgsFor.ffmpeg.dev}/include"
          + pkgsFor.lib.optionalString pkgsFor.stdenv.hostPlatform.isLinux
            " -I${pkgsFor.alsa-lib.dev}/include";

        mkStaramp = { pkgsFor ? pkgs }:
          pkgsFor.rustPlatform.buildRustPackage {
            pname = "staramp";
            version = cargoToml.package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgsFor; [ pkg-config clang ];
            buildInputs = [ pkgsFor.ffmpeg ] ++ linuxLibs pkgsFor;

            LIBCLANG_PATH = "${pkgsFor.llvmPackages.libclang.lib}/lib";
            BINDGEN_EXTRA_CLANG_ARGS = bindgenArgs pkgsFor;

            # freedesktop assets, which mean nothing on macOS.
            postInstall = pkgsFor.lib.optionalString pkgsFor.stdenv.hostPlatform.isLinux ''
              install -Dm644 packaging/staramp.desktop \
                $out/share/applications/staramp.desktop
              install -Dm644 packaging/staramp.png \
                $out/share/icons/hicolor/256x256/apps/staramp.png
              install -Dm644 packaging/staramp.svg \
                $out/share/icons/hicolor/scalable/apps/staramp.svg
            '';

            meta = with pkgsFor.lib; {
              description = "A Winamp-feel terminal music player for local libraries";
              homepage = "https://github.com/bstar/staramp";
              license = licenses.mit;
              mainProgram = "staramp";
              platforms = platforms.linux ++ platforms.darwin;
            };
          };
      in
      {
        packages.default = mkStaramp { };
        packages.staramp = mkStaramp { };

        # There was a `headless` package here, built against ffmpeg-headless
        # for release tarballs. It is gone. Its stated reason was a much
        # smaller closure, and when that was finally measured it was 300.7 MiB
        # against 303.8 MiB -- about 1%. It also had no consumer left: the
        # portable tarball is built in a Debian container for its old glibc,
        # not from nix. What it did have was a cost, a second full compile of
        # the crate in CI, which is most of why a release sat unpublished for
        # forty minutes.
        #
        # (A fully static musl build was attempted and abandoned separately:
        # pkgsStatic cannot evaluate ffmpeg's transitive dependencies,
        # libpulseaudio via libopenmpt/mpg123 and then elfutils. The AppImage
        # covers that ground properly.)

        # `nix flake check` used to check nothing of its own. The package is
        # here because buildRustPackage runs `cargo test` as part of building
        # it, so one command covers the test suite too.
        checks = {
          inherit (self.packages.${system}) default;

          fmt = pkgs.runCommand "cargo-fmt"
            { nativeBuildInputs = [ pkgs.rustfmt ]; }
            ''
              cd ${./.}
              find src -name '*.rs' -print0 \
                | xargs -0 rustfmt --check --edition 2021
              touch $out
            '';
        };

        formatter = pkgs.nixpkgs-fmt;

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [
            rustc cargo rustfmt clippy rust-analyzer
            # scripts/check-version.sh reads `cargo metadata`.
            jq
          ])
          # Only ever used to build a .deb, which only happens on Linux.
          ++ pkgs.lib.optional pkgs.stdenv.hostPlatform.isLinux pkgs.cargo-deb
          ++ buildTools ++ runtimeLibs;

          LIBCLANG_PATH = libclangPath;
          BINDGEN_EXTRA_CLANG_ARGS = bindgenArgs pkgs;

          shellHook = ''
            echo "staramp devshell · rustc $(rustc --version | cut -d' ' -f2) · ffmpeg $(ffmpeg -version | head -1 | cut -d' ' -f3)"
          '';
        };
      });
}
