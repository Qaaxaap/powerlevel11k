# Glossary

This table fixes the terminology for Chinese documents, comments and user-facing text. Use it when you write, translate or edit. Add a term when the code gains one. Change a term here first, then search and replace across the repository.

The Fixed form column gives the form to use, and the forms to avoid. The Chinese column holds the term to write in Chinese text.

## Project and ecosystem

| English | Chinese | Meaning | Fixed form |
|---|---|---|---|
| powerlevel11k | powerlevel11k | Full project name | Write the full name at first mention, then p11k |
| p11k | p11k | Project and command name | Do not translate |
| powerlevel10k / p10k | powerlevel10k / p10k | Predecessor project | Do not translate |
| gitstatus | gitstatus | p10k's git status daemon. This project ships a compatible implementation | Do not translate |

## Architecture and protocol

| English | Chinese | Meaning | Fixed form |
|---|---|---|---|
| engine | 引擎 | Long-lived process that proxies the pty and draws the header | Do not write “engine process” |
| pty | 伪终端 | Pseudo terminal | Write “伪终端（pty）” at first mention in Chinese, then pty |
| transparent proxy | 透明代理 | The engine forwards bytes without parsing them | Do not write “proxy layer” |
| header | 头部 | The multi-row area above the prompt that the engine draws | Do not write “title” |
| prompt | 提示符 | The shell prompt and the moment it appears | Do not write “hint” |
| input line | 输入行 | The command line that the shell's line editor draws | Do not write “input box” |
| line editor | 行编辑器 | zle, readline or fish's reader | Do not write “editor” |
| announce | announce | The FIFO that tells the engine the prompt is ready | Write `announce` in Chinese text, with its role in brackets at first mention. Do not write “通知文件” |
| ack | ack | The file the engine returns to the shell, with the header row count and the prefix | Write `ack` in Chinese text, with “回执” in brackets at first mention. Do not write “回执文件” |
| state directory | 状态目录 | `$XDG_STATE_HOME/p11k/run-<pid>/` | Do not write “temp directory” |
| shell bootstrap | shell 启动引导 | The template that loads the rc files and starts the protocol | Do not write “startup script” |
| wizard | 向导 | The interactive flow of `p11k configure` | Do not write “configurator” |

## Theme and configuration

| English | Chinese | Meaning | Fixed form |
|---|---|---|---|
| theme | 主题 | One complete KDL configuration | Do not write “skin” |
| config | 配置 | The KDL file and its content | Do not write “settings” |
| layout | 布局 | The row and column arrangement of the header and the input line | Do not write “typesetting” |
| line | 行 | One `line` node in the layout | Do not write “column” |
| left column / right column | 左列 / 右列 | The two sides of the layout | Do not write “sidebar” |
| segment | 段 | A styled information unit in the header | Do not write “fragment” or “component” |
| icon | 图标 | The glyph before or after a segment | Do not write “symbol” |
| icon mode | 图标模式 | `nerdfont-complete`, `compatible` or `ascii` | Do not write “icon set” |
| preset | 预设 | A starting point: `lean`, `classic`, `rainbow` or `pure` | Do not write “template” |
| separator | 分隔符 | The powerline arrows and the gap glyphs | Do not write “splitter” |
| frame | 边框 | The decoration glyphs at the row edges | Do not write “box” |
| ruler | 标尺 | The full-width line above the header | Do not write “ruler bar” |
| fold | 折叠 | Collapsing the header into one line after a command is submitted | Do not write “collapse” |
| state | 状态 | An explicit state override for a segment, such as `ERROR` or `ANCHOR` | Do not write “condition” |
| defaults | 默认值 | The fallback layer of segment styles | Do not write “default” |
| transient prompt | 瞬态提示符 | The prompt shown after a command is submitted. The header folds into one line | Write “瞬态提示符（transient prompt）” at first mention in Chinese, then 瞬态提示符. Do not write “临时提示符” |
| instant prompt | 即时提示符 | The prompt on screen before the shell finishes loading its config. p10k has the same feature | Write “即时提示符（instant prompt）” at first mention in Chinese, then 即时提示符 |
| instant header | 即时头部 | Engine-internal name for the header and the input-line prefix drawn in the instant phase | Use it in code comments and implementation documents only. Do not use it for user-visible behaviour |
| prompt char | 提示符字符 | The glyph before the input line, `❯` by default | Do not write “prompt character” |
| right indent | 右侧缩进 | The empty columns kept to the right of the right column | Do not write “right margin” |

## git status

| English | Chinese | Meaning | Fixed form |
|---|---|---|---|
| vcs | 版本控制状态 | The `vcs` segment and the git state | Do not write “version control” |
| branch | 分支 | The current branch name | Do not write “tree” |
| staged | 已暂存 | Changes added to the index | Do not write “cached” |
| unstaged | 未暂存 | Changes in the worktree that are not in the index | Do not write “uncached” |
| untracked | 未跟踪 | Files that git does not track | Do not write “unregistered” |
| stash | 贮藏 | Changes saved by `git stash` | Do not write “暂存” in Chinese, to avoid a clash with staged |
| ahead / behind | 领先 / 落后 | Commit counts relative to the upstream branch | Do not write “before/after” |
| conflicted | 冲突 | Merge conflicts | Do not write “contradiction” |
| clean / dirty | 干净 / 脏 | Worktree state | Do not write “clean/polluted” |

## Documentation and releases

| English | Chinese | Meaning | Fixed form |
|---|---|---|---|
| CHANGELOG | 变更日志 | `CHANGELOG.md` | Keep the file name in English |
| release | 发布 | The release flow that a tag starts | Do not write “issue” |
| tag | 标签 | A `v*` version tag | Do not write “mark” |
| MSRV | 最低支持的 Rust 版本 | Pinned by `rust-toolchain.toml` | Write the full form at first mention |
| Conventional Commits | 约定式提交 | The commit title format | Do not write “regular commits” |
| msgid / msgstr | msgid / msgstr | The gettext source string and its translation | Do not translate |
| translation catalog | 翻译词条文件 | `po/*.po` | Do not write “translation library” |
| squash | 压缩合并 | Merging a pull request as one commit | Do not write “squeeze” |
| pull request / PR | 拉取请求 | The change submission flow | Write “拉取请求（PR）” at first mention in Chinese, then PR |
