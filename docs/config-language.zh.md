# 配置语言

主题用 KDL v2 编写，通过 `--config <文件>` 指定。下面的规则约束一切与配置解析和渲染相关的改动，并可作为配置参考。

## 语法

- 按 KDL v2 规范解析；v1 与 v2 不同处（如 `true` vs `#true`）以 v2 为准。
- Unicode 转义用 `\u{XXXX}`（带花括号）。
- 属性名与节点名用 kebab-case：`shorten-dir-length`、`right-start`、`vcs-remote-icons`。
- 颜色键用短名 `fg` / `bg`，保持书写友好。
- 颜色值：`0`–`255`（256 色）、`#rrggbb`（真彩）、`default`（终端默认）、标准色名（`black`…`brightwhite`，16 色）。
- 示例配置用 `//` 注释，除非有理由不用。

## 结构

目前的顶层节点：`layout`（必写）、`segments`、`defaults`、`separators`、`frame`、`vcs-remote-icons`、`mode`、`icon`、`dir-classes`、`shell`。未识别的顶层节点会被忽略并跳过渲染。

`shell "zsh"` 指定引擎代理哪个 shell，这样 rc 里的启动行不必再写 `--shell`；命令行的 `--shell` 优先级更高，两者都没有时回退到 `$SHELL`。

## 图标模式

- `mode "nerdfont-complete"` / `mode "nerdfont-fontconfig"` / `mode "compatible"` / `mode "ascii"` 选择内置段图标的字符集。`mode` 缺省时默认值取决于 locale：UTF-8（普通终端）→ `nerdfont-complete`；非 UTF-8（控制台 / C locale）→ `ascii`。
- `nerdfont-complete` 与 `nerdfont-fontconfig` 用同一套 Nerd Font 字形，拆分仅出于考虑 p10k 习惯，实际是三套图标集。`compatible` 用标准 Unicode + Powerline 字形；`ascii` 用纯文本。
- 顶层 `icon {}` 按**图标名**逐档覆盖某个图标的字符。图标名跟着字形语义走（`folder`、`go`、`ok`、`error`、`branch`…），多个段共享一个图标名时一处生效（如 `python` 同时覆盖 `virtualenv`/`anaconda`/`pyenv`）：

  ```kdl
  icon {
      ok     { all "V" }              // 纯 ASCII → 三档全落
      error  { all "X" }
      folder { nf "\u{f07c}" compat "" ascii "" } // 每档独立
  }
  ```

  - `all` 自动识别字符类别：Nerd Font 私有区字符只落 `nf` 档；标准 Unicode（如 ✔）落 `nf` + `compat`；纯 ASCII 三档全落。
  - `nf` / `compat` / `ascii` 配置对应档位字符（空串 = 显式无图标）；缺省字段回退引擎默认表（随 `mode` 变）。
- 引擎默认字符取自 p10k。用户 `icon {}` / `separators` / `frame` 优先于 mode 默认。

`icon {}` 的键是图标名。多数段引用的图标名与段名相同（`time`、`date`、`aws`…直接用段名当键）；图标名与段名不同、或多个段共享一个图标名的见下表（覆盖一处即对所有引用段生效）：

| 图标名 | 引用的段 |
|---|---|
| `folder` | `dir` |
| `git` | `vcs`（无远端时）|
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

- `layout { left { … } right { … } }`。`left` / `right` 下用 `line { … }` 声明每行。
- 行内段的声明顺序即显示顺序。
- 段的启用是 flag：写出段名即启用；显式 `#false` 可临时注释掉一段；`#true` 允许但不建议。
- 行内可以放 `text "…"` 静态文本，按 `text` 段样式渲染；其中的 `$VAR` / `${VAR}` 会展开成环境变量。
- 需要空行就声明 `line {}`。
- `layout { prompt-add-newline <N> … }` 在连续 prompt 之间插入 N 个空行（p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE` + `_COUNT`）。写 `#true` 等于 1，写 `0` 关闭。
- `layout { transient-prompt #true … }` 在命令提交瞬间把多行 header 折叠成单行 `❯`（p10k `TRANSIENT_PROMPT`）。仅 zsh 支持：依赖 `zle reset-prompt`，bash/fish 会忽略。
- `layout { show-ruler #true … }` 在 header 之上再铺一整行标尺（p10k `SHOW_RULER`，缺省关）。字符取图标名 `ruler`（nerdfont/compatible 档 `─`、ascii 档 `-`），颜色取 `segments { ruler fg=… }`，没配就用 `defaults`。
- `layout { right-indent <N> }` 让右栏右侧空出 N 格（p10k 空一格，因为 zsh 的 `ZLE_RPROMPT_INDENT` 默认是 1）。缺省 1，写 `0` 则右栏贴最后一格。整行内容加这个缩进放不下时，直接丢掉右栏（连带它的连接分隔符），不会溢出换行。

