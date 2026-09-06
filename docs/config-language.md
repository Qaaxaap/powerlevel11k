# Config language

Themes are written in KDL v2 and specified via `--config <file>`. The
rules below bind any change to config parsing or rendering.

## Syntax

- Parsed per the KDL v2 spec. Where v1 and v2 differ — e.g. `true` vs
  `#true` — the v2 behavior wins.
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
`separators`, `frame`, `vcs-remote-icons`. Unrecognized top-level nodes
are ignored (forward-compatible) and skipped.

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
- `layout { prompt-add-newline #true … }` inserts a blank line between
  consecutive prompts (p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE`, "loose" layout).
- `layout { transient-prompt #true … }` folds the header to a blank line on
  command submit, leaving only the input line (p10k `TRANSIENT_PROMPT`).

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
- `icon="…"` replaces the default icon; `icon=""` removes it. Defaults:
  `dir` folder, `time` clock, `background_jobs` gear (shown even at zero
  jobs), `os` distro badge, `vcs` per remote domain (see
  `vcs-remote-icons`); everything else has no icon.
- `content="…"` is reserved.
- `disabled #true` keeps the segment parsed and styled but skips
  rendering; it affects every place the segment is used.
- Every other attribute goes into the segment's behavior table, read by
  that segment's renderer. Implemented so far: `dir` reads
  `shorten-dir-length` (int, 1–20, default 1); `command_execution_time`
  reads `threshold-seconds` (int, default 3).
- Style fallback has three levels: `state <NAME>` on the segment →
  segment defaults → top-level `defaults`. `dir` uses states `ANCHOR`
  (anchor path, e.g. `~`) and `SHORTENED` (collapsed components).

What the built-in segments render:

- `dir` — current directory, collapsed from the front (`~` under $HOME),
  components colored per state.
- `vcs` — in a git repo: branch plus counts (`+staged ~unstaged
  !conflicted ?untracked ↑ahead ↓behind ≡stashes`).
- `status` — last exit code: `✓` on 0, `✘ N` otherwise.
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
- `toolbox` — container/toolbox name from `/run/.containerenv`.
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
merge — later ones override style/icon/content, while `state`s and
behavior attributes accumulate.

## defaults, separators, frame, vcs-remote-icons

- `defaults fg=… bg=… bold=#true` — the fallback every segment ends at;
  also the default foreground for frame glyphs and `text` elements.
- `separators { segment … sub … end … right-start … right-segment …
  right-sub … gap … }` — the powerline arrow family; values are strings
  (`"\u{e0b0}"`).
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

---

# 配置语言

主题采用 KDL v2 语言编写，并通过 `--config <文件>` 指定。以下规则约束
一切对配置解析和渲染的改动。

## 语法

- 按 KDL v2 规范解析。如 `true` 与 `#true` 一类 v1 与 v2 不同之处，按照 v2 规范。
- Unicode 转义请使用 `\u{XXXX}`（带花括号)。
- 属性名与配置键请使用 kebab-case 规范，例如 `shorten-dir-length`、`right-start`、
  `vcs-remote-icons`。
- 颜色键使用短名，例如 `fg`、`bg` 保持书写友好。
- 颜色值:0–255(256 色)、`#rrggbb`(真彩)、`default`(终端默认)、
  标准色名(`black` 到 `brightwhite`，16 色)。
- 示例配置应当合理使用注释，如无特殊要求，请均使用 `//`。

## 结构

目前的顶层节点为:`layout`(必写)、`segments`、`defaults`、
`separators`、`frame`、`vcs-remote-icons`。未识别的顶层节点会被忽略
(向前兼容)并跳过渲染。

## 布局

- `layout { left { … } right { … } }`。`left`/`right`
  下使用 `line { ... }` 声明每行样式。
- 行内段的声明顺序即显示顺序。
- 段的启用是 flag: 写出段名即启用。显式 `#false` 允许
  (用于临时注释);`#true` 允许但不建议。
- 行内允许 `text "…"` 静态文本，按 `text` 段样式渲染;其中
  `$VAR` / `${VAR}` 会展开成环境变量。
- 如果您需要空行，请用 `line {}` 进行声明。
- `layout { prompt-add-newline #true … }` 在连续 prompt 之间插入一个空行
  （p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE`，即「宽松」布局）。
- `layout { transient-prompt #true … }` 命令提交时把 header 折叠成空白行，
  只留输入行（p10k `TRANSIENT_PROMPT`）。

## 段

- `segments { <名字> fg=… … }` 的每个子节点给一个段配置样式，节点名
  即段名。段名由引擎定义，为与实现同名写成 snake_case(如
  `command_execution_time`、`background_jobs`、`prompt_char`)。
  引用引擎没实现的段名不会报错，仅静默跳过渲染。
  添加段需修改引擎实现，或使用 `text` 进行实现。
- 样式属性:`fg`、`bg`、`bold`。
- `state <NAME> fg=…` 是某个情境下的覆盖:样式,外加可选 `char`(按字符渲染
  的段用它,如 `state ERROR char="✘"`);回退链见下。
- `text-left` / `text-middle` / `text-right` 是附加文字槽,每个都是子节点
  `text-left "…"`(可选 `fg=…`)。它们把文本拼进段——`left` 在 icon 前,
  `middle` 在 icon 与内容之间(仅当段既有 icon 又有内容时渲染),`right` 在
  内容后——不单独成块。颜色缺省跟段走,`fg` 可覆盖前景(bg/bold 仍跟段)。
- `icon="…"` 覆盖默认图标;`icon=""` 去掉图标。默认图标:`dir` 文件夹、
  `time` 时钟、`background_jobs` 齿轮(任务数为 0 也显示)、`os` 发行版
  徽标、`vcs` 按远端域名(`vcs-remote-icons`);
  其余段没有图标。
- `content="…"` 预留。
- `disabled #true` 让段保持解析和样式并跳过渲染。此配置会影响所有使用该段的位置。
- 其它属性一律进段的行为属性表，由对应段的渲染代码读取。目前实现的
  只有:`dir` 读 `shorten-dir-length`(整数，1–20，默认 1);
  `command_execution_time` 读 `threshold-seconds`(整数，默认 3)。
- 样式回退共三级，段上 `state <NAME>` → 段默认 → 顶层 `defaults`。`dir`
  使用 `ANCHOR`(锚路径，如 `~`)和 `SHORTENED`(被折叠的组件)。

内置段渲染目标:

- `dir` — 当前目录，从前面折叠(在 $HOME 下显示 `~`)，部件按 state
  分色。
- `vcs` — 在 git 仓库里显示分支和计数(`+暂存 ~未暂存 !冲突 ?未跟踪
  ↑领先 ↓落后 ≡stash`)。
- `status` — 上次退出码:0 显示 `✓`，非 0 显示 `✘ N`。
- `time` — 当前时间 HH:MM:SS。
- `command_execution_time` — 上次命令耗时，达到 `threshold-seconds`
  才显示(`3s`、`1m5s`、`1h2m3s`)。
- `background_jobs` — 后台任务数。
- `prompt_char` — 不在 header 绘制;渲染输入行提示符(输入行由 shell 画，
  提示符紧跟在 `frame.last-prefix` 之后)。字符由 `char` 属性配置(默认
  `❯`),可为任意字符串,纯字面(不做环境变量展开);`state ERROR`(上次
  退出码非 0)可覆盖字符与样式。各态的提示符必须等宽(宽度在启动期定死,
  占位符协议依赖它);不等宽 → 引擎报错,并进入干净 shell(不加载主题)。
- `os` — 发行版徽标。
- `context` — SSH 下显示 `user@host`,本地 root 显示 `user`,否则隐藏。
- `user` — 当前用户名。
- `host` — 主机名,SSH 或 root 时显示。
- `root_indicator` — root 时显示 `#`,否则隐藏。
- `date` — 当前日期,由 `date-format`(strftime,默认 `%d.%m.%y`)格式化。
- `virtualenv` / `anaconda` / `nodeenv` — 激活的环境名(`(名字)` / `[名字]`),
  来自 `$VIRTUAL_ENV` / `$CONDA_PREFIX` / `$NODE_VIRTUAL_ENV`;未激活则隐藏。
- `pyenv` / `nodenv` / `nvm` / `rbenv` / `chruby` / `rvm` / `goenv` / `jenv` /
  `phpenv` / `luaenv` / `plenv` / `scalaenv` / `perlbrew` — 语言版本,来自对应
  环境变量或祖先目录的 `.X-version` 文件;仅激活时显示。
- `go_version` / `rust_version` / `node_version` / `php_version` /
  `java_version` / `dotnet_version` / `swift_version` / `terraform_version` —
  从 `<cmd> --version` 解析的工具链版本,命令存在才显示(输出有缓存)。
- `cpu_arch` — 从 `/proc/sys/kernel/arch` 读的 CPU 架构(回退 `uname -m`)。
- `load` — 系统负载(读 `/proc/loadavg`)。
- `ram` — 可用内存(`MemAvailable`,人类可读)。
- `swap` — 已用 swap(读 `/proc/meminfo`)。
- `disk_usage` — 当前目录所在分区的已用百分比(`df`)。
- `battery` — 电量百分比 + 状态(读 `/sys/class/power_supply`)。
- `aws` — 当前 AWS profile(读 `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`)。
- `azure` — 默认订阅名(读 `azureProfile.json`)。
- `gcloud` — 当前配置名(读 `active_config`)。
- `kubecontext` — 当前 context(读 `$KUBECONFIG`/`~/.kube/config`)。
- `terraform` — 当前 workspace(读 `.terraform/environment`)。
- `terraform` — 当前 workspace(读 `.terraform/environment`)。
- `ip` — 第一个非回环 IPv4(跑 `ip -4 addr show`)。
- `vpn_ip` — VPN 接口 IP(tailscale/wg/tun/zt)。
- `wifi` — WiFi 接口 + 信号质量(读 `/proc/net/wireless`)。
- `public_ip` — 公网 IP(curl 查询,有缓存)。
- `public_ip` — 公网 IP(curl 查询,有缓存)。
- `detect_virt` — 虚拟化类型(跑 `systemd-detect-virt`)。
- `toolbox` — 容器名(读 `/run/.containerenv`)。
- `dir_writable` — 当前目录不可写时显示 `!`。
- `per_directory_history` — `PER_DIRECTORY_HISTORY_TOGGLE` 的 `global`/`local`。
- `haskell_stack` — stack 版本。
- `package` — `package.json` 的 `name@version`。
- `asdf` — `.tool-versions` 首行。
- `fvm` — `.fvm/flutter_sdk` 的 Flutter 版本。
- `google_app_cred` — `GOOGLE_APPLICATION_CREDENTIALS` 的 project_id。
- `aws_eb_env` — Elastic Beanstalk 环境名(`eb list`)。
- `laravel_version` — Laravel 版本(`php artisan --version`)。
- `rspec_stats` — `app/` 与 `spec/` `.rb` 比例。
- `todo` / `taskwarrior` / `dropbox` — 命令驱动段(工具未装时隐藏)。
- `ssh` — 当前处于 SSH 会话的指示(仅图标)。
- `proxy` — 第一个已设代理环境变量的 host:port(`all_proxy`/`http_proxy`/…)。
- `docker_machine` — `$DOCKER_MACHINE_NAME`。
- `openfoam` — `$WM_PROJECT_VERSION` 的 `OF: <版本>`。
- `nix_shell` — `$IN_NIX_SHELL`(`pure`/`impure`)。
- `ranger` / `yazi` / `nnn` / `lf` — 在文件管理器里的嵌套层级。
- `xplr` / `midnight_commander` / `vim_shell` / `direnv` / `chezmoi_shell` —
  该程序激活时(对应环境变量已设)仅显示图标的指示段。
- `text` — 给 `text "…"` 元素提供样式。

同一段名可以在 `segments` 里写多次，节点会合并:后出现的覆盖样式/
icon/content，`state` 和行为属性累积。

## defaults、separators、frame、vcs-remote-icons

- `defaults fg=… bg=… bold=#true` — 所有段回退的终点;也是帧字符和
  `text` 元素的默认前景。
- `separators { segment … sub … end … right-start … right-segment …
  right-sub … gap … }` — powerline 箭头家族，值为字符串(如
  `"\u{e0b0}"`)。
- `frame fg=… bg=… bold=#true { first-prefix … first-suffix …
  newline-prefix … newline-suffix … last-prefix … last-suffix … }` —
  每行行首/行尾的装饰:第一个 header 行用 `first-*`，后面的 header 行用
  `newline-*`，输入行用 `last-*`。帧级样式(`fg`/`bg`/`bold`)作用于所有
  块，单块可带自己的样式覆盖(如 `last-prefix "╰─" fg=196`)。输入前缀的
  宽度参与几何:引擎按它生成等宽占位符。
- `vcs-remote-icons { github="\u{f113}" … }` — 远端域名子串 → 图标，
  按书写顺序匹配，未匹配使用默认 git 图标。

## 涉及配置的检查单

1. 使用 kebab-case 规范;颜色键用短的 `fg`/`bg`。
2. 启用用 flag 语义。
3. 布尔只写 `#true`/`#false`;转义只写 `\u{…}`。
4. 破坏性更改请进行讨论并提前说明。
5. 请使用 KDL 原生语法，而非贴近 `powerlevel10k`，这没有意义。
6. 配置不负责渲染决策，只声明样式。
