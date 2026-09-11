{
  description = "powerlevel11k (p11k) — a Rust successor to powerlevel10k";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
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

      packages = eachSystem (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "p11k";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            gettext
          ];
          buildInputs = with pkgs; [ openssl ];
          doCheck = false;
          meta = {
            description = "powerlevel10k-compatible prompt engine (pty host + gitstatusd-compatible daemon)";
            license = nixpkgs.lib.licenses.lgpl3Plus;
            mainProgram = "p11k";
          };
        };
      });
    };
}