## 段

- `segments { <名字> fg=……}` 的每个子节点配置一个段的样式，节点名即段名。段名由引擎定义，为与实现同名写成 snake_case（`command_execution_time`、`background_jobs`、`prompt_char`）。引用引擎没实现的段名不会报错，只是静默跳过。要新增段只能扩展引擎，或改用 `text`。
- 样式属性：`fg`、`bg`、`bold`。
- `state <NAME> fg=…` 是情境覆盖：样式 + 可选 `char`（按字符渲染的段用它，如 `state ERROR char="✘"`）；回退链见下。
- `text-left` / `text-middle` / `text-right` 是附加文字槽，每个是子节点 `text-left "…"`（可选 `fg=…`）。它们把文字拼进段——`left` 在图标前，`middle` 在图标与内容之间（仅当段既有图标又有内容时渲染），`right` 在内容后——不单独成块。颜色缺省跟段走，`fg` 覆盖前景（bg/bold 仍跟段）。
- 图标不是段属性。每个段引用一个**图标名**（`dir` → `folder`、`vcs` → `git`、`time` → `time`…），按当前 `mode` 从引擎默认表取字符，也可在顶层 `icon {}` 里按图标名覆盖（见[图标模式](#图标模式)）。各段的默认：`dir` 文件夹、`time` 时钟、`background_jobs` 齿轮（任务数为 0 也显示）、`os` 发行版徽标（随发行版动态）、`vcs` git（或按远端域名，见 `vcs-remote-icons`）；语言 / 云 / 系统段映射到各自的图标名（go、python、aws…），其余无图标。
- `content="…"` 预留。
- `disabled #true` 让段保持解析和样式但跳过渲染；影响所有用到该段的位置。
- 其余属性都进段的行为属性表，由该段的渲染代码读取。目前实现的：
  - `dir`：`shorten-strategy`（折叠策略，见[目录折叠](#目录折叠)）、`shorten-dir-length`（保留级数 / 每级字符数，1–20，默认 1）、`shorten-delimiter`（省略符，默认 `…`；`truncate_to_unique` 不输出它）、`shorten-folder-marker`（marker 文件名，缺省用内置列表）、`home-abbreviation`（home 前缀，默认 `~`）、`path-separator-foreground`（`/` 的颜色）、`path-absolute`（p10k `DIR_PATH_ABSOLUTE`：不缩写 home）、`omit-first-character`（p10k `DIR_OMIT_FIRST_CHARACTER`：绝对路径省掉开头的 `/`，根目录仍显示 `/`）、`path-highlight-foreground` / `path-highlight-bold`（p10k `DIR_PATH_HIGHLIGHT_*`：只给末级组件配色/加粗）、`hyperlink`（p10k `DIR_HYPERLINK`：给目录套 OSC 8 超链接，指向 `file://$PWD`）、`show-writable`（p10k `DIR_SHOW_WRITABLE`，取值同 p10k：`#true` / `"v2"` / `"v3"`；不可写时图标换成锁、state 换成 `NOT_WRITABLE`，v3 下目录不存在则是 `NON_EXISTENT`）。**注意**：p10k 的 `DIR_MAX_LENGTH` / `DIR_MIN_COMMAND_COLUMNS(_PCT)` 是按输入行长度动态截断目录的（p10k 每次按键重画整个 prompt），引擎看不到输入行，故不实现；`truncate_to_unique` 改成按"整行是否超宽"决定折几级。
  - `vcs`：`clean-foreground` / `modified-foreground` / `untracked-foreground` / `conflicted-foreground` / `meta-foreground`（分支与 ahead/behind/stash、staged 与 unstaged 计数、未跟踪、冲突、以及 `@hash` 的 `@` 和 `#tag` 的 `#` 各自的颜色；`conflicted-foreground` 缺省回退 `modified-foreground`，`meta-foreground` 缺省回退段样式）。注意 p10k 生成配置里未跟踪计数是 `%39F`（蓝），与 `POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND`（76，只用于 vcs_info 回退路径）不是同一个值；`meta` 对应 p10k 格式化函数里的 `local meta='%246F'`。`show-changeset` 与 `changeset-hash-length`（默认 8，显式开启时才在分支旁画 `图标 + hash`）；没有本地分支时按 p10k 显示 `#tag`（有标签）或 `@hash`（detached HEAD）；`shorten-length` / `shorten-min-length` / `shorten-strategy` / `shorten-delimiter`（分支名与标签名折叠，两个 length 都配才生效）；`staged-symbol` / `unstaged-symbol` / `conflicted-symbol` / `untracked-symbol` / `ahead-symbol` / `behind-symbol` / `stash-symbol` / `push-ahead-symbol` / `push-behind-symbol`（计数符号，默认 `+ ! ~ ? ⇡ ⇣ * ⇢ ⇠`，与 p10k 格式化函数一致）；`max-num-staged` / `max-num-unstaged` / `max-num-untracked` / `max-num-conflicted`（p10k `VCS_*_MAX_NUM`：计数上限，-1 = 不限）、`max-index-size-dirty`（p10k `VCS_MAX_INDEX_SIZE_DIRTY`：索引超过它就跳过 dirty 扫描，此时按 p10k 画 `─`）、`disabled-workdir-pattern`（p10k `VCS_DISABLED_WORKDIR_PATTERN`：仓库根目录匹配该模式的仓库视为不存在；`~` 展开为 `$HOME`，`|` 分隔多个模式，如 `~(|/foo)|/bar/baz/*`）。计数顺序也照抄 p10k：`⇣behind⇡ahead` → `*stash` → 进行中的操作词（`merge`/`rebase`，用 conflicted 色）→ `~冲突` → `+暂存` → `!未暂存` → `?未跟踪`；ahead 与 behind 之间不加空格。
  - `status`：`ok-foreground` / `error-foreground`、`verbose`（`#false` 时成功不显示）。
  - `command_execution_time`：`threshold-seconds`（默认 3）、`precision`（小数位，默认 2）、`format="H:M:S"`。
  - `time`：`time-format="12h"`。
  - `vi_mode`：`insert` / `normal` / `visual` / `overwrite`（默认 `INSERT` / `NORMAL` / `VISUAL` / `OVERWRITE`）。
  - `date`：`date-format`（strftime，默认 `%d.%m.%y`）。
  - 任意段：`visual-identifier-color` 覆盖该段**图标**的前景色（缺省跟段）。
- 样式回退三级：段上 `state <NAME>` → 段默认 → 顶层 `defaults`。`dir` 用 `ANCHOR`（锚路径，如 `~`）和 `SHORTENED`（被折叠的组件）两个 state；`prompt_char` 除 `ERROR` 外还认 `VIINS` / `VICMD` / `VIVIS` / `VIOWR`（配了对应 `char` 就随 zsh 编辑模式换提示符）。

## 目录折叠

`dir` 的 `shorten-strategy` 对齐 p10k 的 `POWERLEVEL9K_SHORTEN_STRATEGY`：

- `truncate_to_unique`（默认）：每级缩到兄弟目录里的最短唯一前缀，不留省略符。
- `truncate_middle`：每级留前 `shorten-dir-length` + 省略符 + 后同样多字符。
- `truncate_from_right`：每级留前 `shorten-dir-length` + 省略符。
- `truncate_to_last`：只留末 `shorten-dir-length` 级。
- `truncate_to_first_and_last`：首尾各留 `shorten-dir-length` 级，中间省略。
- `truncate_absolute` / `truncate_absolute_chars`：整条路径按字符数截断。
- `truncate_with_folder_marker`：在 marker 文件处折叠。

`truncate_to_unique` 与 p10k 一样**按行宽动态**：整行放得下时原样显示，超宽了才从前往后逐级折到够省为止（实测 130/100 列不折、90 列折一级、80 列折两级）。其余策略与宽度无关，照 p10k 恒定折叠。

内置段渲染目标：

- `dir`—当前目录，从前面折叠（$HOME 下显示 `~`），部件按 state 分色。
- `vcs`—git 仓库里显示分支和计数，顺序与符号同 p10k：`⇣落后⇡领先 *stash merge ~冲突 +暂存 !未暂存 ?未跟踪`；无本地分支时显示 `#tag` 或 `@hash`。
- `status`—上次退出码：取图标名 `ok` / `error` 的字符（随 `mode`，如 ascii 档显示 `ok` / `err N`），出错时附加 `N`。
- `time`—当前时间 HH:MM:SS。
- `command_execution_time`—上次命令耗时，达到 `threshold-seconds` 才显示（`3s`、`1m5s`、`1h2m3s`）。
- `background_jobs`—后台任务数。
- `prompt_char`—不在 header 绘制；渲染输入行提示符（输入行由 shell 画，提示符紧跟 `frame.last-prefix` 之后）。字符由 `char` 属性配置（默认 `❯`），可为任意字符串，纯字面（不做环境变量展开）；`state ERROR`（上次退出码非 0）可覆盖字符与样式。各态的提示符必须等宽（宽度启动期定死，占位符协议依赖它）；不等宽 → 引擎报错，进入干净 shell 而不是加载主题。扩展状态对齐 p10k 的 `STATUS_EXTENDED_STATES`，判据来自管道的各段退出码：`OK_PIPE`（前段失败、最后一段成功）、`ERROR_PIPE`（管道失败）、`ERROR_SIGNAL`（被信号杀死，即退出码 > 128 且不是管道）。配置里没声明的状态回退到 `ERROR`，`OK_PIPE` 回退到无状态，所以只认 `ERROR` 的配置不受影响。各 shell 上报 `$pipestatus`；pwsh 没有这一列，因此不会进入这三个状态。
- `os`—发行版徽标。
- `context`—SSH 下 `user@host`，本地 root 显示 `user`，否则隐藏。
- `user`—当前用户名。
- `host`—主机名，SSH 或 root 时显示。
- `root_indicator`—root 时显示 `#`，否则隐藏。
- `date`—当前日期，由 `date-format` 格式化（strftime，默认 `%d.%m.%y`）。
- `symfony2_version`—从 `app/bootstrap.php.cache` 里带 ` VERSION ` 的行取版本号。
- `symfony2_tests`—`src` + `app/AppKernel.php` 存在时统计 `src/**/*.php` 里测试文件占比，输出 `SF2: 12.34%`，按比例切 state `GOOD`（≥75）/ `AVG`（≥50）/ `BAD`（<50），未声明这些 state 时用 p10k 的内部默认色（青/黄/红）。
- `virtualenv` / `anaconda` / `nodeenv`—激活的环境名（`(名字)` / `[名字]`），来自 `$VIRTUAL_ENV` / `$CONDA_PREFIX` / `$NODE_VIRTUAL_ENV`；未激活则隐藏。
- `pyenv` / `nodenv` / `nvm` / `rbenv` / `chruby` / `rvm` / `goenv` / `jenv` / `phpenv` / `luaenv` / `plenv` / `scalaenv` / `perlbrew`—语言版本，来自对应环境变量或祖先目录的 `.X-version` 文件；仅激活时显示。
- `go_version` / `rust_version` / `node_version` / `php_version` / `java_version` / `dotnet_version` / `swift_version` / `terraform_version`—从 `<cmd> --version` 解析的工具链版本，命令存在才显示（输出有缓存）。
- `cpu_arch`—CPU 架构（读 `/proc/sys/kernel/arch`，回退 `uname -m`）。
- `load`—系统负载（读 `/proc/loadavg`）。
- `ram`—可用内存（`MemAvailable`，人类可读）。
- `swap`—已用 swap（读 `/proc/meminfo`）。
- `disk_usage`—当前目录所在分区的已用百分比（`df`）。
- `battery`—电量百分比 + 状态（读 `/sys/class/power_supply`）。
- `aws`—当前 AWS profile（读 `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`）。
- `azure`—默认订阅名（读 `azureProfile.json`）。
- `gcloud`—当前配置名（读 `active_config`）。
- `kubecontext`—当前 context（读 `$KUBECONFIG`/`~/.kube/config`）。
- `terraform`—当前 workspace（读 `.terraform/environment`）。
- `ip`—第一个非回环 IPv4（跑 `ip -4 addr show`）。
- `vpn_ip`—VPN 接口 IP（tailscale/wg/tun/zt）。
- `wifi`—WiFi 接口 + 信号质量（读 `/proc/net/wireless`）。
- `public_ip`—公网 IP（curl 查询，有缓存）。
- `detect_virt`—虚拟化类型（跑 `systemd-detect-virt`）。
- `toolbox`—容器 / toolbox 名（读 `/run/.containerenv`）。
- `dir_writable`—当前目录不可写时显示 `!`。
- `per_directory_history`—`PER_DIRECTORY_HISTORY_TOGGLE` 的 `global`/`local`。
- `haskell_stack`—stack 版本。
- `package`—`package.json` 的 `name@version`。
- `asdf`—`.tool-versions` 首行。
- `fvm`—`.fvm/flutter_sdk` 的 Flutter 版本。
- `google_app_cred`—`GOOGLE_APPLICATION_CREDENTIALS` 的 project_id。
- `aws_eb_env`—Elastic Beanstalk 环境名（`eb list`）。
- `laravel_version`—Laravel 版本（`php artisan --version`）。
- `rspec_stats`—`app/` 与 `spec/` 的 `.rb` 覆盖比例。
- `todo` / `taskwarrior` / `dropbox`—命令驱动段（工具未装时隐藏）。
- `ssh`—SSH 会话指示（仅图标）。
- `proxy`—第一个已设代理环境变量的 host:port（`all_proxy`/`http_proxy`/…）。
- `docker_machine`—`$DOCKER_MACHINE_NAME`。
- `openfoam`—`$WM_PROJECT_VERSION` 的 `OF: <版本>`。
- `nix_shell`—`$IN_NIX_SHELL`（`pure`/`impure`）。
- `ranger` / `yazi` / `nnn` / `lf`—在文件管理器里的嵌套层级。
- `xplr` / `midnight_commander` / `vim_shell` / `direnv` / `chezmoi_shell`—程序激活时（对应环境变量已设）仅显示图标的指示段。
- `text`—给 `text "…"` 元素提供样式。

同一段名可以在 `segments` 里写多次，节点会合并：后出现的覆盖样式，`state` 和行为属性累积。

## defaults、separators、frame、vcs-remote-icons

- `defaults fg=… bg=… bold=#true`—所有**段**回退的终点（对应 p10k 段继承的全局 `POWERLEVEL9K_BACKGROUND`）：段没写 `bg` 就继承它，`defaults` 也没写 `bg` 时该段**透明**（终端自己的底色，早期版本会强制黑底）。帧字符和 `text` 元素只继承这里的 `fg` / `bold`，**不吃 `bg`**——p10k 的 `╭─` / `╰─` 在 classic 里也只有前景色、是透明的。所以统一底色的主题（classic）是**逐段写 `bg`**，wizard 之后新增的段（时间段等）会按当前风格补上同样的底色。
- `separators { segment … sub … end … left-tail … right-tail … right-start … right-segment … right-sub … gap … }`—powerline 箭头家族，值为字符串（如 `"\u{e0b0}"`）。节点上还可带三个颜色属性：`gap-foreground`（左右栏之间填充字符的前景，p10k 的 `MULTILINE_FIRST_PROMPT_GAP_FOREGROUND`）、`sub-foreground` / `right-sub-foreground`（**同底**细线的前景，p10k 的 `sep_color`；缺省跟后段前景色）。箭头本身不用这些属性：它按前后段底色渐变（powerline 风格）。
- `frame fg=… bg=… bold=#true { first-prefix … first-suffix … newline-prefix … newline-suffix … last-prefix … last-suffix … }`—每行行首/行尾的装饰：第一个 header 行用 `first-*`，后续 header 行用 `newline-*`，输入行用 `last-*`。帧级样式（`fg`/`bg`/`bold`）作用于所有块，单块可带自己的样式覆盖（如 `last-prefix "╰─" fg=196`）。输入前缀的宽度参与几何：引擎按它生成等宽占位符。
- `vcs-remote-icons { github="\u{f113}" … }`—远端域名子串 → 图标，按书写顺序匹配，未匹配用默认 git 图标。
- `dir-classes { class "~/work/**" state="WORK" icon="★" … }`—按顺序拿 `$PWD` 匹配，第一条命中的规则决定 dir 段的 state（用 `state WORK fg=…` 上色）与图标；`icon=""` 表示**不要图标**（同 p10k）。配了 `show-writable` 时 state 还会拼上 `_NOT_WRITABLE` / `_NON_EXISTENT` 后缀（p10k 规则）。模式方言与 p10k 的 zsh 扩展 glob 不同：`~` 展开成 $HOME、`*` `?` `[…]` 不跨 `/`、`**` 跨目录、模式以 `/` 结尾表示整棵子树。
- 不写 `dir-classes` 时，dir 图标走 p10k 自带的四条规则：`/etc` 及其下级 → `etc`、`$HOME` → `home`、它下面的任意目录 → `home-sub`、其余 → `folder`。它们就是普通图标表条目，可以用 `icon { home-sub { all "H" } }` 覆盖。

## 涉及配置的检查单

1. 名字用 kebab-case；颜色键用短的 `fg`/`bg`。
2. 启用用 flag 语义。
3. 布尔只写 `#true`/`#false`；转义只写 `\u{…}`。
4. 破坏性变更先讨论并提前说明。
5. 保持 KDL 原生语法，不贴近 powerlevel10k 的写法——这里没有可搬的东西。
6. 配置不做渲染决策，只声明样式。
