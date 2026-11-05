# Contributing to powerlevel11k

[中文](CONTRIBUTING.zh.md)

Thanks for your interest.

The project is in early development. These rules are deliberately small and will grow with the codebase.

## Development environment

Everything the build and the tests need is pinned in `flake.nix`. The demo recorder needs extra tools, listed under [Demo images](#demo-images). Enter the devShell and use the recipes in `justfile`. `just` on its own lists all of them.

```bash
nix develop
just check         # fmt-check + clippy + test: the fast gate
just nix-check     # the same tests inside the nix sandbox, debug build
just build         # the binaries: p11k and p11k-d
```

`just verify` is what CI runs on x86_64 and aarch64. The `nix` job runs `just nix-build` and checks the package. `just differential` runs the compatibility tests on both architectures. `just check` is the faster gate without `deny` and `spell`. The MSRV job runs `cargo check --all-targets --locked`. The other recipes cover the rest of the infrastructure:

- `just deny`: dependency licences, bans and advisories
- `just spell`: typos
- `just i18n`: regenerate the gettext template
- `just changelog`: regenerate `CHANGELOG.md` from the commit history
- `just demo`: re-record `docs/images/`

The devShell exports `LOCALE_ARCHIVE`. Without it, `setlocale("zh_CN.UTF-8")` fails and the i18n test has to skip.

To run the engine straight from a build:

```bash
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/pwsh/zsh>
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

## Benchmarks

Two harnesses measure `p11k-gitstatus::RepoCache::build_fields`, the call a prompt makes on every precmd:

```bash
just bench       # wall-clock milliseconds, criterion
just bench-iai   # instruction counts, Callgrind
```

`just bench` prints milliseconds. Quote the machine with any number you take from it: load, frequency and cache pressure all show up in wall-clock time.

`just bench-iai` prints instruction counts. The counts repeat between runs of the same binary. The two 50-file cases repeat exactly, and the 5000-file case stays within 0.03%, because `readdir` order depends on the file system. Judge a change by these counts, and treat differences below 0.1% as noise.

Both harnesses build their fixture in the temporary directory, so they need no environment variable. They also need no repository outside this one. CI compiles them through `cargo clippy --all-targets` but does not run them: benchmark timing belongs on a quiet machine.

The devShell provides Valgrind and `iai-callgrind-runner`. Outside Nix, install them yourself:

```bash
cargo install iai-callgrind-runner --version 0.16.1   # must match iai-callgrind in Cargo.toml
# plus your distribution's valgrind package
```

## Commit conventions

- [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`, `perf:`, ...). A `!` marks breaking changes.
- Commit messages in English.
- One commit carries one logical change. Unrelated work must not be bundled. A commit that mixes a refactor with a feature will be asked to be split.
- The subject is at most 72 characters and completes the sentence "if applied, this commit will ...". The body states *why* and does not restate the diff.

## Code style

- Run `just fmt` before every commit. `just check` verifies the formatting along with the lint and the tests.
- `unsafe` is not permitted unless a comment justifies it and a test or benchmark backs it up. The git status core may need it for direct `fstatat`/`MetadataExt` access. That is a sanctioned exception, not a precedent for the rest of the crate.
- Public API requires doc comments. The gitstatus protocol code must document the wire format next to the parsing code. Byte-level compatibility with the original daemon is a hard requirement.

## Config language

The theme config language has its own spec, maintained in step with the engine: [docs/config-language.md](docs/config-language.md). Changing config parsing or rendering must follow it.

## Demo images

`docs/images/` is generated. `tools/demo/capture.sh` builds a small git repository with a staged, an unstaged, an untracked and a stashed change. It then runs the engine in a pty of a fixed size and keeps what the terminal would have shown.

The still images come from tmux pane dumps, because fish asks the terminal questions at startup that a bare pty does not answer.

The keystrokes are in `tools/demo/scenes/demo.json`, the theme in `tools/demo/demo.kdl`, and `tools/demo/presets/` holds the files behind the presets image.

