# p11k task runner. `just` with no recipe lists these; `just verify` is what CI runs.
# Nothing here is CI-only: every recipe runs locally the same way.
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

# Lint with the warning set CI uses.
lint:
    cargo clippy --all-targets --locked -- -D warnings

# Run every test.
test:
    cargo test --all --locked

# Run the differential tests against the original gitstatusd. The devShell sets GITSTATUSD_PATH.
differential:
    cargo test -p p11k-d --test compat -- --ignored

# Everything a pull request must pass.
check: fmt-check nix-fmt-check lint test

# The full local gate. CI also runs `just differential` and `just nix-build`.
verify: check deny spell

# Build the binaries.
build:
    cargo build --release --locked

# Run the wall-clock benchmark. Quote its numbers with the machine they came from; the machine's
# load shows up in them.
bench:
    cargo bench -p p11k-gitstatus --bench build_fields_criterion

# Run the instruction-count benchmark. Callgrind counts do not move with the machine, so this is
# what judges a change. The devShell provides valgrind and the iai-callgrind runner.
bench-iai:
    cargo bench -p p11k-gitstatus --bench build_fields_iai

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
# reference frame. The build runs in the default devShell so cargo caches apply; the capture and
# the comparison run in the visual devShell with --ignore-environment, so nothing comes from the
# caller. See tools/visual/README.md.
render-check *args:
    nix develop --command cargo build -p p11k-engine --bin p11k
    nix develop .#visual --ignore-environment --command bash tools/visual/check.sh {{args}}

# Capture new frames after a deliberate rendering change. The captures go to
# tools/visual/expected-pending. They are not references until a person reviews and accepts
# them. Pass -j/--jobs N to change the bats parallelism.
render-update *args:
    nix develop --command cargo build -p p11k-engine --bin p11k
    nix develop .#visual --ignore-environment --command bash tools/visual/check.sh --update {{args}}

# Copy the reviewed pending frames into tools/visual/expected. Run this only after a person
# checks each image against the intended layout. The frames then become the reference that CI
# compares against.
render-accept:
    bash tools/visual/accept.sh
