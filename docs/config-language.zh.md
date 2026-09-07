# 配置语言

主题用 KDL v2 编写，通过 `--config <文件>` 指定。下面的规则约束一切
与配置解析和渲染相关的改动，并可作为配置参考。

## 语法

- 按 KDL v2 规范解析；v1 与 v2 不同处（如 `true` vs `#true`）以 v2 为准。
- Unicode 转义用 `\u{XXXX}`（带花括号）。
- 属性名与节点名用 kebab-case：`shorten-dir-length`、`right-start`、
  `vcs-remote-icons`。
- 颜色键用短名 `fg` / `bg`，保持书写友好。
- 颜色值：`0`–`255`（256 色）、`#rrggbb`（真彩）、`default`（终端默认）、
  标准色名（`black` … `brightwhite`，16 色）。
- 示例配置用 `//` 注释，除非有理由不用。

## 结构

目前的顶层节点：`layout`（必写）、`segments`、`defaults`、`separators`、
`frame`、`vcs-remote-icons`、`mode`、`icon`。未识别的顶层节点会被忽略并跳过渲染。

## 图标模式

- `mode "nerdfont-complete"` / `mode "nerdfont-fontconfig"` /
  `mode "compatible"` / `mode "ascii"` 选择内置段图标的字符集。`mode` 缺省时默认值取决于 locale：
  UTF-8（普通终端）→ `nerdfont-complete`；非 UTF-8（控制台 / C locale）
  → `ascii`。
- `nerdfont-complete` 与 `nerdfont-fontconfig` 用同一套 Nerd Font 字形，
  拆分仅出于考虑 p10k 习惯，实际是三套图标集。`compatible`
  用标准 Unicode + Powerline 字形；`ascii` 用纯文本。
- 顶层 `icon {}` 按**图标名**逐档覆盖某个图标的字符。图标名跟着字形语义走
  （`folder`、`go`、`ok`、`error`、`branch`…），多个段共享一个图标名时一处
  生效（如 `python` 同时覆盖 `virtualenv`/`anaconda`/`pyenv`）：

  ```kdl
  icon {
      ok     { all "V" }              // 纯 ASCII → 三档全落
      error  { all "X" }
      folder { nf "\u{f07c}" compat "" ascii "" } // 每档独立
  }
  ```

  - `all` 自动识别字符类别：Nerd Font 私有区字符只落 `nf` 档；标准
    Unicode（如 ✔）落 `nf` + `compat`；纯 ASCII 三档全落。
  - `nf` / `compat` / `ascii` 配置对应档位字符（空串 = 显式无
    图标）；缺省字段回退引擎默认表（随 `mode` 变）。
- 引擎默认字符取自 p10k。用户 `icon {}` /
  `separators` / `frame` 优先于 mode 默认。

`icon {}` 的键是图标名。多数段引用的图标名与段名相同（`time`、`date`、
`aws`…直接用段名当键）；图标名与段名不同、或多个段共享一个图标名的见下表
（覆盖一处即对所有引用段生效）：

| 图标名 | 引用的段 |
|---|---|
| `folder` | `dir` |
| `git` | `vcs`（无远端时） |
| `background-jobs` | `background_jobs` |
| `go` | `go_version`、`goenv` |
| `rust` | `rust_version` |
| `node` | `node_version`、`nodeenv`、`nodenv`、`nvm` |
| `php` | `php_version`、`phpenv` |
| `java` | `java_version`、`jenv` |
| `dotnet` | `dotnet_version` |
| `terraform` | `terraform_version`、`terraform` |
| `cpu-arch` | `cpu_arch` |
| `python` | `virtualenv`、`anaconda`、`pyenv` |
| `ruby` | `rbenv`、`chruby`、`rvm` |
| `lua` | `luaenv` |
| `perl` | `plenv`、`perlbrew` |
| `scala` | `scalaenv` |
| `disk` | `disk_usage` |
| `gcloud` | `gcloud`、`google_app_cred` |
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
| `ok` / `error` | `status` 的 OK / ERROR 态 |
| `branch` | `vcs` 分支前的图标 |

`os` 的徽标随发行版动态（arch/ubuntu/…），不经过 `icon {}` 覆盖。

## 布局

- `layout { left { … } right { … } }`。`left` / `right` 下用 `line { … }`
  声明每行。
- 行内段的声明顺序即显示顺序。
- 段的启用是 flag：写出段名即启用；显式 `#false` 可临时注释掉一段；
  `#true` 允许但不建议。
- 行内可以放 `text "…"` 静态文本，按 `text` 段样式渲染；其中的
  `$VAR` / `${VAR}` 会展开成环境变量。
