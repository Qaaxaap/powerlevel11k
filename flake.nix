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
          nativeCheckInputs = with pkgs; [ zsh ];
          preCheck = ''
            export LANG=C.UTF-8
            export LOCALE_ARCHIVE=${localesOf pkgs}/lib/locale/locale-archive
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
