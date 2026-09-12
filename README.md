# powerlevel11k (p11k)

A prompt that keeps powerlevel10k alive, with Rust.

The architecture is naturally cross-shell. bash/fish/pwsh/zsh are currently
supported.

powerlevel10k moved to maintenance-only mode in 2024: no new features, most
bugs will not be fixed, help requests are ignored, and the repository will not
be handed over. It still works today, but anyone who wants more than what
ships in v1.20.0 has nowhere to go. p11k picks up from there — same look and
feel, but the whole chain is Rust.

Two main branches live in this repository:

- **`main`: a full theme engine.** It renders the prompt itself and depends
  on no shell theme; it draws the prompt for any shell in a simple,
  controllable way.
- **`compat/p10k`: a drop-in p10k backend.** It vendors the original p10k
  theme and replaces only the kernel — `gitstatusd` → the Rust daemon
  `p11k-d`. Config, look, vcs stay identical.

## The p11k engine

The engine is a pty host and transparent proxy: it runs a theme-less shell
inside a pty and passes bytes through untouched, taking over only for a
moment at prompt time.

Once a shell hook announces the prompt, the engine draws the themed header,
then hands back. The shell's own line editor draws the input line, so
completion, history and vi-mode geometry stay self-consistent.

Geometry is guaranteed by a **placeholder protocol**: the shell renders a
theme made of meaningless placeholders, and the engine fills the real header
in only after the placeholder has rendered. The engine never parses ANSI in
the pty output and never touches what you type.

### What it can do

If one sentence is needed to describe what this project is worth, it would
be:

> It brings powerlevel10k to bash/fish and to any shell.

- Multi-line themed header: p10k-style, generated from KDL v2 config; config
  files are human-friendly.
- **Instant header**: open a terminal and the prompt is there, no waiting for
  a slow shell config to load.
- **Transient prompt** (zsh): the header folds to a single-line `❯` the moment
  a command is submitted (p10k `TRANSIENT_PROMPT`); bash/fish/pwsh have no zle,
  so it does nothing there.
- **vi_mode / history / loose layout**: vi mode indicator, history number,
  `prompt-add-newline` blank line, on/off per config.
- **80+ built-in segments**: dir, vcs, status, time, command_execution_time,
  background jobs, language versions (go/rust/node/php/java/dotnet/swift/…),
  cloud (aws/azure/gcloud/kube/terraform), network (ip/vpn/wifi/public_ip),
  system (load/ram/swap/battery/disk), environment indicators
  (virtualenv/pyenv/rvm/nvm/…) — almost one-to-one with p10k's segments.
- **Git status**: a Rust gitstatus core (index parsing, `fstatat` scanning,
  parallel shards, untracked cache) runs on a background thread — a
  nixpkgs-scale repo does not stall the prompt.
- Themes are **KDL v2** config
  ([docs/config-language.md](docs/config-language.md)): `layout` / `segments`
  / `defaults` / `separators` / `frame` / `vcs-remote-icons` / `mode` / `icon`.
  Swap the config file, swap the theme; the engine hard-codes no visuals.
- **Icon/font mode** offers three icon sets — `nerdfont` / `compatible` /
  `ascii` — pick the one that matches your actual font. On a non-UTF-8 locale
  (console / C locale) it auto-falls-back to `ascii` rather than render every
  non-ASCII char as a box. A top-level `icon{}` table overrides characters per
  **icon name**.

### Quick start

**Remove your existing shell theme first, then load p11k.**

p11k is not a traditional shell theme; without removing the previous theme
the two would stack on screen.