- 需要空行就声明 `line {}`。
- `layout { prompt-add-newline #true … }` 在连续 prompt 之间插入一个空行
  （p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE`，「宽松」布局）。
- `layout { transient-prompt #true … }` 在命令提交瞬间把多行 header 折叠成
  单行 `❯`（p10k `TRANSIENT_PROMPT`）。仅 zsh 支持：依赖 `zle reset-prompt`，
  bash/fish 会忽略。

## 段

- `segments { <名字> fg=… … }` 的每个子节点配置一个段的样式，节点名即段名。
  段名由引擎定义，为与实现同名写成 snake_case（`command_execution_time`、
  `background_jobs`、`prompt_char`）。引用引擎没实现的段名不会报错，只是
  静默跳过。要新增段只能扩展引擎，或改用 `text`。
- 样式属性：`fg`、`bg`、`bold`。
- `state <NAME> fg=…` 是情境覆盖：样式 + 可选 `char`（按字符渲染的段用它，
  如 `state ERROR char="✘"`）；回退链见下。
- `text-left` / `text-middle` / `text-right` 是附加文字槽，每个是子节点
  `text-left "…"`（可选 `fg=…`）。它们把文字拼进段——`left` 在图标前，
  `middle` 在图标与内容之间（仅当段既有图标又有内容时渲染），`right` 在内容
  后——不单独成块。颜色缺省跟段走，`fg` 覆盖前景（bg/bold 仍跟段）。
- 图标不是段属性。每个段引用一个**图标名**（`dir` → `folder`、`vcs` → `git`、
  `time` → `time`…），按当前 `mode` 从引擎默认表取字符，也可在顶层 `icon {}`
  里按图标名覆盖（见[图标模式](#图标模式)）。各段的默认：`dir` 文件夹、
  `time` 时钟、`background_jobs` 齿轮（任务数为 0 也显示）、`os` 发行版徽标
  （随发行版动态）、`vcs` git（或按远端域名，见 `vcs-remote-icons`）；语言 /
  云 / 系统段映射到各自的图标名（go、python、aws…），其余无图标。
- `content="…"` 预留。
- `disabled #true` 让段保持解析和样式但跳过渲染；影响所有用到该段的位置。
- 其余属性都进段的行为属性表，由该段的渲染代码读取。目前实现的：`dir` 读
  `shorten-dir-length`（整数，1–20，默认 1）；`command_execution_time` 读
  `threshold-seconds`（整数，默认 3）。
- 样式回退三级：段上 `state <NAME>` → 段默认 → 顶层 `defaults`。`dir` 用
  `ANCHOR`（锚路径，如 `~`）和 `SHORTENED`（被折叠的组件）两个 state。

内置段渲染目标：

- `dir` — 当前目录，从前面折叠（$HOME 下显示 `~`），部件按 state 分色。
- `vcs` — git 仓库里显示分支和计数（`+暂存 ~未暂存 !冲突 ?未跟踪
  ↑领先 ↓落后 ≡stash`）。
- `status` — 上次退出码：取图标名 `ok` / `error` 的字符（随 `mode`，如
  ascii 档显示 `ok` / `err N`），出错时附加 `N`。
- `time` — 当前时间 HH:MM:SS。
- `command_execution_time` — 上次命令耗时，达到 `threshold-seconds` 才显示
  （`3s`、`1m5s`、`1h2m3s`）。
- `background_jobs` — 后台任务数。
- `prompt_char` — 不在 header 绘制；渲染输入行提示符（输入行由 shell 画，
  提示符紧跟 `frame.last-prefix` 之后）。字符由 `char` 属性配置（默认 `❯`），
  可为任意字符串，纯字面（不做环境变量展开）；`state ERROR`（上次退出码
  非 0）可覆盖字符与样式。各态的提示符必须等宽（宽度启动期定死，占位符
  协议依赖它）；不等宽 → 引擎报错，进入干净 shell 而不是加载主题。
- `os` — 发行版徽标。
- `context` — SSH 下 `user@host`，本地 root 显示 `user`，否则隐藏。
- `user` — 当前用户名。
- `host` — 主机名，SSH 或 root 时显示。
- `root_indicator` — root 时显示 `#`，否则隐藏。
- `date` — 当前日期，由 `date-format` 格式化（strftime，默认 `%d.%m.%y`）。
- `virtualenv` / `anaconda` / `nodeenv` — 激活的环境名（`(名字)` / `[名字]`），
  来自 `$VIRTUAL_ENV` / `$CONDA_PREFIX` / `$NODE_VIRTUAL_ENV`；未激活则隐藏。
- `pyenv` / `nodenv` / `nvm` / `rbenv` / `chruby` / `rvm` / `goenv` / `jenv` /
  `phpenv` / `luaenv` / `plenv` / `scalaenv` / `perlbrew` — 语言版本，来自对应
  环境变量或祖先目录的 `.X-version` 文件；仅激活时显示。
- `go_version` / `rust_version` / `node_version` / `php_version` /
  `java_version` / `dotnet_version` / `swift_version` / `terraform_version` —
  从 `<cmd> --version` 解析的工具链版本，命令存在才显示（输出有缓存）。
- `cpu_arch` — CPU 架构（读 `/proc/sys/kernel/arch`，回退 `uname -m`）。
- `load` — 系统负载（读 `/proc/loadavg`）。
- `ram` — 可用内存（`MemAvailable`，人类可读）。
- `swap` — 已用 swap（读 `/proc/meminfo`）。
- `disk_usage` — 当前目录所在分区的已用百分比（`df`）。
- `battery` — 电量百分比 + 状态（读 `/sys/class/power_supply`）。
- `aws` — 当前 AWS profile（读 `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`）。
- `azure` — 默认订阅名（读 `azureProfile.json`）。
- `gcloud` — 当前配置名（读 `active_config`）。
- `kubecontext` — 当前 context（读 `$KUBECONFIG`/`~/.kube/config`）。
- `terraform` — 当前 workspace（读 `.terraform/environment`）。
- `ip` — 第一个非回环 IPv4（跑 `ip -4 addr show`）。
- `vpn_ip` — VPN 接口 IP（tailscale/wg/tun/zt）。
- `wifi` — WiFi 接口 + 信号质量（读 `/proc/net/wireless`）。
- `public_ip` — 公网 IP（curl 查询，有缓存）。
- `detect_virt` — 虚拟化类型（跑 `systemd-detect-virt`）。
- `toolbox` — 容器 / toolbox 名（读 `/run/.containerenv`）。
- `dir_writable` — 当前目录不可写时显示 `!`。
- `per_directory_history` — `PER_DIRECTORY_HISTORY_TOGGLE` 的 `global`/`local`。
- `haskell_stack` — stack 版本。
- `package` — `package.json` 的 `name@version`。
- `asdf` — `.tool-versions` 首行。
- `fvm` — `.fvm/flutter_sdk` 的 Flutter 版本。
- `google_app_cred` — `GOOGLE_APPLICATION_CREDENTIALS` 的 project_id。
- `aws_eb_env` — Elastic Beanstalk 环境名（`eb list`）。
- `laravel_version` — Laravel 版本（`php artisan --version`）。
- `rspec_stats` — `app/` 与 `spec/` 的 `.rb` 覆盖比例。
- `todo` / `taskwarrior` / `dropbox` — 命令驱动段（工具未装时隐藏）。
- `ssh` — SSH 会话指示（仅图标）。
- `proxy` — 第一个已设代理环境变量的 host:port（`all_proxy`/`http_proxy`/…）。
- `docker_machine` — `$DOCKER_MACHINE_NAME`。
- `openfoam` — `$WM_PROJECT_VERSION` 的 `OF: <版本>`。
- `nix_shell` — `$IN_NIX_SHELL`（`pure`/`impure`）。
- `ranger` / `yazi` / `nnn` / `lf` — 在文件管理器里的嵌套层级。
- `xplr` / `midnight_commander` / `vim_shell` / `direnv` / `chezmoi_shell` —
  程序激活时（对应环境变量已设）仅显示图标的指示段。
- `text` — 给 `text "…"` 元素提供样式。

同一段名可以在 `segments` 里写多次，节点会合并：后出现的覆盖样式，
`state` 和行为属性累积。

## defaults、separators、frame、vcs-remote-icons

- `defaults fg=… bg=… bold=#true` — 所有段回退的终点；也是帧字符和
  `text` 元素的默认前景。
- `separators { segment … sub … end … right-start … right-segment …
  right-sub … gap … }` — powerline 箭头家族，值为字符串（如 `"\u{e0b0}"`）。
- `frame fg=… bg=… bold=#true { first-prefix … first-suffix …
  newline-prefix … newline-suffix … last-prefix … last-suffix … }` —
  每行行首/行尾的装饰：第一个 header 行用 `first-*`，后续 header 行用
  `newline-*`，输入行用 `last-*`。帧级样式（`fg`/`bg`/`bold`）作用于所有块，
  单块可带自己的样式覆盖（如 `last-prefix "╰─" fg=196`）。输入前缀的宽度
  参与几何：引擎按它生成等宽占位符。
- `vcs-remote-icons { github="\u{f113}" … }` — 远端域名子串 → 图标，
  按书写顺序匹配，未匹配用默认 git 图标。

## 涉及配置的检查单

1. 名字用 kebab-case；颜色键用短的 `fg`/`bg`。
2. 启用用 flag 语义。
3. 布尔只写 `#true`/`#false`；转义只写 `\u{…}`。
4. 破坏性变更先讨论并提前说明。
5. 保持 KDL 原生语法，不贴近 powerlevel10k 的写法——这里没有可搬的东西。
6. 配置不做渲染决策，只声明样式。
