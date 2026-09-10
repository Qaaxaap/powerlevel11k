# powerlevel11k (p11k)

用 Rust 延续 powerlevel10k 生命的提示符。

架构天然跨 shell。当前支持 bash/fish/zsh。

powerlevel10k 于 2024 年进入纯维护模式：不新增功能、多数 bug 不再修、求助被
忽略、仓库不会移交。它今天依然好用，但想要 v1.20.0 之外东西的人没有去处。
p11k 从这里接手——保持 p10k 的外观与"快"，但全链路换成 Rust。

项目目前的两个主要分支：

- **`main`：完整主题引擎。** 自行渲染，不依赖 shell 主题，为任意 shell 以简单可控的方式渲染提示符。
- **`compat/p10k`：p10k 即插即用后端。** vendor 原版 p10k 主题，只把内核
  `gitstatusd` 换成 Rust 的 `p11k-d`，配置、外观、vcs 一切照旧。

## powerlevel11k 引擎

引擎是一个 pty 宿主与透明代理，在 pty 里跑一个无主题 shell，字节原样
透传，只在 prompt 时刻短暂接管。

shell 钩子宣告 prompt 后，引擎绘制主题 header，画完交还。输入行由 shell 自己的行
编辑器画，补全、历史、vi 模式的几何自洽。

几何靠"占位协议"保证：让 shell 渲染由无意义占位符组成的主题，引擎等占位渲染完再
回填真实 header。引擎不解析 pty 输出里的 ANSI，也不碰用户输入的内容。

### 能力

如果需要一句话形容这个项目的价值，我会说

> 它让 powerlevel10k 走向了 bash/fish 乃至任意shell

- 多行主题 header：p10k 样式，按 KDL v2 配置生成，配置文件人类友好。
- **instant header**：打开终端立刻见 prompt，不等 shell 慢悠悠加载配置。
- **transient prompt**（zsh）：提交命令后 header 折叠成单行 `❯`（对应 p10k
  `TRANSIENT_PROMPT`）；bash/fish 没有 zle，此功能不生效。
- **vi_mode / history / 宽松布局**：vi 模式指示、历史命令号、`prompt-add-newline`
  空行等，跟随配置开合。
- **80+ 内置段**：dir、vcs、status、time、command_execution_time、后台任务、
  语言版本（go/rust/node/php/java/dotnet/swift/…）、云（aws/azure/gcloud/kube/
  terraform）、网络（ip/vpn/wifi/public_ip）、系统（load/ram/swap/battery/disk）、
  环境指示器（virtualenv/pyenv/rvm/nvm/…）……与 p10k 的段几乎一一对齐。
- **git 状态**：Rust 实现的 gitstatus 内核（index 解析、`fstatat` 扫描、并行
  分片、untracked cache）在后台线程算，nixpkgs 级大仓库也不卡 prompt。
- 主题用 **KDL v2** 表达（[docs/config-language.md](docs/config-language.md)）：
  `layout` / `segments` / `defaults` / `separators` / `frame` /
  `vcs-remote-icons` / `mode` / `icon`。换配置即换主题，引擎不硬编码视觉。
- **图标/字体模式** 提供三套图标：`nerdfont` / `compatible` / `ascii`，可根据实际字体选用。
  locale 非 UTF-8（控制台/C locale）时自动降级
  `ascii`，避免任何非 ASCII 都渲染成方块。顶层 `icon{}` 可按**图标名**逐档
  覆盖字符。


### 快速开始

**请清除掉原本的 shell 主题，再行加载 p11k**

p11k 并非传统的 shell 主题，如果不清除原有 shell 主题则会叠加显示。

```bash
cargo build -p p11k-engine

# 开发：直接启动主题引擎（让 P11K_USER_ZSHRC 指向一个去掉了主题的配置副本）。
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/zsh>

# 带主题文件（缺省用内置 lean 主题）。
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

当"主题"用：在 shell 的 rc 里加一行 exec：

```bash
# zsh
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell zsh
# bash
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell bash
# fish
if not set -q P11K_ENGINE; exec /path/to/p11k --shell fish; end
```

引擎 exec 覆盖启动 shell，继承其环境；内部 shell 会重新 source 用户配置
（主题本身除外——引擎就是主题）。`P11K_ENGINE` 用于打破递归。

### 语言

引擎本身和 `p11k configure` 的用户可见文案走 gettext。英文是源语言：代码里的
msgid 就是英文，找不到翻译时原样输出。翻译放在
`crates/p11k-engine/po/<lang>.po`，由 `build.rs` 用 `msgfmt` 编成 `.mo`
（没有 `msgfmt` 时只警告不报错，编出的程序就是英文的）。语言按标准环境变量取
（`LANG`、`LC_ALL`、`LANGUAGE`）：

```bash
LANG=zh_CN.UTF-8 target/debug/p11k --shell zsh   # 中文界面
target/debug/p11k --shell zsh                    # 英文（默认）
```

`P11K_LOCALEDIR` 可覆盖翻译目录（缺省用构建期写进二进制的那个），例如装到系统
后用 `P11K_LOCALEDIR=/usr/share/locale`。加一门语言就是加一个文件再重新构建：
把 `po/zh_CN.po` 复制成 `po/<lang>.po`，改 `msgstr`，`cargo build -p p11k-engine`。
词条与代码的同步由 `tools/i18n.sh extract|update` 负责，源码、`po/p11k.pot`、
各语言词条三者只要对不上，单元测试就会失败。

## compat/p10k：gitstatusd 即插即用

p10k 的 zsh 渲染是十年打磨，不该重写。`compat/p10k` 分支 vendor 原版 p10k
主题，只替换内核：`p11k-d`（Rust 守护进程）按 gitstatusd 线上协议逐字节
应答，给原版 `gitstatus.plugin.zsh` 当后端：

```zsh
export GITSTATUS_DAEMON=/path/to/p11k-d
```

配置、外观、status、vcs 与 p10k 完全一致。热 git 请求在 nixpkgs 级仓库
（54k 文件 / 38k 目录，22 核）约 100ms，C++ 原版约 65ms（并行扫描修复前
约 320ms）。

## 状态与路线

已完成：

- gitstatusd 兼容内核（v1.5.5 协议）与性能核心：index 解析、`fstatat`
  扫描、并行分片、untracked cache
- 引擎占位协议：多行 header 几何自洽，zsh/bash/fish 三 shell
- instant / transient prompt、vi_mode、history、宽松布局
- 80+ 段与 KDL 主题语言、图标三档 mode 与 `icon{}` 覆盖

待办：

- `p10k configure` 式配置向导
- 剩余段的补齐与框架集成（oh-my-zsh、prezto、zinit）
- daemon / 引擎的长期维护与加固（这正是 p11k 存在的意义）

## 许可证

LGPL-3.0-or-later，见 [LICENSE](LICENSE)。

`compat/p10k` 分支 vendor 的 powerlevel10k 主题为 MIT（版权归 Roman
Perepelitsa 及贡献者，原版权声明保留在
`compat/p10k/vendor/powerlevel10k/LICENSE`）。
