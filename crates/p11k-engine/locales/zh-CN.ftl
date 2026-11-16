# p11k — 简体中文。
#
# 风格名（Lean/Classic/Rainbow/Pure/Original/Snazzy/Unicode/ASCII）保持英文，
# p10k 用户对这些名字的认知是英文的。
# 变量（{ $name }）由调用点传入，位置按中文语序安排。
# 标点一律全角；完整句子以「。」结尾，标题与选项不带句末标点。

# ===== 命令行与配置 =====
cli-cannot-read-config = 读取配置失败，回退到内置 lean 主题：{ $detail }
cli-unknown-preset = 未知预设 { $preset }，回退到内置 lean 主题（可为 lean/classic/rainbow/pure）
cli-not-in-session-unset = 不在 p11k 会话中（P11K_ENGINE_PID 未设置）
cli-invalid-engine-pid = 无效的 P11K_ENGINE_PID（必须是正整数）
cli-not-in-session-missing = 不在 p11k 会话中（会话数据缺失，请重启会话）
cli-cannot-write-reload-request = 无法写入 reload 请求：{ $detail }
cli-cannot-remove-reload-request = 无法删除 reload 请求：{ $detail }
cli-cannot-signal-engine = 无法向引擎发送信号（会话可能已退出）
cli-cannot-read-session-file = 无法读取会话文件：{ $detail }
cli-token-mismatch = 会话令牌不匹配，该进程号可能属于另一个进程
cli-no-theme-file = 本会话没有主题文件：启动时用的是 --preset 或内置主题
cli-unknown-shell = 配置里的 shell { $shell } 无法识别，回退到 $SHELL（可为 zsh/bash/fish/pwsh）
config-invalid-hex-colour = { $key }：十六进制颜色需要六位数字：{ $value }
config-unknown-colour-name = { $key }：未知颜色名：{ $name }；{ $help }
config-colour-help = 请使用颜色名、0-255、#rrggbb 或 default
config-colour-index-out-of-range = { $key }：颜色索引超出 0-255：{ $index }
config-expected-colour = { $key }：需要一个颜色（0-255、#rrggbb、颜色名或 default）
config-kdl-parse-error = KDL 解析失败：{ $detail }
config-state-needs-name = `state` 需要一个名字（位置参数）
config-element-needs-text = { $element } 需要文本（位置参数）
config-unknown-key = 未知配置键：{ $key }
config-mode-fallback = "{ $declared }" 无法在当前 locale 显示，回退到 "{ $effective }"
config-glyph-fallback = { $field } "{ $glyph }"：无法显示，使用内置默认值
config-unrenderable-glyphs = 当前 locale 在 "{ $mode }" 模式下存在无法显示的字形：{ $glyphs }
config-cannot-render = 无法渲染主题：{ $error }

# ===== 引擎 =====
engine-inner-shell-failed = 内部 shell 启动失败
main-recursive-launch = 检测到递归启动（在 p11k 会话内再次启动 p11k）
main-bootstrap-hint = 请确认 shell rc 中的引导行带有保护条件，例如：
main-cannot-exec-shell = 无法启动 shell：{ $detail }

# ===== 预设与向导选项 =====
preset-lean = Lean
preset-classic = Classic
preset-rainbow = Rainbow
preset-pure = Pure
preset-original = Original
preset-snazzy = Snazzy
wizard-yes = 是
wizard-no = 否
wizard-restart = 重新开始
wizard-quit = 退出
wizard-font-diamond = 看起来像菱形（旋转的正方形）吗？
wizard-font-lock = 看起来像锁吗？
wizard-font-angle = 看起来像 >< 但更高更粗吗？
wizard-style-title = 提示符风格
wizard-shade-lightest = 最浅
wizard-shade-light = 浅
wizard-shade-dark = 深
wizard-shade-darkest = 最深
wizard-colours-title = 提示符配色
wizard-colours-256 = 256 色
wizard-colours-8 = 8 色
wizard-colour-title = 提示符颜色
wizard-frame-colour-title = 边框颜色
wizard-charset-title = 字符集
wizard-charset-unicode = Unicode
wizard-charset-ascii = ASCII
wizard-rprompt-title = 非永久内容的位置
wizard-side-left = 左侧
wizard-side-right = 右侧
wizard-time-title = 显示当前时间？
wizard-time-12h = 12 小时制
wizard-time-24h = 24 小时制
wizard-separators-title = 提示符分隔符
wizard-separator-angled = 尖角
wizard-separator-vertical = 竖线
wizard-heads-title = 提示符首端符
wizard-tails-title = 提示符尾端符
wizard-shape-flat = 扁平
wizard-shape-blurred = 渐隐
wizard-shape-sharp = 尖角
wizard-shape-slanted = 斜角
wizard-shape-round = 圆角
wizard-height-title = 提示符高度
wizard-height-one-line = 一行
wizard-height-two-lines = 两行
wizard-connection-title = 提示符连接线
wizard-connection-disconnected = 断开
wizard-connection-dotted = 点线
wizard-connection-solid = 实线
wizard-frame-title = 提示符边框
wizard-frame-none = 无边框
wizard-frame-full = 完整
wizard-spacing-title = 提示符间距
wizard-spacing-compact = 紧凑
wizard-spacing-sparse = 稀疏
wizard-icons-title = 图标
wizard-icons-few = 少量图标
wizard-icons-many = 较多图标
wizard-flow-title = 提示符措辞
wizard-flow-concise = 简洁
wizard-flow-fluent = 流畅
wizard-transient-title = 启用瞬态提示符？
wizard-preview = 预览（当前目录 + 退出码）：

# ===== 向导框架与写盘 =====
wizard-needs-terminal = p11k configure 需要在交互式终端中运行
wizard-use-theme = 使用此主题？
wizard-yes-write-config = 是，写入配置
wizard-aborted = p11k 配置向导已中止。
wizard-nothing-written = 未写入任何内容。
wizard-overwrite = p11k 配置文件 { $path } 已存在，是否覆盖？
wizard-new-config = 新配置：{ $path }。
wizard-add-to-rc = 将对应的一行加入 shell rc：
wizard-engine-is-theme = 引擎即主题：请先移除原有的 ZSH_THEME/主题设置，否则会叠加。
