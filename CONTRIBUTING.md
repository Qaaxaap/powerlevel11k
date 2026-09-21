# Contributing to powerlevel11k

[中文](CONTRIBUTING.zh.md)

Thanks for your interest.

The project is in early development. These rules are deliberately small and
will grow with the codebase.

## Development environment

Everything the build and the tests need is pinned in `flake.nix`:

```bash
nix develop        # rust (cargo/clippy/rustfmt) + msgfmt + zsh + zh_CN.UTF-8
cargo test --all
nix build          # the binaries: p11k and p11k-d
```

The devShell exports `LOCALE_ARCHIVE`; without it `setlocale("zh_CN.UTF-8")`
fails and the i18n test has to skip. CI runs exactly these `nix develop`
commands, so there is no second environment to keep in sync — plus one check
that the nix package still builds and carries its catalogs.

To run the engine straight from a build:

```bash
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/pwsh/zsh>
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

## Commit conventions

- [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
  (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`, `perf:`, ...).
  A `!` marks breaking changes.
- Commit messages in English.
- One commit carries one logical change; unrelated work must not be bundled.
  A commit mixing a refactor with a feature will be asked to be split.
- The subject is at most 72 characters and completes the sentence
  "if applied, this commit will ...". The body states *why*, and does not
  restate the diff.

## Code style

- Run `cargo fmt --all` before every commit; CI checks the formatting.
- `cargo clippy --all-targets --locked -- -D warnings` and
  `cargo test --all --locked` must pass.
- `unsafe` is not permitted unless a comment justifies it and a test or
  benchmark backs it up. The git status core may need it for direct
  `fstatat`/`MetadataExt` access; that is a sanctioned exception, not a
  precedent for the rest of the crate.
- Public API requires doc comments. The gitstatus protocol code must document
  the wire format next to the parsing code: byte-level compatibility with the
  original daemon is a hard requirement.

## Config language

The theme config language has its own spec, maintained in step with the
engine: [docs/config-language.md](docs/config-language.md). Changing config
parsing or rendering must follow it.

## Demo images

`docs/images/` is generated. `tools/demo/capture.sh` builds a small git
repository with a staged, an unstaged, an untracked and a stashed change, runs
the engine in a pty of a fixed size, and keeps what the terminal would have
shown; the still images come from tmux pane dumps, because fish asks the
terminal questions at startup that a bare pty does not answer. The keystrokes
are in `tools/demo/scenes/demo.json`, the theme in `tools/demo/demo.kdl`, and
`tools/demo/presets/` holds the files behind the presets image.

Re-recording needs a built `p11k` (`cargo build --release -p p11k-engine`), zsh,
bash, fish, python3, tmux, agg (asciinema/agg), ImageMagick and the Maple Mono
Nerd Font. None of them are in the devShell: the images are committed, and
nothing in CI regenerates them.

## Translations

User-facing text uses gettext: English msgids in the code, catalogs in
`crates/p11k-engine/po/`. When adding, removing or rewording user-facing text:

```bash
tools/i18n.sh extract   # xgettext -> po/p11k.pot
tools/i18n.sh update    # msgmerge the new entries into every po/*.po
```

then fill in the new `msgstr`s. Three rules are enforced by the test suite:

- Text translated where it is written uses `t("…")`; text collected and
  translated later (wizard question titles and options) must be wrapped in
  `msgid("…")` so the extractor sees it. Both must be plain string literals.
- `cargo test -p p11k-engine` fails if the sources, `po/p11k.pot` and the
  catalogs disagree in either direction. A missing extract/update or a stale
  entry cannot pass.
- Adding a language: copy `po/zh_CN.po` to `po/<lang>.po`, translate the
  `msgstr`s and rebuild. The nix package compiles everything under `po/`, so
  `flake.nix` needs no change.

## AI policy

This repository embraces AI, on the following terms. Ignoring them may result
in restrictions on, or a ban from, contributing to this repository.

- Contributors must understand, and take responsibility for, every line they
  submit. Pointless or harmful submissions are not allowed.
- Understanding the code is not enough: developing with generative AI without
  supervision is discouraged. Submissions with obvious defects, or that do not
  match what they claim to do, will be rejected.

## Releases

A `v*` tag publishes: the workflow builds `p11k` and `p11k-d` and attaches a
tarball and a `.deb` to that tag's GitHub release.

The tag must match `[workspace.package] version` in `Cargo.toml`; the workflow
checks this and fails otherwise.

## Reporting issues

Reports must be specific. A good report contains: OS and shell versions
(`zsh --version`), the p11k version, the terminal emulator, and the smallest
config that reproduces the problem. Configuration questions will be redirected
to Discussions.

## Pull requests

1. Fork, branch, and commit per the rules above.
2. One PR carries one change. State the motivation in the description.
3. Test the patch locally at least once before submitting. A patch that does
   not compile or cannot run wastes review time; repeated low-quality
   submissions may lead to contribution restrictions.
4. Follow the [AI policy](#ai-policy) above.
5. Submitting a PR licenses the contribution under LGPL-3.0-or-later.

## License

Everything in this repository, contributions included, is LGPL-3.0-or-later.
