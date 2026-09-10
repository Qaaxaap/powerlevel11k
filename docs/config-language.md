# Config language

Themes are written in KDL v2 and passed via `--config <file>`. The rules
below govern any change to config parsing or rendering, and double as a
config reference.

## Syntax

- Parsed per the KDL v2 spec. Where v1 and v2 differ — e.g. `true` vs
  `#true` — v2 wins.
- Unicode escapes use `\u{XXXX}` (braced).
- Attribute and node names follow kebab-case: `shorten-dir-length`,
  `right-start`, `vcs-remote-icons`.
- Color keys are the short `fg` / `bg`, to keep configs easy to write.
- Color values: `0`–`255` (256-color), `#rrggbb` (truecolor), `default`
  (terminal default), or a standard name (`black` … `brightwhite`, the 16
  basic colors).
- Example configs use `//` comments unless there is a reason not to.

## Structure

Top-level nodes today: `layout` (required), `segments`, `defaults`,
`separators`, `frame`, `vcs-remote-icons`, `mode`, `icon`. Unrecognized
top-level nodes are ignored and skipped.

## Icon mode

- `mode "nerdfont-complete"` / `mode "nerdfont-fontconfig"` /
  `mode "compatible"` / `mode "ascii"` pick the character set for built-in
  segment icons. When `mode` is absent the default follows the locale:
  UTF-8 (normal terminals) → `nerdfont-complete`; non-UTF-8 (console / C
  locale) → `ascii`.
- `nerdfont-complete` and `nerdfont-fontconfig` share one Nerd Font glyph
  set — the split only mirrors p10k's naming — so there are three icon
  sets in practice. `compatible` uses standard Unicode + Powerline glyphs;
  `ascii` uses plain text.
- Top-level `icon {}` overrides a named icon's character per mode. Icon
  names follow the glyph's meaning (`folder`, `go`, `ok`, `error`,
  `branch`…); a name shared by several segments overrides all of them at
  once (`python` covers `virtualenv`/`anaconda`/`pyenv`):

  ```kdl
  icon {
      ok     { all "V" }              // ASCII → applies to all three modes
      error  { all "X" }
      folder { nf "\u{f07c}" compat "" ascii "" } // each mode set independently
  }
  ```

  - `all` auto-detects the glyph class: Nerd Font private-use chars fall
    on `nf` only; standard Unicode (e.g. ✔) on `nf` + `compat`; pure
    ASCII on all three modes.
  - `nf` / `compat` / `ascii` set that mode's glyph (empty string =
    explicitly no icon); an omitted field falls back to the engine
    default table (which follows `mode`).
- Engine defaults come from p10k. User `icon {}` / `separators` / `frame`
  win over the mode defaults.

The key of `icon {}` is an icon name. Most segments reference an icon name
identical to the segment name (`time`, `date`, `aws`… — use the segment
name as the key); where the icon name differs from the segment name, or
several segments share one icon name, see the table below (one override
applies to every referencing segment):

| icon name | referencing segments |
|---|---|
| `folder` | `dir` |
| `git` | `vcs` (when no remote) |
| `background-jobs` | `background_jobs` |
| `go` | `go_version`, `goenv` |
| `rust` | `rust_version` |
| `node` | `node_version`, `nodeenv`, `nodenv`, `nvm` |
| `php` | `php_version`, `phpenv` |
| `java` | `java_version`, `jenv` |
| `dotnet` | `dotnet_version` |
| `terraform` | `terraform_version`, `terraform` |
| `cpu-arch` | `cpu_arch` |
| `python` | `virtualenv`, `anaconda`, `pyenv` |
| `ruby` | `rbenv`, `chruby`, `rvm` |
| `lua` | `luaenv` |
| `perl` | `plenv`, `perlbrew` |
| `scala` | `scalaenv` |
| `disk` | `disk_usage` |
| `gcloud` | `gcloud`, `google_app_cred` |
| `kube` | `kubecontext` |
| `network` | `ip` |
| `vpn` | `vpn_ip` |
| `public-ip` | `public_ip` |
| `lock` | `dir_writable` |
| `history` | `per_directory_history` |
| `haskell` | `haskell_stack` |
| `flutter` | `fvm` |
| `aws-eb` | `aws_eb_env` |
| `laravel` | `laravel_version` |
| `test` | `rspec_stats` |
| `server` | `docker_machine` |
| `nix` | `nix_shell` |
| `mc` | `midnight_commander` |
| `vim` | `vim_shell` |
| `chezmoi` | `chezmoi_shell` |
| `ok` / `error` | `status`'s OK / ERROR states |
| `branch` | the glyph before the `vcs` branch |

