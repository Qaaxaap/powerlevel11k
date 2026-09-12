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
- `layout { show-ruler #true … }` lays a full-width ruler line above the
  header (p10k `SHOW_RULER`, off by default). The glyph comes from the
  `ruler` icon (nerdfont/compatible `─`, ascii `-`) and the color from
  `segments { ruler fg=… }`, falling back to `defaults`.
- `layout { right-indent <N> }` keeps N columns free to the right of the
  right column (p10k leaves one because zsh's `ZLE_RPROMPT_INDENT`
  defaults to 1). Default 1; `0` butts the right column against the last
  column. A row whose content plus this indent does not fit drops the
  right column (and its connecting separator) instead of scrolling.

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
    default `~`), `path-separator-foreground` (color of `/`),
    `path-absolute` (p10k `DIR_PATH_ABSOLUTE`), `omit-first-character`
    (p10k `DIR_OMIT_FIRST_CHARACTER`: absolute paths lose the leading
    `/`, the root still shows `/`), `path-highlight-foreground` /
    `path-highlight-bold` (p10k `DIR_PATH_HIGHLIGHT_*`, last component
    only), `hyperlink` (p10k `DIR_HYPERLINK`, OSC 8 `file://$PWD`) and
    `show-writable` (p10k `DIR_SHOW_WRITABLE`, same values:
    `#true` / `"v2"` / `"v3"`; a non-writable directory swaps the icon for
    a lock and the state for `NOT_WRITABLE`, or `NON_EXISTENT` under v3).
    **Note**: p10k's `DIR_MAX_LENGTH` and `DIR_MIN_COMMAND_COLUMNS(_PCT)`
    truncate the directory based on the length of the typed input line
    (p10k redraws the whole prompt on every keystroke); the engine never
    sees the input line, so they are not implemented —
    `truncate_to_unique` instead folds only as much as the row width
    demands.
  - `vcs`: `clean-foreground` / `modified-foreground` /
    `untracked-foreground` / `conflicted-foreground` / `meta-foreground`
    (branch plus ahead/behind/stash, staged and unstaged counts, untracked
    files, conflicts, and the `@` of `@hash` / `#` of `#tag`;
    `conflicted-foreground` falls back to `modified-foreground`,
    `meta-foreground` to the segment style). Note that p10k's generated
    configs draw the untracked count in `%39F` (blue) — that is not the
    same value as `POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND` (76), which
    only feeds the vcs_info fallback path; `meta` mirrors p10k's
    `local meta='%246F'`. Also `show-changeset` and
    `changeset-hash-length` (default 8; only with `show-changeset` does
    the branch get an `icon + hash`). With no local branch the segment
    shows `#tag` when HEAD is tagged and `@hash` otherwise (detached
    HEAD), both as in p10k; `shorten-length` / `shorten-min-length` /
    `shorten-strategy` / `shorten-delimiter` (branch and tag shortening,
    needs both lengths); `staged-symbol` / `unstaged-symbol` /
    `conflicted-symbol` / `untracked-symbol` / `ahead-symbol` /
    `behind-symbol` / `stash-symbol` / `push-ahead-symbol` /
    `push-behind-symbol` (count glyphs, default
    `+ ! ~ ? ⇡ ⇣ * ⇢ ⇠`, matching p10k's formatter); `max-num-staged` /
    `max-num-unstaged` / `max-num-untracked` / `max-num-conflicted`
    (p10k `VCS_*_MAX_NUM`, counting cap, -1 = unlimited) and
    `max-index-size-dirty` (p10k `VCS_MAX_INDEX_SIZE_DIRTY`: above it the
    dirty scan is skipped and p10k's `─` is drawn); `disabled-workdir-pattern`
    (p10k `VCS_DISABLED_WORKDIR_PATTERN`: a repo whose workdir matches is
    treated as if it did not exist, `~` expands to `$HOME` and `|` separates
    alternatives, e.g. `~(|/foo)|/bar/baz/*`). The order matches
    p10k too: `⇣behind⇡ahead` → push counts → `*stash` → in-progress
    action word (`merge`/`rebase`, colored as conflicted) →
    `~conflicts` → `+staged` → `!unstaged` → `?untracked`, with no space
    between ahead and behind.
  - `status`: `ok-foreground` / `error-foreground`, `verbose` (`#false`
    hides success).
  - `command_execution_time`: `threshold-seconds` (default 3), `precision`
    (decimals, default 2), `format="H:M:S"`.
  - `time`: `time-format="12h"`.
  - `vi_mode`: `insert` / `normal` / `visual` / `overwrite` (default
    `INSERT` / `NORMAL` / `VISUAL` / `OVERWRITE`).
  - `date`: `date-format` (strftime, default `%d.%m.%y`).
  - `symfony2_version`: reads the ` VERSION ` line of
    `app/bootstrap.php.cache`.
  - `symfony2_tests`: with `src` and `app/AppKernel.php` present, counts
    `*.php` under `src` and the share of test files, printing
    `SF2: 12.34%` under the states `GOOD` (>=75) / `AVG` (>=50) /
    `BAD` (<50), which default to p10k's cyan/yellow/red so long as the
    config does not declare them.
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

Like p10k, `truncate_to_unique` is width-dependent: it shows the path as
is when the row fits and folds components front to back only as far as
the overflow demands (measured: unfolded at 130/100 columns, one level
folded at 90, two at 80). The other strategies always fold, as in p10k.

What the built-in segments render:

- `dir` — current directory, collapsed from the front (`~` under $HOME),
  components colored per state.
- `vcs` — in a git repo: branch plus counts in p10k's order and with
  p10k's glyphs (`⇣behind⇡ahead *stashes merge ~conflicted +staged
  !unstaged ?untracked`; no local branch → `#tag` or `@hash`).
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
  Extended states follow p10k's `STATUS_EXTENDED_STATES`, driven by the
  pipeline's exit codes: `OK_PIPE` (an earlier stage failed, the last one
  succeeded), `ERROR_PIPE` (the pipeline failed), `ERROR_SIGNAL` (killed by
  a signal, i.e. exit code above 128, and not a pipeline). A state the
  config does not declare falls back to `ERROR`, and `OK_PIPE` falls back to
  no state, so a config that only knows `ERROR` is unaffected. The shells
  report `$pipestatus`; pwsh has no such column, so it never reaches these
  three states.
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

- `defaults fg=… bg=… bold=#true` — the fallback every **segment** ends at
  (the analogue of the global `POWERLEVEL9K_BACKGROUND` that p10k's
  segments inherit): a segment that does not set `bg` inherits it, and
  with no `bg` in `defaults` either the segment is **transparent** (the
  terminal's own background; earlier versions forced black here). Frame
  glyphs and `text` elements only inherit `fg` / `bold` from here and
  **never its `bg`** — p10k's `╭─` / `╰─` are foreground-only in classic
  too. That is why uniform-background styles write `bg` on every segment,
  and why the wizard gives segments it adds later (the time segment) the
  same background for the current style.
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
- `dir-classes { class "~/work/**" state="WORK" icon="★" … }` — `$PWD` is
  matched against each class in order and the first hit picks the dir
  segment's state (styled via `state WORK fg=…`) and icon; `icon=""` means
  no icon at all, as in p10k. With `show-writable` the state also gets the
  `_NOT_WRITABLE` / `_NON_EXISTENT` suffix (p10k's rule). The pattern
  dialect differs from p10k's zsh extended globs: `~` expands to $HOME,
  `*` `?` `[…]` stay within one path component, `**` crosses directories,
  and a trailing `/` matches the whole subtree.
- With no `dir-classes` node, the dir icon follows p10k's own four
  built-in classes: `/etc` and below → `etc`, `$HOME` → `home`, any
  directory below it → `home-sub`, everything else → `folder`. They are
  icon-table entries like any other, so `icon { home-sub { all "H" } }`
  overrides one.

## Checklist for config changes

1. kebab-case names; short color keys (`fg`/`bg`).
2. Enablement is flag-based.
3. Booleans are `#true` / `#false`; escapes are `\u{…}`.
4. Breaking changes are discussed and announced in advance.
5. Keep syntax native KDL rather than powerlevel10k-flavored — that
   means nothing here.
6. Config does not make rendering decisions; it only declares style.
