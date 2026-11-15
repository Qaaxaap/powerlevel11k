# 为 powerlevel11k 做贡献

[English](CONTRIBUTING.md)

感谢关注。

本项目处于早期开发阶段。以下规则刻意保持精简，将随代码库一同演进。

## 开发环境

构建与测试所需的一切固定在 `flake.nix` 里。demo 录制需要额外工具，见[演示图片](#演示图片)。进入 devShell 后使用 `justfile` 的配方，`just` 不带参数会列出全部配方。

```bash
nix develop
just check         # fmt-check + clippy + test：快速门
just nix-check     # 同样的测试，跑在 nix 沙箱里
just build         # 生成两个二进制：p11k 与 p11k-d
```

CI 在 x86_64 与 aarch64 上执行 `just verify`，不存在第二套需要同步的命令行。`just check` 是更快的前置门，不含 `deny` 与 `spell`。MSRV job 执行 `cargo check --all-targets --locked`。其余配方覆盖基础设施的其它部分：

- `just deny`：依赖许可、禁用与安全通告
- `just spell`：拼写检查
- `just changelog`：由提交历史重新生成 `CHANGELOG.md`
- `just demo`：重新录制 `docs/images/`

devShell 会导出 `LOCALE_ARCHIVE`，其中包含测试所需的 C.UTF-8 与 it_IT 等 locale。

以开发方式启动引擎：

```bash
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/pwsh/zsh>
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

## 基准测试

两个 harness 都测 `p11k-gitstatus::RepoCache::build_fields`，每次 precmd 都会调它。

```bash
just bench       # 墙钟毫秒，criterion
just bench-iai   # 指令数，Callgrind
```

`just bench` 输出毫秒。引用这个数字时要连同机器一起写，因为负载、频率与缓存压力都会体现在墙钟时间里。

`just bench-iai` 输出指令数。同一份二进制重复跑，两个 50 文件的 case 逐位一致，5000 文件的 case 差 0.03%（`readdir` 顺序取决于文件系统）。判断改动快慢以指令数为准，差异小于 0.1% 视为噪声。

两个 harness 都在临时目录里自建 fixture，不依赖环境变量，也不需要仓库之外的目录。CI 通过 `cargo clippy --all-targets` 编译它们，但不运行，因为计时需要在安静机器上做。

devShell 提供 Valgrind 与 `iai-callgrind-runner`。不在 Nix 环境里时，自己安装：

```bash
cargo install iai-callgrind-runner --version 0.16.1   # 必须与 Cargo.toml 里的 iai-callgrind 一致
# 以及发行版提供的 valgrind 包
```

## 提交约定

- 采用[约定式提交](https://www.conventionalcommits.org/zh-hans/v1.0.0/)（`feat:`、`fix:`、`chore:`、`docs:`、`refactor:`、`test:`、`perf:` 等），破坏性变更以 `!` 标记。
- 提交信息使用英文。
- 一个提交只承载一项逻辑变更，无关改动不要合并提交。混有重构与功能的提交将被要求拆分。
- 标题不超过 72 字符，且能接在“应用此提交后将会……”之后。正文说明 *原因*，不复述 diff 内容。

## 代码风格

- 每次提交前执行 `just fmt`。`just check` 会一并检查格式、lint 与测试。
- 不要使用 `unsafe`，除非有注释说明理由，并有测试或基准作为支撑。git 状态核心因直接访问 `fstatat`/`MetadataExt` 可能需要 `unsafe`。该处为明确许可的例外，不构成其余代码的先例。
- 公开 API 须有文档注释。gitstatus 协议代码必须在解析代码旁写明线上格式。与原版守护进程的字节级兼容是硬性要求。

## 配置语言

主题配置语言有独立的规范文档，与引擎同步维护：[docs/config-language.zh.md](docs/config-language.zh.md)。改动配置解析或渲染时须遵循该规范。

## 演示图片

`docs/images/` 里的图由 `tools/demo/capture.sh` 生成。脚本先建一个小 git 仓库（暂存、未暂存、未跟踪、stash 各一处）。然后在固定尺寸的 pty 里运行引擎，把终端本来会显示的内容留下来。

静图取自 tmux 的 pane dump，因为 fish 启动时会向终端发问询，裸 pty 不会应答。

按键在 `tools/demo/scenes/demo.json`，主题在 `tools/demo/demo.kdl`，presets 图用的四份文件在 `tools/demo/presets/`。

重新录制需要已构建的 `p11k`（`cargo build --release -p p11k-engine`），以及 devShell 提供的 zsh、bash、fish、python3、tmux。`just demo` 通过 `nix shell` 拉取 agg、ImageMagick 和 fontconfig。图像使用 Maple Mono Nerd Font；`fc-match` 找不到字体时，用 `FONT_FILE` 指定另一个字体文件。

重新录制后，逐项检查四件事。字体是目标字体。图标没有缺字。zsh、bash、fish 三个 pane 都正常。布局在至少两种宽度下正常。

## 文案翻译

英文是仓库的工作语言，代码、注释、提交信息和文档用英文。中文版本由维护者维护，与英文同步。术语见 [docs/glossary.md](docs/glossary.md)。

作者不是英语母语者，英文翻译可能不准确。英文与代码行为不一致时以代码为准，并在 PR 或 issue 中指出。

用户可见文案使用 Fluent：消息 id 在代码中，中英文词条位于 `crates/p11k-engine/locales/`。新增、删除或改写用户可见文案时：

1. 在 `locales/en.ftl` 中新增或修改消息。id 保持稳定，值为英文原文。按原有风格书写：小写开头、结尾不加句号、细节放在冒号后。
2. 在 `locales/zh-CN.ftl` 中为同一个 id 补上中文。
3. 在调用点使用 id：`t("wizard-yes")`。带变量的消息由调用点传值：`t_args("cli-unknown-shell", &[("shell", name)])`；`.ftl` 中用 `{ $shell }` 放置变量。
4. `cargo test -p p11k-engine` 会检查两个词条文件的 id 一致、源码引用的 id 都已定义、没有未使用的消息。
5. 新增语言：创建 `locales/<lang>.ftl` 并包含相同 id。语言按 `P11K_LANG`、`LC_ALL`、`LC_MESSAGES`、`LANG`、`LANGUAGE` 的顺序选择，英文为回退语言。`P11K_LOCALEDIR` 可指向包含 `.ftl` 文件的目录，用于在不重新构建时测试词条。

词条会在构建时编入二进制：发布产物不需要额外的数据文件或翻译工具。

## AI 政策

本仓库接受 AI 辅助开发，但需遵守以下约定。违反可能导致限制或禁止向本仓库提交内容。

- 开发者需理解并为所提交的每一行代码负责。无意义或有害的提交不被接受。
- 即使理解代码，也不建议在无人监督的情况下用生成式 AI 开发。有明显漏洞或与声明不符的提交将被拒绝。
- 使用 AI 辅助的提交，由提交者负责验证。未经运行、或无法说明验证方式的内容不要提交。

## 发布

发布由 tag 触发：`v*` 形式的 tag 会构建 `p11k` 与 `p11k-d`，并将 tar 包与 `.deb` 附到该 tag 的 GitHub Release。

打 tag 前先执行 `just changelog v<version>` 并提交结果。`CHANGELOG.md` 由约定式提交历史生成。不传 tag 参数时，条目保留在 `Unreleased` 段。

tag 的版本号必须与 `Cargo.toml` 中 `[workspace.package] version` 一致。workflow 会校验，不一致时失败。

## 报告问题

报告需具体。一份合格的报告包含：操作系统与 shell 版本（`zsh --version`）、p11k 版本、终端模拟器，以及能复现问题的最小配置。配置类提问将转至 Discussions。

## Pull Request 流程

1. Fork、创建分支，并按上述约定提交。
2. 一个 PR 只承载一项变更，描述中说明动机。
3. 提交前须在本地至少测试一次。无法编译或无法运行的补丁会浪费审查时间。反复提交低质量补丁可能导致贡献受限。
4. 在描述里写验证证据：实际运行的命令与输出摘要。无法运行的检查要说明原因，并写出维护者本地验证的步骤。
5. 描述和提交信息只引用仓库里的内容：项目命令、仓库文件和公开链接。不要提到 `.cache/` 草稿、本地脚本或仓库外的计划。
6. 改动 `docs/images/` 或它的生成器时，先跑生成器的前置检查，再做一次人眼验收。在描述里记录验收结论。
7. 维护者可以直接关闭不满足上述要求的 PR，或与声明不符的 PR，不进入 review。
8. 遵守上述 [AI 政策](#ai-政策)。
9. 提交 PR 即表示同意贡献以 LGPL-3.0-or-later 授权。

## 许可证

本仓库全部内容（含贡献）均为 LGPL-3.0-or-later。