```bash
cargo build -p p11k-engine

# dev: start the engine directly (point P11K_USER_ZSHRC at a copy of your rc
# with the theme removed).
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/pwsh/zsh>

# with a theme file (defaults to the built-in lean theme).
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

As a "theme": add one exec line to the shell's rc:

```bash
# zsh
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell zsh
# bash
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell bash
# fish
if not set -q P11K_ENGINE; exec /path/to/p11k --shell fish; end
```

For PowerShell put this in `$PROFILE` (pwsh has no `exec`, so the outer shell
exits once the engine does):

```powershell
if (-not $env:P11K_ENGINE) { & '/path/to/p11k' --shell pwsh; exit }
```

Pass `--shell` explicitly: `$SHELL` does not change when pwsh is started from
zsh or bash, and the engine reads nothing else to recognise it.

### Installing

With Nix, from the flake:

```bash
nix run github:Qaaxaap/powerlevel11k -- --shell zsh   # try it without installing
nix profile install github:Qaaxaap/powerlevel11k
```

As a home-manager input:

```nix
inputs.p11k.url = "github:Qaaxaap/powerlevel11k";
# then in a module:
home.packages = [ inputs.p11k.packages.${pkgs.system}.default ];
```

On Debian and Ubuntu, from the `.deb` attached to a
[release](https://github.com/Qaaxaap/powerlevel11k/releases):

```bash
sudo apt install ./p11k_0.1.0_amd64.deb
```

Anywhere else, unpack the tarball. `bin/` and `share/` are relocatable — the
catalogs are looked up next to the executables as well as in the system
directory — so it runs from wherever it lands:

```bash
tar xzf p11k-0.1.0-x86_64-linux.tar.gz -C ~/.local
~/.local/p11k-0.1.0-x86_64-linux/bin/p11k --shell zsh
```

Without Nix, from a checkout:

```bash
cargo install --path crates/p11k-engine
```

The build wants `msgfmt` (gettext) for the translations; the run time wants
whichever shell gets proxied, and the tarball and `.deb` builds expect
`libssl3`. The package carries `p11k` and `p11k-d`, the standalone gitstatusd
replacement; `p11k` does not need the daemon. Linux only so far: the git index
scan still assumes Linux `stat` semantics.

### Reloading the theme

`p11k reload` re-reads the theme file in an already running session:

```bash
$EDITOR ~/.config/p11k/p11k.kdl
p11k reload
```

The engine repaints the header and restarts its git worker, so the `vcs`
properties take effect as well. A file that no longer parses is reported by the
command and the theme in use is kept. The width of the input line is fixed when
the engine starts, so a `prompt_char` of a different width is ignored (the
engine log says so); everything else — colours, layout, segments, separators,
frame — applies right away.

### Development environment

Everything the build and the tests need is pinned in `flake.nix`:

```bash
nix develop        # rust (cargo/clippy/rustfmt) + msgfmt + zsh + zh_CN.UTF-8
cargo test --all
nix build          # the binaries: p11k and p11k-d
```

The devShell also exports `LOCALE_ARCHIVE`, otherwise `setlocale("zh_CN.UTF-8")`
fails and the i18n translation test has to skip — and a skipped test is not a
test. CI runs exactly these `nix develop` commands, so there is no separate CI
environment to keep in sync.

The engine execs over the launching shell and inherits its environment; the
inner shell re-sources the user config (except the theme — the engine is the
theme). `P11K_ENGINE` breaks recursion.

### Language

User-facing text of the engine and of `p11k configure` goes through gettext.
English is the source language: message ids in the code are English and are
printed verbatim when no translation exists. Translations live in
`crates/p11k-engine/po/<lang>.po` and are compiled to `.mo` by `build.rs` with
`msgfmt` (skipped with a warning when `msgfmt` is missing — the build still
succeeds, untranslated). The locale is picked up from the standard environment
(`LANG`, `LC_ALL`, `LANGUAGE`):

```bash
LANG=zh_CN.UTF-8 target/debug/p11k --shell zsh   # Chinese UI
target/debug/p11k --shell zsh                    # English (default)
```

`P11K_LOCALEDIR` overrides the translation directory (defaults to the one baked
in at build time), e.g. `P11K_LOCALEDIR=/usr/share/locale` for a system install.
Adding a language is one file plus a rebuild — copy `po/zh_CN.po` to
`po/<lang>.po`, translate the `msgstr`s, `cargo build -p p11k-engine`. Catalogs
are kept in sync with the code by `tools/i18n.sh extract|update`, and a unit
test fails when the sources, `po/p11k.pot` and the catalogs disagree.

## compat/p10k: a drop-in gitstatusd

p10k's zsh rendering is a decade of polish and should not be rewritten. The
`compat/p10k` branch vendors the original p10k theme and replaces only the
kernel: `p11k-d` (a Rust daemon) answers the gitstatusd wire protocol
byte-for-byte, backing the original `gitstatus.plugin.zsh`:

```zsh
export GITSTATUS_DAEMON=/path/to/p11k-d
```

Config, look, status, vcs stay identical to p10k. Hot git requests on a
nixpkgs-scale repo (54k files / 38k dirs, 22 cores) take ~100ms vs ~65ms for
the C++ original (was ~320ms before the parallel-scan fix).

## Status & roadmap

Done:

- gitstatusd-compatible core (v1.5.5 protocol) and the performance core:
  index parsing, `fstatat` scanning, parallel shards, untracked cache
- engine placeholder protocol: self-consistent multi-line geometry across
  zsh / bash / fish / pwsh
- instant / transient prompt, vi_mode, history, loose layout
- 80+ segments and the KDL theme language, three-mode icons + `icon{}`
  overrides
- `p11k configure` wizard, gettext catalogues (zh_CN), CI
- `p11k reload`, and the p10k feature set up to extended status states
  (`OK_PIPE` / `ERROR_PIPE` / `ERROR_SIGNAL`) and `VCS_DISABLED_WORKDIR_PATTERN`
- Nix flake package: `nix run` / `nix profile install` / home-manager input,
  with the catalogs installed and the version read from `Cargo.toml`

Roadmap:

- the p10k knobs that are still missing, all of them non-default settings:
  `ICON_PADDING=moderate`, explicit `ICON_BEFORE_CONTENT` overrides,
  `TRANSIENT_PROMPT=same-dir` (`transient-prompt #true` is p10k's `always`) and
  `LEGACY_ICON_SPACING`. Rendering under p10k's defaults already matches.
  `DIR_MAX_LENGTH` and `DIR_MIN_COMMAND_COLUMNS(_PCT)` cannot be implemented:
  they need zle's buffer, which the engine never sees.
- macOS support
- long-term maintenance and hardening of the daemon and engine (the point of
  p11k)

## License

LGPL-3.0-or-later. See [LICENSE](LICENSE).

The powerlevel10k theme vendored on the `compat/p10k` branch is MIT (c) 2019
Roman Perepelitsa and contributors; the original copyright notice is kept
verbatim at `compat/p10k/vendor/powerlevel10k/LICENSE`.