The `os` badge is dynamic per distro (arch/ubuntu/…) and is not routed
through `icon {}`.

## Layout

- `layout { left { … } right { … } }`. Under `left` / `right`, each row
  is declared as a `line { … }` node.
- Segments inside a line render in declaration order.
- Enablement is a flag: writing the segment name enables it. An explicit
  `#false` is allowed (to comment a segment out temporarily); `#true`
  works but is discouraged.
- A line may also contain `text "…"` static text, styled with the `text`
  segment. `$VAR` / `${VAR}` inside expand to environment variables.
- For an empty row, declare `line {}`.
- `layout { prompt-add-newline <N> … }` inserts N blank lines between
  consecutive prompts (p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE` plus
  `_COUNT`, "loose" layout). `#true` means 1; `0` turns it off.
- `layout { transient-prompt #true … }` folds the multi-line header down to
  a single-line `❯` the moment a command is submitted (p10k
  `TRANSIENT_PROMPT`). zsh-only: it depends on `zle reset-prompt`, so
  bash/fish ignore it.

## Segments

- Each child node of `segments { <name> fg=… … }` styles one segment;
  the node name is the segment name. Segment names are engine-defined and
  snake_case to match the implementation (`command_execution_time`,
  `background_jobs`, `prompt_char`). Referencing a name the engine does
  not implement does not error — that segment is silently skipped.
  Adding a segment means extending the engine, or using `text` instead.
- Style attributes: `fg`, `bg`, `bold`.
- `state <NAME> fg=…` is a situational override: style, plus an optional
  `char` for glyph segments (e.g. `state ERROR char="✘"`); fallback
  chain below.
- `text-left` / `text-middle` / `text-right` are attach slots, each a
  child node `text-left "…"` with an optional `fg=…`. They concatenate
  text into the segment — `left` before the icon, `middle` between icon
  and content (rendered only when the segment has both), `right` after
  the content — and never form their own block. Color follows the segment
  unless `fg` overrides the foreground (bg/bold still follow).
- Icons are not a segment attribute. Each segment references an **icon
  name** (`dir` → `folder`, `vcs` → `git`, `time` → `time`…), rendered
  from the engine default table for the current `mode` and overridable
  per icon name in the top-level `icon {}` (see
  [Icon mode](#icon-mode)). Defaults by segment: `dir` folder, `time`
  clock, `background_jobs` gear (shown even at zero jobs), `os` distro
  badge (dynamic per distro), `vcs` git (or per remote domain, see
  `vcs-remote-icons`); language / cloud / system segments map to their
  own icon name (go, python, aws…), the rest have none.
- `content="…"` is reserved.
- `disabled #true` keeps the segment parsed and styled but skips
  rendering; it affects every place the segment is used.
- Every other attribute goes into the segment's behavior table, read by
  that segment's renderer. Implemented so far:
  - `dir`: `shorten-strategy` (see [Dir shortening](#dir-shortening)),
    `shorten-dir-length` (levels to keep / chars per level, 1–20, default
    1), `shorten-delimiter` (ellipsis, default `…`;
    `truncate_to_unique` never emits it), `shorten-folder-marker` (marker
    file name, default built-in list), `home-abbreviation` (home prefix,
    default `~`), `path-separator-foreground` (color of `/`).
  - `vcs`: `clean-foreground` / `modified-foreground` /
    `untracked-foreground` (branch plus ahead/behind/stash, change
    counts, untracked files); `show-changeset` and
    `changeset-hash-length` (default 8; detached HEAD shows the commit
    automatically); `shorten-length` / `shorten-min-length` /
    `shorten-strategy` / `shorten-delimiter` (branch shortening, needs
    both lengths); `staged-symbol` / `unstaged-symbol` /
    `conflicted-symbol` / `untracked-symbol` / `ahead-symbol` /
    `behind-symbol` / `stash-symbol` (count glyphs, default
    `+ ~ ! ? ↑ ↓ ≡`).
  - `status`: `ok-foreground` / `error-foreground`, `verbose` (`#false`
    hides success).
  - `command_execution_time`: `threshold-seconds` (default 3), `precision`
    (decimals, default 2), `format="H:M:S"`.
  - `time`: `time-format="12h"`.
  - `vi_mode`: `insert` / `normal` / `visual` / `overwrite` (default
    `INSERT` / `NORMAL` / `VISUAL` / `OVERWRITE`).
  - `date`: `date-format` (strftime, default `%d.%m.%y`).
  - any segment: `visual-identifier-color` overrides the segment's **icon**
    foreground (segment color by default).
- Style fallback has three levels: `state <NAME>` on the segment →
  segment defaults → top-level `defaults`. `dir` uses states `ANCHOR`
  (anchor path, e.g. `~`) and `SHORTENED` (collapsed components);
  besides `ERROR`, `prompt_char` also honors `VIINS` / `VICMD` / `VIVIS` /
  `VIOWR` (with the matching `char`, the prompt glyph follows the zsh
  editing mode).

## Dir shortening

`dir`'s `shorten-strategy` mirrors p10k's `POWERLEVEL9K_SHORTEN_STRATEGY`:

- `truncate_to_unique` (default): each component shrinks to the shortest
  prefix unique among its siblings; no ellipsis.
- `truncate_middle`: each component keeps the first `shorten-dir-length`
  chars, an ellipsis, then the same number of chars from the end.
- `truncate_from_right`: each component keeps the first
  `shorten-dir-length` chars plus an ellipsis.
- `truncate_to_last`: keep only the last `shorten-dir-length` levels.
- `truncate_to_first_and_last`: keep `shorten-dir-length` levels at each
  end, elide the middle.
- `truncate_absolute` / `truncate_absolute_chars`: cut the whole path to a
  character count.
- `truncate_with_folder_marker`: fold at the marker file.

What the built-in segments render:

- `dir` — current directory, collapsed from the front (`~` under $HOME),
  components colored per state.
- `vcs` — in a git repo: branch plus counts (`+staged ~unstaged
  !conflicted ?untracked ↑ahead ↓behind ≡stashes`).
- `status` — last exit code: the glyph of icon names `ok` / `error` (per
  `mode`, e.g. ascii shows `ok` / `err N`), with `N` appended on error.
- `time` — current time HH:MM:SS.
- `command_execution_time` — how long the last command ran, shown only
  at or above `threshold-seconds` (`3s`, `1m5s`, `1h2m3s`).
- `background_jobs` — number of background jobs.
- `prompt_char` — not drawn in the header; it renders the input-line
  glyph (drawn by the shell, right after `frame.last-prefix`). The glyph
  is the `char` attribute (default `❯`), any literal string — no
  environment expansion. `state ERROR` (exit code non-zero) may override
  glyph and style. All states' glyphs must share one width (prompt
  geometry is fixed at startup); on mismatch the engine reports the
  error and drops into a clean shell instead of theming.
- `os` — distro badge.
- `context` — `user@host` in SSH, `user` as local root, hidden otherwise.
- `user` — current username.
- `host` — hostname, shown in SSH or as root.
- `root_indicator` — `#` as root, hidden otherwise.
- `date` — current date, formatted via `date-format` (strftime, default `%d.%m.%y`).
- `virtualenv` / `anaconda` / `nodeenv` — active environment name (`(name)` /
  `[name]`) from `$VIRTUAL_ENV` / `$CONDA_PREFIX` / `$NODE_VIRTUAL_ENV`; hidden
  when not activated.
- `pyenv` / `nodenv` / `nvm` / `rbenv` / `chruby` / `rvm` / `goenv` / `jenv` /
  `phpenv` / `luaenv` / `plenv` / `scalaenv` / `perlbrew` — language version from
  the matching env var or an ancestor `.X-version` file; shown only when active.
- `go_version` / `rust_version` / `node_version` / `php_version` /
  `java_version` / `dotnet_version` / `swift_version` / `terraform_version` —
  toolchain version parsed from `<cmd> --version`, shown only when the command
  is available (output cached).
- `cpu_arch` — CPU architecture from `/proc/sys/kernel/arch` (fallback `uname -m`).
- `load` — system load from `/proc/loadavg`.
- `ram` — available memory (`MemAvailable`, human-readable).
- `swap` — used swap (from `/proc/meminfo`).
- `disk_usage` — used percent of the current directory's partition (`df`).
- `battery` — charge percent + status from `/sys/class/power_supply`.
- `aws` — current AWS profile from `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`.
- `azure` — default subscription name from `azureProfile.json`.
- `gcloud` — active config name from `active_config`.
- `kubecontext` — current context from `$KUBECONFIG`/`~/.kube/config`.
- `terraform` — current workspace from `.terraform/environment`.
- `ip` — first non-loopback IPv4 (`ip -4 addr show`).
- `vpn_ip` — VPN interface IP (tailscale/wg/tun/zt).
- `wifi` — WiFi interface + link quality from `/proc/net/wireless`.
- `public_ip` — public IP via curl (cached).
- `detect_virt` — virtualization type (`systemd-detect-virt`).
- `toolbox` — container / toolbox name from `/run/.containerenv`.
- `dir_writable` — `!` when the cwd is not writable.
- `per_directory_history` — `global`/`local` from `PER_DIRECTORY_HISTORY_TOGGLE`.
- `haskell_stack` — stack version.
- `package` — `name@version` from `package.json`.
- `asdf` — first `.tool-versions` entry.
- `fvm` — Flutter version from `.fvm/flutter_sdk`.
- `google_app_cred` — GCP project_id from `GOOGLE_APPLICATION_CREDENTIALS`.
- `aws_eb_env` — Elastic Beanstalk environment (`eb list`).
- `laravel_version` — Laravel version (`php artisan --version`).
- `rspec_stats` — RSpec coverage ratio of `app/` vs `spec/` `.rb`.
- `todo` / `taskwarrior` / `dropbox` — command-driven segment (hidden when the
  tool isn't installed).
- `ssh` — indicator that the session is SSH (icon only).
- `proxy` — host:port of the first set proxy env var (`all_proxy`/`http_proxy`/…).
- `docker_machine` — `$DOCKER_MACHINE_NAME`.
- `openfoam` — `OF: <version>` from `$WM_PROJECT_VERSION`.
- `nix_shell` — `$IN_NIX_SHELL` (`pure`/`impure`).
- `ranger` / `yazi` / `nnn` / `lf` — nesting level inside the file manager.
- `xplr` / `midnight_commander` / `vim_shell` / `direnv` / `chezmoi_shell` —
  icon-only indicator shown while that program is active (env var set).
- `text` — provides the style for `text "…"` elements.

The same segment name may appear several times in `segments`; the nodes
merge — later ones override style, while `state`s and behavior attributes
accumulate.

## defaults, separators, frame, vcs-remote-icons

- `defaults fg=… bg=… bold=#true` — the fallback every segment ends at;
  also the default foreground for frame glyphs and `text` elements.
- `separators { segment … sub … end … left-tail … right-tail … right-start …
  right-segment … right-sub … gap … }` — the powerline arrow family;
  values are strings (`"\u{e0b0}"`). The node may also carry three color
  properties: `gap-foreground` (foreground of the filler between the left
  and right columns, p10k `MULTILINE_FIRST_PROMPT_GAP_FOREGROUND`), and
  `sub-foreground` / `right-sub-foreground` (foreground of the sub
  separator, which sits on the **same** background as its segment, p10k
  `sep_color`; defaults to the following segment's foreground). The arrows
  themselves do not use these: they blend along the neighboring segment
  backgrounds (powerline style).
- `frame fg=… bg=… bold=#true { first-prefix … first-suffix …
  newline-prefix … newline-suffix … last-prefix … last-suffix … }` —
  row-edge decorations: the first header row uses `first-*`, later header
  rows `newline-*`, the input line `last-*`. Frame-level style
  (`fg`/`bg`/`bold`) applies to every piece; a piece can carry its own
  style to override it, e.g. `last-prefix "╰─" fg=196`. The input prefix
  width participates in geometry: the engine pads the placeholder to
  match it.
- `vcs-remote-icons { github="\u{f113}" … }` — remote domain substring →
  icon, matched in order; unmatched remotes use the default git icon.

## Checklist for config changes

1. kebab-case names; short color keys (`fg`/`bg`).
2. Enablement is flag-based.
3. Booleans are `#true` / `#false`; escapes are `\u{…}`.
4. Breaking changes are discussed and announced in advance.
5. Keep syntax native KDL rather than powerlevel10k-flavored — that
   means nothing here.
6. Config does not make rendering decisions; it only declares style.
