# 视觉渲染检查

[English](README.md)

渲染检查在真实的 kitty 终端中运行 `p11k`。终端运行在无头 sway 合成器中。检查把捕获的像素与已提交的参考帧比较。代理测试回答另一个问题。代理测试问“引擎写了哪些字节”。本检查问“用户看到了什么”。

工具链位于 `visual` 包。默认 devShell 不含它。sway、kitty 和 grim 体积大。只有本检查需要它们。本检查也不属于 `just verify`。

## 运行

```sh
# Capture 16 frames (4 shells × 3 widths + dirty scene) and compare them.
nix develop --command just render-check [-j N]

# After an intended rendering change: capture new frames into expected-pending/.
nix develop --command just render-update [-j N]

# After a person reviews each pending image against the intended layout:
nix develop --command just render-accept
```

`-j/--jobs N` 设置 bats 并行度。默认值为 4。`just render-update` 从不改动 `expected/`。它写入 `tools/visual/expected-pending/<arch>/`，git 忽略该目录。`just render-accept` 把已审查的待审图复制到 `tools/visual/expected/`，并删除待审目录。只有审查之后，帧才成为参考帧。

四个 shell 对应 `tools/visual/tests` 下的四个 bats 测试文件。每个测试捕获一帧，打印一行 TAP 结果（`ok 1 bash reference frame at 700x450`）。测试失败时显示捕获日志。每次运行的完整日志在 `.cache/render-check/<frame>/capture.log`。使用 `-j 4` 时，16 帧约需 25 秒。比较约需几百毫秒。

捕获图和差异图写入 `.cache/render-check/`：

```text
.cache/render-check/
  zsh-700x450/p11k-zsh-700x450-000.png
  zsh-560x450/p11k-zsh-560x450-000.png
  zsh-900x450/p11k-zsh-900x450-000.png
  zsh-dirty/p11k-zsh-dirty-000.png
  diff-<frame>.png        # only for a failed comparison
```

## 帧

每个 shell 有四帧：

| 帧 | 宽度 | 场景 |
| --- | --- | --- |
| `<shell>-700x450.png` | 700 px | 干净仓库（`reference`） |
| `<shell>-560x450.png` | 560 px | 干净仓库（窄） |
| `<shell>-900x450.png` | 900 px | 干净仓库（宽） |
| `<shell>-dirty.png` | 800 px | 带 stash 和未提交计数的仓库 |

干净场景在 `/tmp/p11k-visual-repo` 的固定仓库中运行。该仓库的 `main` 分支保持干净。脏场景在 `/tmp/p11k-visual-repo-dirty` 中运行。该仓库有一个 stash、一个已暂存改动、一个未暂存改动和一个未跟踪文件。`prepare.sh` 一次性创建两个仓库、固定的 HOME 和启动脚本。此步骤在并行测试开始前执行。

每帧是一张由人审查过的截图。比较忽略抗锯齿像素和 1% 及以下的颜色差异。布局、文字、颜色或位置的差异大于该阈值，检查失败。

四个 shell 绘制相同的头部。它们的输入行、回显和重绘行为不同。因此共享帧会让未生成该帧的 shell 失败。正确提示符的截图是基线。项目没有手写的期望渲染。

检查比较解码后的像素。比较 PNG 文件还会比较编码器和元数据。这些项目不影响屏幕。`compare.sh` 运行 `odiff --aa --threshold 0.01`。比较忽略抗锯齿像素和小颜色差异。它报告不同像素的数量。

参考帧按架构存放（`expected/x86_64-linux/`、`expected/aarch64-linux/`）。字体光栅在不同架构之间可能不同。

## 确定性

检查固定每个可能移动像素的输入：

- 无头 sway 只有一个输出，帧尺寸固定。XKB 布局和 locale 固定。
- kitty 来自同一个 Nix closure。`tools/visual/kitty.conf` 固定字体系列、字号、光标形状、光标闪烁、内边距和终端颜色。
- Maple Mono NF 作为字体系列。wrapper 通过 `FONTCONFIG_FILE` 提供它。
- 上述两个固定仓库和固定 HOME。HOME 中没有用户 rc 文件。
- `tools/visual/reference.kdl`。没有段会自行变化（时间、执行时间、后台任务）。没有段依赖机器（OS 图标）。
- `kitty.conf` 用背景色绘制光标。因此帧不依赖终端是否绘制光标，也不依赖闪烁相位。
- 检查在两次相同的 grim 捕获之后截取帧。它从不使用固定 sleep。helper 为 zsh、bash 和 fish 等待 `--min-wait 0.5`。因为 pwsh 启动更慢，helper 为它等待 `--min-wait 1.5`。命令结束后，helper 还等待 `--type-wait 0.2`。输入回显在稳定性检查开始前已完成。
- `record-tui.sh` 固定 `WLR_RENDERER=pixman`。软件渲染器是确定性的，不需要 GPU。因此本地运行和 CI 运行产生相同像素。GPU（gles2）帧可能有一个像素的差异。gles2 只用于手动实验，不要用它的帧作为参考。

修改 kitty、字体或渲染器时，必须在同一个提交中生成新参考帧。

## 添加 shell 或场景

每个 shell 在 `tools/visual/tests` 中有一个 bats 文件。一个测试通过 `helper.bash` 捕获一帧。`helper.bash` 调用 `record-tui.sh` 和 `compare.sh`。

`prepare.sh` 一次性创建共享仓库、HOME 和启动脚本。此步骤在测试并行运行前执行。新场景需要在 `prepare.sh` 中增加新主题或新仓库状态，并为每个 shell 增加一个测试。保持参考帧集合小且易于审查。

## CI

`.github/workflows/ci.yml` 中的 `render` job 在 x86_64 上运行检查。该 job 不在 `just verify` 中运行。visual 工具链的 devShell 体积大，参考帧只存在于 x86_64。`changes` job 在仅文档改动时跳过该 job。其它 job 使用相同规则。

该 job 有意共享 x86_64 缓存键。两个 job 需要相同的 flake 输入和相同的 cargo 构建目录。为每个 job 使用第二个键会存储第二份副本。Nix store 缓存上限为 3G。该上限为 visual devShell 留出空间。运行失败时会上传 `.cache/render-check/`（捕获图和差异图）作为工作流产物。