Re-recording needs a built `p11k` (`cargo build --release -p p11k-engine`) and the devShell for zsh, bash, fish, python3 and tmux. `just demo` pulls agg, ImageMagick and fontconfig through `nix shell`. The images use the Maple Mono Nerd Font. Set `FONT_FILE` to another font file when `fc-match` does not find it.

After re-recording, check four things. The font is the intended one. No glyph is missing. The zsh, bash and fish panes all look right. The layout holds at more than one width.

## Translations

English is the working language of this repository: code, comments, commit messages and documentation are written in English. The maintainer keeps the Chinese version in sync. Terminology follows [docs/glossary.md](docs/glossary.md).

The author is not a native English speaker, so the English text may contain translation errors. When the English text disagrees with the code's behaviour, the code wins. Point out the mismatch in a pull request or issue.

User-facing text uses gettext: English msgids in the code, catalogs in `crates/p11k-engine/po/`. When adding, removing or rewording user-facing text:

```bash
tools/i18n.sh extract   # xgettext -> po/p11k.pot
tools/i18n.sh update    # msgmerge the new entries into every po/*.po
```

Then fill in the new `msgstr`s. Three rules are enforced by the test suite:

- Text translated where it is written uses `t("…")`. Text collected and translated later (wizard question titles and options) must be wrapped in `msgid("…")` so the extractor sees it. Both must be plain string literals.
- `cargo test -p p11k-engine` fails if the sources, `po/p11k.pot` and the catalogs disagree in either direction. A missing extract or update, or a stale entry, cannot pass.
- Adding a language: copy `po/zh_CN.po` to `po/<lang>.po`, translate the `msgstr`s and rebuild. The nix package compiles everything under `po/`, so `flake.nix` needs no change.

## AI policy

This repository accepts AI-assisted development, on the following terms. Ignoring them may result in restrictions on, or a ban from, contributing to this repository.

- Contributors must understand, and take responsibility for, every line they submit. Pointless or harmful submissions are not allowed.
- Understanding the code is not enough. Developing with generative AI without supervision is discouraged. Submissions with obvious defects, or that do not match what they claim to do, will be rejected.
- A submitter using AI assistance is responsible for verifying the result. Do not submit code you have not run, or cannot explain how to verify.

## Releases

A `v*` tag publishes. The workflow builds `p11k` and `p11k-d`, and attaches a tarball and a `.deb` to that tag's GitHub release.

Before tagging, run `just changelog v<version>` and commit the result. `CHANGELOG.md` is generated from the Conventional Commit history. Without the tag argument the entries stay under `Unreleased`.

The tag must match `[workspace.package] version` in `Cargo.toml`. The workflow checks this and fails otherwise.

## Reporting issues

Reports must be specific. A good report contains: OS and shell versions (`zsh --version`), the p11k version, the terminal emulator, and the smallest config that reproduces the problem. Configuration questions will be redirected to Discussions.

## Pull requests

1. Fork, branch, and commit per the rules above.
2. One PR carries one change. State the motivation in the description.
3. Test the patch locally at least once before submitting. A patch that does not compile or cannot run wastes review time. Repeated low-quality submissions may lead to contribution restrictions.
4. Put the evidence in the description: the commands you ran and a summary of their output. If a check could not run, say why and write down how the maintainer can run it locally.
5. Describe the change and its commits in terms of the repository: project commands, repository files and public links. Do not mention drafts in `.cache/`, local scripts or plans outside the repository.
6. A change to `docs/images/` or to its generator must pass the generator's prechecks, and a human review of the result. Record the review in the description.
7. The maintainer may close a pull request without review. This applies when the change does not meet these requirements, or does not match what it claims to do.
8. Follow the [AI policy](#ai-policy) above.
9. Submitting a PR licenses the contribution under LGPL-3.0-or-later.

## License

Everything in this repository, contributions included, is LGPL-3.0-or-later.
