# p11k task runner. `just` with no recipe lists these; `just verify` is the full local gate.
# Nothing here is CI-only: the workflows call these recipes and add nothing beyond checking out
# the repository and uploading the artifacts they produce.
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

# Verify the Nix files are formatted (the flake formatter is nixfmt).
nix-fmt-check:
    nixfmt --check flake.nix

# Check spelling.
spell:
    typos

# Check the workflow files: syntax with actionlint, security with zizmor.
workflows:
    actionlint .github/workflows/*.yml
    zizmor --min-severity high .github/workflows

# Lint without running the tests: formatting, clippy, spelling and the workflow files.
lint: fmt-check nix-fmt-check spell workflows
    cargo clippy --all-targets --locked -- -D warnings

# Check the Conventional Commit subjects in a range. Defaults to the last commit.
commit-msg range="HEAD~1..HEAD":
    cargo xtask commit-msg --range {{range}}

# Run every test.
test:
    cargo test --all --locked

# Run the differential tests against the original gitstatusd. The devShell sets GITSTATUSD_PATH.
differential:
    cargo test -p p11k-d --test compat -- --ignored

# Check the workspace with the MSRV Cargo.toml declares.
#
# This recipe needs rustup and therefore runs outside the devShell, whose toolchain is a plain
# build. `just msrv` in a devShell fails with "no such command: +1.95".
msrv:
    cargo xtask msrv

# Check dependency licences, bans and advisories.
deny:
    cargo deny check

# The fast local gate.
check: lint test

# Everything a pull request must pass.
verify: check deny differential

# Build the binaries.
build:
    cargo build --release --locked

# Build one target's release artifacts into target/dist/<triple>. The target is a triple such as
# x86_64-unknown-linux-gnu; see `target.rs` for the accepted set.
dist target:
    cargo xtask dist --target {{target}}

# Merge the checksums and dependency lists of every built target into target/dist.
assemble:
    cargo xtask assemble

# Check a release tag against Cargo.toml and CHANGELOG.md before pushing it.
release-check tag:
    cargo xtask release-check {{tag}}

# Build through nix and check that the package holds both binaries.
nix-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    out=$(nix build --no-link --print-out-paths)
    for bin in p11k p11k-d; do
        test -x "$out/bin/$bin"
    done
    "$out/bin/p11k" --version

# Build through nix, the way a release does.
nix-build:
    nix build

# Build the checks derivation (the test suite inside the nix sandbox).
nix-check:
    nix flake check

# Run the wall-clock benchmark. Quote its numbers with the machine they came from; the machine's
# load shows up in them.
bench:
    cargo bench -p p11k-gitstatus --bench build_fields_criterion

# Run the instruction-count benchmark. Callgrind counts do not move with the machine, so this is
# what judges a change. The devShell provides valgrind and the iai-callgrind runner.
bench-iai:
    cargo bench -p p11k-gitstatus --bench build_fields_iai

# Regenerate CHANGELOG.md from the Conventional Commit history. Pass a tag to write that
# version's section instead of Unreleased: `just changelog v1.0.0`.
changelog tag="":
    git-cliff --output CHANGELOG.md {{ if tag != "" { "--tag " + tag } else { "" } }}

# Re-record the demo captures. agg, ImageMagick and fontconfig are pulled on demand: they are
# heavy, and CI never runs this recipe. Set FONT_FILE to a font file when fc-match does not
# find the default Maple Mono family.
demo:
    nix shell nixpkgs#agg nixpkgs#imagemagick nixpkgs#fontconfig --command tools/demo/capture.sh

# Capture the reference prompt in a real terminal and compare the pixels with the committed
# reference frame. The build uses the default devShell so cargo caches apply; the capture and the
# comparison run in the visual devShell with --ignore-environment, so nothing comes from the
# caller. See tools/visual/README.md.
render-check *args:
    cargo build -p p11k-engine --bin p11k
    nix develop .#visual --ignore-environment --command bash tools/visual/check.sh {{args}}

# Capture new frames after a deliberate rendering change. The captures go to
# tools/visual/expected-pending. They are not references until a person reviews and accepts
# them. Pass -j/--jobs N to change the bats parallelism.
render-update *args:
    cargo build -p p11k-engine --bin p11k
    nix develop .#visual --ignore-environment --command bash tools/visual/check.sh --update {{args}}

# Copy the reviewed pending frames into tools/visual/expected. Run this only after a person
# checks each image against the intended layout. The frames then become the reference that CI
# compares against.
render-accept:
    bash tools/visual/accept.sh
