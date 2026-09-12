# powerlevel11k (p11k)

[English](README.md)

用 Rust 重写的 powerlevel10k 提示符引擎。

powerlevel10k 在 2024 年进入纯维护模式：不再新增功能，多数 bug 不再修，
issue 无人回应，仓库也不会移交。它今天依然好用，但想要 v1.20.0 之后东西的
人没有去处。p11k 从这里接手：保住 p10k 的外观与速度，把整条链路换成 Rust。

它不再是传统的 shell 主题。引擎直接占住终端（pty 宿主 + 透明代理），在 pty
里跑一个未加载主题的 shell，字节原样透传，只在提示符出现的瞬间接管绘制。
同一套主题因此可以跑在 zsh、bash、fish 和 pwsh 上——这是 p10k 做不到的。

## 引擎原理

引擎是 pty 宿主兼透明代理：它在 pty 中启动一个未加载主题的 shell，原样转发
字节，仅在提示符出现的时刻接管绘制。

shell 钩子宣告提示符就绪后，引擎绘制主题头部，绘制完成即交还控制。输入行仍
由 shell 自己的行编辑器绘制，补全、历史、vi 模式的几何因此保持自洽。

几何由占位协议保证：shell 渲染的是由占位符构成的主题骨架，引擎在骨架渲染
完成后回填真实头部。引擎不解析 pty 输出中的 ANSI 序列，也不修改用户输入。

## 功能

- **多行头部**：p10k 风格，由 KDL v2 配置生成。
- **instant header**：终端打开即显示提示符，不等待 shell 加载完配置。
- **transient prompt**（仅 zsh）：命令提交后头部折叠为单行 `❯`，对应 p10k 的
  `TRANSIENT_PROMPT`。其它 shell 不支持。
- **vi_mode / history / 宽松布局**：vi 模式指示、历史命令号、
  `prompt-add-newline` 空行等，随配置启用。
- **80+ 内置段**：dir、vcs、status、time、command_execution_time、后台任务；
  语言版本（go/rust/node/php/java/dotnet/swift/…）；云（aws/azure/gcloud/kube/
  terraform）；网络（ip/vpn/wifi/public_ip）；系统（load/ram/swap/battery/disk）；
  环境指示器（virtualenv/pyenv/rvm/nvm/…）。与 p10k 的段基本一一对应。
- **git 状态**：仓库状态在后台线程计算，nixpkgs 规模的大仓库也不会阻塞提示符。
- **主题语言 KDL v2**：`layout` / `segments` / `defaults` / `separators` /
  `frame` / `vcs-remote-icons` / `mode` / `icon`，语法见
  [docs/config-language.zh.md](docs/config-language.zh.md)。视觉不硬编码在
  引擎里，换配置即换主题。
- **三套图标**：`nerdfont` / `compatible` / `ascii`，按实际字体选用；locale 非
  UTF-8 时自动降级为 `ascii`。顶层 `icon{}` 可按图标名逐档覆盖字符。

## 安装

Nix flake：

```bash
nix run github:Qaaxaap/powerlevel11k -- --shell zsh   # 不安装，直接运行
nix profile install github:Qaaxaap/powerlevel11k
```

作为 home-manager input：

```nix
inputs.p11k.url = "github:Qaaxaap/powerlevel11k";
home.packages = [ inputs.p11k.packages.${pkgs.system}.default ];
```

