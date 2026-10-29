{
  description = "powerlevel11k (p11k) — a Rust successor to powerlevel10k";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
      eachSystem = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
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
    in
    {
      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
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
            git
            pkg-config
            openssl
            cmake
          ];
          LOCALE_ARCHIVE = "${localesOf pkgs}/lib/locale/locale-archive";
          RUST_BACKTRACE = "1";
        };
      });

      checks = eachSystem (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "p11k-tests";
          inherit version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            gettext
            git
          ];
          buildInputs = with pkgs; [ openssl ];
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
          buildInputs = with pkgs; [ openssl ];
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
