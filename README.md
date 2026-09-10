# powerlevel11k (p11k)

A prompt that keeps powerlevel10k alive, with Rust.

The architecture is naturally cross-shell. bash/fish/zsh are currently
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
  a command is submitted (p10k `TRANSIENT_PROMPT`); bash/fish have no zle, so
  it does nothing there.
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
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/zsh>

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
`po/<lang>.po`, translate the `msgstr`s, `cargo build -p p11k-engine`. A unit
test keeps `po/` honest: every msgid in a catalog must still appear in the
sources.

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
  zsh / bash / fish
- instant / transient prompt, vi_mode, history, loose layout
- 80+ segments and the KDL theme language, three-mode icons + `icon{}`
  overrides

Roadmap:

- a `p10k configure`-style wizard
- remaining segments and framework integrations (oh-my-zsh, prezto, zinit)
- long-term maintenance and hardening of the daemon and engine (the point of
  p11k)

## License

LGPL-3.0-or-later. See [LICENSE](LICENSE).

The powerlevel10k theme vendored on the `compat/p10k` branch is MIT (c) 2019
Roman Perepelitsa and contributors; the original copyright notice is kept
verbatim at `compat/p10k/vendor/powerlevel10k/LICENSE`.