Debian / Ubuntu，从 [release](https://github.com/Qaaxaap/powerlevel11k/releases)
下载 `.deb`：

```bash
sudo apt install ./p11k_0.1.0_amd64.deb
```

其它发行版解压 tar 包。`bin/` 与 `share/` 可重定位：词条既在系统目录中查找，
也在可执行文件旁查找。

```bash
tar xzf p11k-0.1.0-x86_64-linux.tar.gz -C ~/.local
~/.local/p11k-0.1.0-x86_64-linux/bin/p11k --shell zsh
```

tar 包需要系统已安装 OpenSSL 3 运行库，常见发行版默认提供。

从源码构建并安装：

```bash
cargo install --path crates/p11k-engine
```

需要 Rust 工具链。`msgfmt`（gettext）用于生成翻译，缺失时构建仍然成功，界面
只是没有翻译。

以上构建均为 x86_64 Linux。

## 接入 shell

若原先使用其它 shell 主题，必须先移除：p11k 不是传统 shell 主题，两者会叠加
显示。

在 shell 的 rc 文件中加入一行：

```bash
# zsh
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell zsh
# bash
[[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell bash
# fish
if not set -q P11K_ENGINE; exec /path/to/p11k --shell fish; end
```

PowerShell 写入 `$PROFILE`：

```powershell
if (-not $env:P11K_ENGINE) { & '/path/to/p11k' --shell pwsh; exit }
```

必须显式给出 `--shell`：从 zsh 或 bash 启动 pwsh 时 `$SHELL` 不会改变，引擎
不使用其它线索判断。

引擎以 exec 覆盖启动它的 shell，并继承其环境；内层 shell 会重新加载用户配置
（主题除外）。`P11K_ENGINE` 用于防止递归启动。

## 主题配置

主题是 KDL v2 文件，语法见 [docs/config-language.zh.md](docs/config-language.zh.md)。

引擎默认读取 `$XDG_CONFIG_HOME/p11k/p11k.kdl`（通常是 `~/.config/p11k/p11k.kdl`），
该文件不存在时使用内置的 lean 主题；`p11k configure` 向导会把配置写到那里。
`--config <path>` 可指定其它文件，`--preset <name>` 可选用内置主题
（lean / classic / rainbow / pure）。

`p11k reload` 让正在运行的会话重新读取主题文件，无需重开 shell：

```bash
$EDITOR ~/.config/p11k/p11k.kdl
p11k reload
```

引擎重绘头部并重启 git worker，`vcs` 相关属性同样生效。文件解析失败时由该
命令报错，当前主题保持不变。输入行的宽度在引擎启动时固定，改变了宽度的
`prompt_char` 会被忽略并在引擎日志中记录；颜色、布局、段、分隔符与边框立即
生效。

## 界面语言

界面文案支持中文与英文，更多翻译欢迎您贡献。按标准环境变量（`LANG`、`LC_ALL`、`LANGUAGE`）选择，
默认英文：

```bash
LANG=zh_CN.UTF-8 p11k --shell zsh   # 中文界面
p11k --shell zsh                    # 英文（默认）
```

`P11K_LOCALEDIR` 可指定词条目录，通过安装包或 flake 安装时无需设置。

## compat/p10k：替换 gitstatusd

`compat/p10k` 分支把原版 p10k 主题 vendor 进来，只替换内核：`p11k-d` 按 gitstatusd 线上协议逐字节应答，可直接作为原版 `gitstatus.plugin.zsh` 的后端。

```bash
export GITSTATUS_DAEMON=/path/to/p11k-d
```

配置、外观、status 与 vcs 与 p10k 完全一致。

该分支不是主要开发方向，主线是 `main` 上的引擎，但仍会保证维护。

## 状态与路线

已完成：

- gitstatusd 兼容内核（v1.5.5 协议）与性能核心：index 解析、`fstatat` 扫描、
  并行分片、untracked cache
- 占位协议：多行头部几何自洽，覆盖 zsh / bash / fish / pwsh
- instant / transient prompt、vi_mode、history、宽松布局
- 80+ 段、KDL 主题语言、三档图标与 `icon{}` 覆盖
- `p11k configure` 向导、gettext 词条（zh_CN）、CI
- `p11k reload`；p10k 功能已对齐至扩展状态（`OK_PIPE` / `ERROR_PIPE` /
  `ERROR_SIGNAL`）与 `VCS_DISABLED_WORKDIR_PATTERN`
- 发布产物：Nix flake、`.deb` 与 tar 包

待办：

- p10k 尚未实现的开关，均只在非默认取值下才有差异：`ICON_PADDING=moderate`、
  显式的 `ICON_BEFORE_CONTENT` 覆盖、`TRANSIENT_PROMPT=same-dir`
  （`transient-prompt #true` 等价于 p10k 的 `always`）、`LEGACY_ICON_SPACING`。
  按 p10k 默认配置渲染时已经一致。`DIR_MAX_LENGTH` 与
  `DIR_MIN_COMMAND_COLUMNS(_PCT)` 无法实现：它们需要读取 zle 的 buffer，而
  引擎看不到。
- macOS 支持
- daemon 与引擎的长期维护和加固

## 参与开发

构建与测试环境、提交规范、翻译流程与发布流程见
[CONTRIBUTING.zh.md](CONTRIBUTING.zh.md)。

## 许可证

LGPL-3.0-or-later，见 [LICENSE](LICENSE)。

`compat/p10k` 分支 vendor 的 powerlevel10k 主题为 MIT（版权归 Roman
Perepelitsa 及贡献者，原版权声明保留在
`compat/p10k/vendor/powerlevel10k/LICENSE`）。
