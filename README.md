# powerlevel11k (p11k)

[中文](README.zh.md) · [![All Contributors](https://img.shields.io/github/all-contributors/Qaaxaap/powerlevel11k?color=ee8449&style=flat-square)](#contributors) · [![CI](https://github.com/Qaaxaap/powerlevel11k/actions/workflows/ci.yml/badge.svg)](https://github.com/Qaaxaap/powerlevel11k/actions/workflows/ci.yml) · [![LGPL-3.0-or-later](https://img.shields.io/badge/license-LGPL--3.0--or--later-blue)](COPYING.LESSER)

A prompt engine that continues powerlevel10k, written in Rust.

![p11k in zsh: git status, a test run with its execution time, a background job](docs/images/demo.gif)

p11k is still in development. It is feature-complete and used daily by its developer, but not settled: the config language can still change.

powerlevel10k entered pure maintenance mode in 2024. It gets no new features, most bugs stay unfixed, issues go unanswered, and the repository will not be handed over. It still works well today, but nothing past v1.20.0 is coming. p11k takes over from there, with the look and the speed of p10k and the whole chain rewritten in Rust.

It is no longer a traditional shell theme. The engine owns the terminal and acts as a transparent proxy. It runs a theme-less shell in a pty, passes bytes through untouched, and takes over only at the moment a prompt appears. One theme can therefore run on any shell, given a small adaptation layer.

bash, fish, zsh and pwsh are supported today. Further shells are welcome.

## Try it

```bash
nix run github:Qaaxaap/powerlevel11k -- --shell zsh   # no install, no config
```

Anything else is under [Installing](#installing).

## One theme, every shell

The header is drawn by the engine, not by the shell, so one theme file gives the same prompt under zsh, bash and fish.

![the same theme in zsh, bash and fish](docs/images/shells.png)

## Presets

`--preset lean|classic|rainbow|pure` picks a starting point without writing a config. `p11k configure` walks through the same choices as p10k's wizard and writes out the KDL. All four are ordinary themes, and anything in them can be edited.

The four palettes are shown in one layout so that they can be compared. A preset also carries a layout of its own: lean and pure are single-line and unframed. Colours come from the preset, and a segment a preset does not define keeps the terminal's default.

![lean, classic, rainbow and pure](docs/images/presets.png)

## Features

- **Multi-line header**: p10k style, generated from a KDL v2 config.
- **Instant header**: the prompt is on screen as soon as the terminal opens, without waiting for the shell to finish loading its config.
- **Transient prompt**: the header folds into a single `❯` line once a command is submitted. Currently zsh only.
- **vi_mode / history / loose layout**: vi mode indicator, history command number, `prompt-add-newline` blank lines and the like, enabled per config.
- **80+ built-in segments**, matching p10k's set one for one:
  - `dir`, `vcs`, `status`, `time`, `command_execution_time`, `background_jobs`
  - language versions: go, rust, node, php, java, dotnet, swift
  - cloud: aws, azure, gcloud, kube, terraform
  - network: ip, vpn, wifi, public_ip
  - system: load, ram, swap, battery, disk
  - environment indicators: virtualenv, pyenv, rvm, nvm
- **git status**: computed on a background thread, so even a huge repository does not block the prompt.
- **KDL v2 theme language**: `layout` / `segments` / `defaults` / `separators` / `frame` / `vcs-remote-icons` / `mode` / `icon`. See [docs/config-language.md](docs/config-language.md). Visuals are not hardcoded in the engine, so swapping the config swaps the theme.
- **Three icon sets**: `nerdfont` / `compatible` / `ascii`, picked to match the font in use. A non-UTF-8 locale falls back to `ascii` automatically. A top-level `icon{}` overrides individual characters per tier.

## How the engine works

The engine is a pty host and a transparent proxy. It starts a theme-less shell in a pty, forwards bytes untouched, and takes over drawing only at the moment a prompt appears.

Once a shell hook announces that the prompt is ready, the engine draws the theme header and hands control back when it is done. The input line is still drawn by the shell's own line editor, so completion, history and vi mode stay geometrically consistent.

The geometry rests on a zero-width prompt-ready signal. The shell's prompt string carries `OSC 1337;p11k;p` after the blank rows the header needs. The engine paints the header into those rows when the signal reaches it. The prefix belongs to the shell's prompt, so the line editor draws the buffer right after it. The engine does not parse ANSI sequences out of the pty stream, and it does not touch user input.

## Installing

With Nix, from the flake:

```bash
nix profile install github:Qaaxaap/powerlevel11k
```

As a home-manager input:

```nix
inputs.p11k.url = "github:Qaaxaap/powerlevel11k";
home.packages = [ inputs.p11k.packages.${pkgs.system}.default ];
```

On Debian and Ubuntu, from the `.deb` attached to a [release](https://github.com/Qaaxaap/powerlevel11k/releases):

```bash
sudo apt install ./p11k_<version>_amd64.deb
```

On Arch Linux, from the [AUR](https://aur.archlinux.org), with any AUR helper:

```bash
yay -S powerlevel11k
# or powerlevel11k-git, which follows the latest fixes
```

Anywhere else, unpack the tarball. `bin/` and `share/` are relocatable: the catalogs are looked up in the system directory and next to the executables alike.

```bash
tar xzf p11k-<version>-x86_64-linux.tar.gz -C ~/.local
~/.local/p11k-<version>-x86_64-linux/bin/p11k --shell zsh
```

The tarball needs the system C library and the GCC runtime. Common distributions ship both by default. Each release also carries `runtime-dependencies.md` with the exact list, generated by `ldd` from the binaries.

To build and install from source:

```bash
cargo install --path crates/p11k-engine
```

This needs a Rust toolchain. `msgfmt` (gettext) generates the translations. A missing `msgfmt` still builds, and the interface simply has no translations.

All builds are x86_64 Linux.

## Wiring it into a shell

If another shell theme is in use, remove it first. p11k is not a traditional shell theme, and the two would stack on screen.

Add one line to the shell's rc file:

```bash
# zsh
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell zsh
# bash
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell bash
# fish
if not set -q P11K_ENGINE; exec /path/to/p11k --shell fish; end
```

For PowerShell, put this in `$PROFILE`:

```powershell
if (-not $env:P11K_ENGINE) { & '/path/to/p11k' --shell pwsh; exit }
```

`--shell` may be left out: `shell "zsh"` in the theme config names the shell instead. With neither, the engine falls back to `$SHELL`, which may not be the shell you expect.

The engine execs over the shell that started it and inherits its environment. The inner shell re-sources the user config, except for the theme. `P11K_ENGINE` prevents recursive startup.

Because of that, keep the p11k bootstrap line at the top of your rc file. The lines above it would otherwise be loaded twice.

## Theme configuration

The theme is a KDL v2 file. The syntax is in [docs/config-language.md](docs/config-language.md).

The engine reads `$XDG_CONFIG_HOME/p11k/p11k.kdl` and falls back to the built-in lean theme when that file does not exist. The `p11k configure` wizard writes its config there. `--config <path>` names a different file, and `--preset <name>` selects a built-in theme (lean / classic / rainbow / pure).

`p11k reload` makes a running session re-read the theme file, with no need to restart the shell:

```bash
$EDITOR ~/.config/p11k/p11k.kdl
p11k reload
```

The engine repaints the header and restarts its git worker, so `vcs` properties take effect as well. A file that fails to parse is reported by the command, and the theme in use is kept. The width of the input line is fixed when the engine starts. A `prompt_char` of a different width is ignored and recorded in the engine log. Colours, layout, segments, separators and the frame apply immediately.

## Interface language

The interface is available in Chinese and English. More translations are welcome. The language follows the standard environment variables (`LANG`, `LC_ALL`, `LANGUAGE`) and defaults to English:

```bash
LANG=zh_CN.UTF-8 p11k --shell zsh   # Chinese interface
p11k --shell zsh                    # English (default)
```

`P11K_LOCALEDIR` can point at a catalog directory. Installations through a package or the flake need no such setting.

## compat/p10k: replacing gitstatusd

The `compat/p10k` branch vendors the original p10k theme and replaces only its core. `p11k-d` answers on the gitstatusd wire protocol, byte for byte, and works as the backend of the original `gitstatus.plugin.zsh`.

```bash
export GITSTATUS_DAEMON=/path/to/p11k-d
```

Configuration, appearance, status and vcs are identical to p10k.

That branch is not the main line of development. `main` carries the engine, but the branch will still be maintained.

## Status

Done:

- gitstatusd-compatible core (v1.5.5 protocol) and the performance core: index parsing, `fstatat` scanning, parallel shards, untracked cache
- prompt-ready signal protocol: self-consistent multi-line geometry across zsh / bash / fish / pwsh
- instant / transient prompt, vi_mode, history, loose layout
- 80+ segments, the KDL theme language, three icon tiers and `icon{}` overrides
- the `p11k configure` wizard, gettext catalogs (zh_CN), CI
- `p11k reload`, and p10k feature parity up to the extended status states (`OK_PIPE` / `ERROR_PIPE` / `ERROR_SIGNAL`) and `VCS_DISABLED_WORKDIR_PATTERN`
- p10k knobs: `ICON_PADDING`, `ICON_BEFORE_CONTENT` and `TRANSIENT_PROMPT` match p10k at every setting, including `moderate`, explicit per-segment overrides and `same-dir`
- release artifacts: a Nix flake, a `.deb` and a tarball

Not done:

- `LEGACY_ICON_SPACING` will not be implemented. It reproduces a powerlevel9k layout bug.
- `DIR_MAX_LENGTH` and `DIR_MIN_COMMAND_COLUMNS(_PCT)` cannot be implemented. They need to read zle's buffer, which the engine never sees.
- macOS support
- long-term maintenance and hardening of the daemon and the engine

## Contributing

Build and test environment, commit conventions, the translation workflow and the release process are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributors

<!-- ALL-CONTRIBUTORS-LIST:START - Do not remove or modify this section -->
<!-- prettier-ignore-start -->
<!-- markdownlint-disable -->
<table>
  <tbody>
    <tr>
      <td align="center" valign="top" width="14.28%"><a href="https://github.com/Qaaxaap"><img src="https://avatars.githubusercontent.com/u/175310223?v=4&size=100" width="100px" height="100px" alt="Qaaxaap"/><br /><sub><b>Qaaxaap</b></sub></a><br /><a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=Qaaxaap" title="Code">💻</a> <a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=Qaaxaap" title="Documentation">📖</a> <a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=Qaaxaap" title="Tests">⚠️</a> <a href="#infra-Qaaxaap" title="Infrastructure (Hosting, Build-Tools, etc)">🚇</a> <a href="#translation-Qaaxaap" title="Translation">🌍</a> <a href="https://github.com/Qaaxaap/powerlevel11k/pulls?q=is%3Apr+reviewed-by%3AQaaxaap" title="Reviewed Pull Requests">👀</a></td>
      <td align="center" valign="top" width="14.28%"><a href="http://revitalize.top"><img src="https://avatars.githubusercontent.com/u/247487531?v=4&size=100" width="100px" height="100px" alt="Cwhirly"/><br /><sub><b>Cwhirly</b></sub></a><br /><a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=Cwhirly" title="Code">💻</a> <a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=Cwhirly" title="Documentation">📖</a></td>
      <td align="center" valign="top" width="14.28%"><a href="https://github.com/RafaelJust"><img src="https://avatars.githubusercontent.com/u/74719647?v=4&size=100" width="100px" height="100px" alt="RafaelJust"/><br /><sub><b>RafaelJust</b></sub></a><br /><a href="https://github.com/Qaaxaap/powerlevel11k/commits?author=RafaelJust" title="Documentation">📖</a> <a href="https://github.com/Qaaxaap/powerlevel11k/issues?q=author%3ARafaelJust" title="Bug reports">🐛</a> <a href="#platform-RafaelJust" title="Packaging/porting to new platform">📦</a></td>
    </tr>
  </tbody>
</table>

<!-- markdownlint-restore -->
<!-- prettier-ignore-end -->

<!-- ALL-CONTRIBUTORS-LIST:END -->

## License

LGPL-3.0-or-later: [COPYING.LESSER](COPYING.LESSER) is the license itself, [COPYING](COPYING) is the GPL-3.0 it extends.

The powerlevel10k theme vendored on the `compat/p10k` branch is MIT (c) Roman Perepelitsa and contributors. The original copyright notice is kept in `compat/p10k/vendor/powerlevel10k/LICENSE`.
