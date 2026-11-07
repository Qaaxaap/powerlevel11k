{
  description = "powerlevel11k (p11k) — a Rust successor to powerlevel10k";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  # The devShell pins the MSRV from `rust-toolchain.toml`; the release workflow uses the same
  # version through rustup. nixpkgs alone would drift to whatever Rust it packages.
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
      eachSystem =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
      localesOf =
        pkgs:
        pkgs.glibcLocales.override {
          allLocales = false;
          locales = [
            "C.UTF-8/UTF-8"
            "en_US.UTF-8/UTF-8"
            "zh_CN.UTF-8/UTF-8"
          ];
        };
      # iai-callgrind's harness shells out to this runner, and its version has to match the
      # `iai-callgrind` dependency in `crates/p11k-gitstatus/Cargo.toml`. nixpkgs does not
      # package it, so the flake builds it from the crate. Bump both together.
      iaiCallgrindRunner =
        pkgs:
        pkgs.rustPlatform.buildRustPackage rec {
          pname = "iai-callgrind-runner";
          version = "0.16.1";
          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-wJTwaqAz8GWCJ/l9GRXYBVBkpPYrWxN4VQ7GdRFXmzM=";
          };
          cargoHash = "sha256-4N7P23bCeeJee/Cm3sSORByh+HzflOENqYqpu629mpA=";
          doCheck = false;
        };

      # The visual render check runs p11k inside a real kitty terminal in a headless sway
      # compositor. It has its own devShell: sway, kitty and grim are heavy, and only the render
      # check needs them. The check enters that devShell with `--ignore-environment` so the run
      # cannot pick anything up from the caller.
      #
      # kitty is a GL client and the compositor has no GPU on a runner without /dev/dri, so the
      # renderer driver paths have to be injected on non-NixOS systems.
      glWrap =
        pkgs: pkg:
        pkgs.writeShellScriptBin (pkgs.lib.getName pkg) ''
          export LD_LIBRARY_PATH=${pkgs.mesa}/lib:${pkgs.libglvnd}/lib
          export __EGL_VENDOR_LIBRARY_FILENAMES=${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json
          export LIBGL_DRIVERS_PATH=${pkgs.mesa}/lib/dri
          export GBM_BACKENDS_PATH=${pkgs.mesa}/lib/gbm
          export __GLX_VENDOR_LIBRARY_NAME=mesa
          exec ${pkg}/bin/${pkgs.lib.getName pkg} "$@"
        '';

      # The captured frames contain the font raster, so the family is part of the fixed
      # environment. Maple Mono NF is the family the reference frames are made with; DejaVu
      # covers characters outside the family.
      #
      # The file must not include /etc/fonts/conf.d: that directory differs between the local
      # host and the CI runner, and the hinting/subpixel settings in it change the glyph raster.
      # The raster settings are pinned here instead.
      visualFontsConf =
        pkgs:
        pkgs.writeText "p11k-visual-fonts.conf" ''
          <?xml version="1.0" encoding="UTF-8"?>
          <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
          <fontconfig>
            <dir>${pkgs.dejavu_fonts}/share/fonts</dir>
            <dir>${pkgs.maple-mono."NF"}/share/fonts</dir>
            <cachedir>/tmp/p11k-visual-fontconfig</cachedir>
            <match target="font">
              <edit name="antialias" mode="assign"><bool>true</bool></edit>
              <edit name="hinting" mode="assign"><bool>true</bool></edit>
              <edit name="hintstyle" mode="assign"><const>hintslight</const></edit>
              <edit name="rgba" mode="assign"><const>none</const></edit>
              <edit name="lcdfilter" mode="assign"><const>lcdnone</const></edit>
            </match>
          </fontconfig>
        '';

    in
    {
      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            rust-analyzer
            gettext
            zsh
            # The engine's bash tests drive readline: the placeholder protocol, clear-screen and
            # the completion pager. nixpkgs' default `bash` is the non-interactive build (no
            # readline, no `bind`), so the proxy tests would fail on it for the wrong reason.
            bashInteractive
            # The engine proxies fish and pwsh too, and their protocol layers are exercised by
            # tests/proxy.rs: the shells have to come from here rather than from whatever happens
            # to be on a developer's PATH.
            fish
            powershell
            # pwsh's PSReadLine asks the terminal for the cursor position on every keystroke and
            # blocks until the reply arrives, so its tests drive a tmux session: a bare pty never
            # answers, and without the answer pwsh's redraws land on wrong coordinates.
            tmux
            # Task runner, changelog generator, and the dependency and spelling checks:
            # `just check`, `just changelog`, `just deny`, `just spell`. They live here so CI
            # runs the same commands a developer runs.
            just
            git-cliff
            cargo-deny
            typos
            # `just nix-fmt-check` checks the flake itself.
            nixfmt
            # `just demo` runs tools/demo/capture.sh; its other tools (agg, ImageMagick,
            # fontconfig) are heavy and are pulled on demand by that recipe instead of sitting
            # in every devShell. python3 is small and runs record.py.
            python3
            git
            pkg-config
            cmake
            # `just bench-iai` counts instructions with Callgrind: the harness needs the runner,
            # and the runner needs valgrind. Both come from here so no setup step is involved.
            valgrind
            (iaiCallgrindRunner pkgs)
            # The issue #29 regression case drives zsh-autocomplete and zsh-syntax-highlighting;
            # the gitstatusd differential tests compare against the original binary. Without these
            # the tests skip, and a local run would not match CI.
            zsh-autocomplete
            zsh-syntax-highlighting
            gitstatus
          ];
          shellHook = ''
            export P11K_TEST_ZSH_AUTOCOMPLETE=${pkgs.zsh-autocomplete}/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh
            export P11K_TEST_ZSH_SYNTAX_HIGHLIGHTING=${pkgs.zsh-syntax-highlighting}/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh
            export GITSTATUSD_PATH=${pkgs.gitstatus}/bin/gitstatusd
          '';
          LOCALE_ARCHIVE = "${localesOf pkgs}/lib/locale/locale-archive";
          RUST_BACKTRACE = "1";
        };

        # The render check's toolchain. `just render-check` enters this shell with
        # `--ignore-environment` for the test half, so everything the run needs is set here and
        # nothing comes from the caller. The build half stays in the default devShell so cargo
        # uses the same source/target caches.
        visual = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            (glWrap pkgs pkgs.sway)
            pkgs.sway
            # sway starts through dbus-run-session; without dbus in PATH the wrapper cannot find
            # the session bus daemon once the caller's environment is cleared.
            pkgs.dbus
            (glWrap pkgs pkgs.kitty)
            pkgs.kitty
            pkgs.grim
            pkgs.bats
            # `bats --jobs` shells out to GNU parallel.
            pkgs.parallel
            pkgs.imagemagick
            pkgs.zsh
            pkgs.bashInteractive
            pkgs.fish
            pkgs.powershell
            pkgs.git
            pkgs.jq
            pkgs.coreutils
            pkgs.gnused
            pkgs.gnugrep
            pkgs.gawk
          ];
          shellHook = ''
            export LANG=C.UTF-8
            export LC_ALL=C.UTF-8
            export FONTCONFIG_FILE=${visualFontsConf pkgs}
            export P11K_VISUAL_REFERENCE=${./tools/visual/reference.kdl}
            export P11K_VISUAL_KITTY_CONF=${./tools/visual/kitty.conf}
            export P11K_VISUAL_ARCH=${pkgs.stdenv.hostPlatform.system}
            export P11K_VISUAL_RECORD_TUI=${./tools/visual/record-tui.sh}
            export P11K_VISUAL_DBUS_DAEMON=${pkgs.dbus}/bin/dbus-daemon
            export P11K_VISUAL_DBUS_CONF=${pkgs.dbus}/share/dbus-1/session.conf
          '';
        };
      });

      formatter = eachSystem (pkgs: pkgs.nixfmt-rfc-style);

      checks = eachSystem (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "p11k-tests";
          inherit version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          # The checks serve developers, not release artifacts: build them in the debug profile
          # so they skip the release optimization pass.
          buildType = "debug";
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            gettext
            git
          ];
          nativeCheckInputs = with pkgs; [
            zsh
            bashInteractive
            fish
            powershell
            tmux
            # The transient-prompt hook has to coexist with plugins that hook the same zle
            # widgets; issue #29 needed zsh-autocomplete plus zsh-syntax-highlighting to
            # reproduce, so the regression case loads the real ones instead of a mock.
            zsh-autocomplete
            zsh-syntax-highlighting
          ];
          preCheck = ''
            export LANG=C.UTF-8
            export LOCALE_ARCHIVE=${localesOf pkgs}/lib/locale/locale-archive
            export TERMINFO_DIRS=${pkgs.ncurses.out}/share/terminfo
            export P11K_TEST_ZSH_AUTOCOMPLETE=${pkgs.zsh-autocomplete}/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh
            export P11K_TEST_ZSH_SYNTAX_HIGHLIGHTING=${pkgs.zsh-syntax-highlighting}/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh
            export HOME=$TMPDIR
            git config --global user.email "ci@example.com"
            git config --global user.name "CI"
          '';
        };
      });

      packages = eachSystem (pkgs: {
        # Exposed so the devShell can pull it in and so `nix build .#iai-callgrind-runner`
        # works for a manual check.
        iai-callgrind-runner = iaiCallgrindRunner pkgs;
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "p11k";
          inherit version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            gettext
          ];
          doCheck = false;
          P11K_LOCALEDIR = "${placeholder "out"}/share/locale";
          postInstall = ''
            for po in crates/p11k-engine/po/*.po; do
              lang=$(basename "$po" .po)
              mkdir -p "$out/share/locale/$lang/LC_MESSAGES"
              msgfmt -o "$out/share/locale/$lang/LC_MESSAGES/p11k.mo" "$po"
            done
          '';
          meta = {
            description = "powerlevel10k-compatible prompt engine (pty host + gitstatusd-compatible daemon)";
            homepage = "https://github.com/Qaaxaap/powerlevel11k";
            license = nixpkgs.lib.licenses.lgpl3Plus;
            platforms = pkgs.lib.platforms.linux;
            mainProgram = "p11k";
          };
        };
      });
    };
}
