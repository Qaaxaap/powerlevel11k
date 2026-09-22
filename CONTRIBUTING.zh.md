# 为 powerlevel11k 做贡献

[English](CONTRIBUTING.md)

感谢关注。

本项目处于早期开发阶段。以下规则刻意保持精简，将随代码库一同演进。

## 开发环境

构建与测试所需的一切均固定在 `flake.nix`：

```bash
nix develop
cargo test --all
nix build          # 生成两个二进制：p11k 与 p11k-d
```

devShell 会导出 `LOCALE_ARCHIVE`；缺少该变量时 `setlocale("zh_CN.UTF-8")` 失败，i18n 测试将被跳过。CI 执行的即为上述 `nix develop` 命令，不存在第二套需要同步的环境；此外另有一项检查，确认 Nix 包仍可构建且包含词条。

以开发方式启动引擎：

```bash
P11K_USER_ZSHRC=/path/to/your/rc target/debug/p11k --shell <bash/fish/pwsh/zsh>
target/debug/p11k --shell zsh --config path/to/theme.kdl
```

## 提交约定

- 采用[约定式提交](https://www.conventionalcommits.org/zh-hans/v1.0.0/)（`feat:`、`fix:`、`chore:`、`docs:`、`refactor:`、`test:`、`perf:` 等），破坏性变更以 `!` 标记。
- 提交信息使用英文。
- 一个提交只承载一项逻辑变更，无关改动不得合并提交。混有重构与功能的提交将被要求拆分。
- 标题不超过 72 字符，且能接在"应用此提交后将会……"之后；正文说明 *原因*，不复述 diff 内容。

## 代码风格

- 每次提交前执行 `cargo fmt --all`，CI 会检查格式。
- `cargo clippy --all-targets --locked -- -D warnings` 与 `cargo test --all --locked` 必须通过。
- 不得使用 `unsafe`，除非有注释说明理由，并有测试或基准作为支撑。git 状态核心因直接访问 `fstatat`/`MetadataExt` 可能需要 `unsafe`；该处为明确许可的例外，不构成其余代码的先例。
- 公开 API 须有文档注释。gitstatus 协议代码必须在解析代码旁写明线上格式：与原版守护进程的字节级兼容是硬性要求。

## 配置语言

主题配置语言有独立的规范文档，与引擎同步维护：[docs/config-language.zh.md](docs/config-language.zh.md)。改动配置解析或渲染时须遵循该规范。

## 演示图片

`docs/images/` 里的图是生成的。`tools/demo/capture.sh` 先建一个小 git 仓库（暂存、未暂存、未跟踪、stash 各一处），再在固定尺寸的 pty 里运行引擎，把终端本来会显示的内容留下来；静图取自 tmux 的 pane dump，因为 fish 启动时会向终端发问询，裸 pty 不会应答。按键在 `tools/demo/scenes/demo.json`，主题在 `tools/demo/demo.kdl`，presets 图用的四份文件在 `tools/demo/presets/`。

重新录制需要已构建的 `p11k`（`cargo build --release -p p11k-engine`），以及 zsh、bash、fish、python3、tmux、agg（asciinema/agg）、ImageMagick 和 Maple Mono Nerd Font。

## 文案翻译

用户可见文案使用 gettext：代码中写英文 msgid，词条位于 `crates/p11k-engine/po/`。新增、删除或改写用户可见文案时：

```bash
tools/i18n.sh extract   # xgettext → po/p11k.pot
tools/i18n.sh update    # msgmerge 将新条目并入每个 po/*.po
```

随后补全新条目的 `msgstr`。以下三条规则由测试保证：

- 就地翻译的文案使用 `t("…")`；先收集、稍后统一翻译的文案（wizard 的问题标题与选项）须以 `msgid("…")` 包裹，以便提取器识别。两者都必须为纯字符串字面量。
- 源码、`po/p11k.pot` 与各语言词条三者中任意一处不一致，`cargo test -p p11k-engine` 即失败。遗漏 extract/update 或残留过时条目均无法通过测试。
- 新增语言：将 `po/zh_CN.po` 复制为 `po/<lang>.po`，翻译 `msgstr` 后重新构建。Nix 包会编入 `po/` 下的全部语言，无需改动 `flake.nix`。

## AI 政策

本仓库积极拥抱 AI 潮流，但您需遵守如下约定，否则可能会被处以限制或禁止向本仓库提交内容。

- 开发者需理解并为所提交的每一行代码负责。无意义提交或有害提交不被允许。
- 即使您理解代码，但仍不建议在无人监督的情况下使用生成式 AI 进行开发。有明显漏洞或与声明不符的提交将被拒绝。

## 发布

发布由 tag 触发：`v*` 形式的 tag 会构建 `p11k` 与 `p11k-d`，并将 tar 包与 `.deb` 附到该 tag 的 GitHub Release。

tag 的版本号必须与 `Cargo.toml` 中 `[workspace.package] version` 一致；workflow 会校验，不一致时失败。

## 报告问题

报告需具体。一份合格的报告包含：操作系统与 shell 版本（`zsh --version`）、p11k 版本、终端模拟器，以及能复现问题的最小配置。配置类提问将转至 Discussions。

## Pull Request 流程

1. Fork、创建分支，并按上述约定提交。
2. 一个 PR 只承载一项变更，描述中说明动机。
3. 提交前须在本地至少测试一次。无法编译或无法运行的补丁会浪费审查时间；反复提交低质量补丁可能导致贡献受限。
4. 遵守上述 [AI 政策](#ai-政策)。
5. 提交 PR 即表示同意贡献以 LGPL-3.0-or-later 授权。

## 许可证

本仓库全部内容（含贡献）均为 LGPL-3.0-or-later。
