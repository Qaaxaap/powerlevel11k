# p11k task runner. `just` with no recipe lists these; `just check` is what a pull request
# must pass. Nothing here is CI-only: every recipe runs locally the same way.
set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# List the recipes.
default:
    @just --list

# Format the workspace.
fmt:
    cargo fmt --all

# Verify the formatting without writing.
fmt-check:
    cargo fmt --all -- --check

# Lint with the warning set CI uses.
lint:
    cargo clippy --all-targets --locked -- -D warnings

# Run every test.
test:
    cargo test --all --locked

# Everything a pull request must pass.
check: fmt-check lint test

# Build the binaries.
build:
    cargo build --release

# Build through nix, the way a release does.
nix-build:
    nix build

# Build the checks derivation (the test suite inside the nix sandbox).
nix-check:
    nix flake check

# Check dependency licences, bans and advisories.
deny:
    cargo deny check

# Check spelling.
spell:
    typos

# Regenerate the gettext template and merge it into every catalog.
i18n:
    tools/i18n.sh extract
    tools/i18n.sh update

# Regenerate CHANGELOG.md from the Conventional Commit history.
changelog:
    git-cliff --output CHANGELOG.md

# Re-record the demo captures (needs zsh, bash, fish, python3, tmux, agg, ImageMagick).
demo:
    tools/demo/capture.sh
