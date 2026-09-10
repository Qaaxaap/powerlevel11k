//! 主题配置:KDL 解析 + 内置 lean 主题。
//!
//! 用 crate [`kdl`](kdl-rs,KDL 官方参考实现)解析，使用 KDL v2
//!
//! 布局用行结构表达多行:
//! - 布局:`layout { left { line {…} line {…} } right { line {…} } }`。
//!   `left`/`right` 下每个 `line` 节点即一行;行内是段节点(`dir #true` 等,布尔原生,
//!   `#false`/未列出则不启用,顺序=children 顺序);行与行之间就是换行。
//! - 段:`segments { dir fg=39 bold=#true shorten-strategy="t" … }`。
//!   段节点带属性:`fg`、`bg`、`bold`、
//!   `content`/`icon`/`prefix`/`suffix`、`disabled`;其余进
//!   [`Segment::props`](行为,段渲染函数按需读)。
//! - state 覆盖:段节点下 `state <NAME> fg=…` 子节点;其样式覆盖段默认,
//!   即 p10k `SEG[_STATE]_ATTR` 三段回退(段STATE → 段 → [`Config::defaults`] 全局兜底)。
//! - 默认:顶层 `defaults { … }` 是全局回退样式。
//!
//! 段功能由代码实现,外观/布局全由配置驱动，换配置即换主题,不硬编码视觉。

use std::collections::BTreeMap;
use std::fmt;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

/// 颜色:数字=256 调色板、`#rrggbb`=24 位、`"default"`/缺省=继承终端。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Xterm(u8),
    Rgb(u8, u8, u8),
    Named(String),
}

impl Color {
    fn from_str(s: &str) -> Color {
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    return Color::Rgb(r, g, b);
                }
            }
            Color::Named(s.to_string())
        } else if s == "default" {
            Color::Default
        } else {
            Color::Named(s.to_string())
        }
    }

    fn from_value(v: &KdlValue) -> Color {
        match v {
            KdlValue::String(s) => Color::from_str(s),
            KdlValue::Integer(n) => Color::Xterm((*n).clamp(0, 255) as u8),
            _ => Color::Default,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Default => write!(f, "default"),
            Color::Xterm(n) => write!(f, "{n}"),
            Color::Rgb(r, g, b) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            Color::Named(n) => write!(f, "{n}"),
        }
    }
}

/// 一段的视觉样式。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Style {
    /// 从一组 KDL 属性读样式键。
    fn from_entries(entries: &[kdl::KdlEntry]) -> Style {
        let mut s = Style::default();
        for e in entries {
            let Some(name) = e.name() else { continue };
            match name.value() {
                "fg" => s.fg = Color::from_value(e.value()),
                "bg" => s.bg = Color::from_value(e.value()),
                "bold" => s.bold = bool_val(e.value()),
                _ => {}
            }
        }
        s
    }
}

/// 一个布局元素(行内的一个段或静态文本)。
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// 普通段。
    Seg(String),
    /// 与相邻段同底贴合(p10k 的 `<seg>_joined` 尾缀)。
    Joined(String),
    /// 静态文本(如 `text "some text"`,原样输出,不解释为段)。
    Text(String),
}

/// 布局:左右各是一组行;每行一组元素,行与行=换行。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub left: Vec<Vec<Element>>,
    pub right: Vec<Vec<Element>>,
    pub add_newline: bool,
    /// 宽松布局:连续 prompt 之间留一个空行(p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE`)。
    pub prompt_add_newline: bool,
    /// 瞬态 prompt:命令提交后把多行 header 折叠成单行 `❯`(p10k `TRANSIENT_PROMPT`)。
    /// 仅 zsh 支持(依赖 zle reset-prompt),bash/fish 忽略。
    pub transient_prompt: bool,
}

/// 分隔符/端符族(可配置字符,powerline 风格)。
///
/// - `segment`:异底段间箭头(如 powerline ``)。空=不画(纯文本空格)。
/// - `sub`:同底段间细线(如 ``)。
/// - `end`:左栏末尾端符(右三角 ``,指向前方/行尾)。
/// - `left_tail`:左栏首段起始端符(p10k `LEFT_PROMPT_FIRST_SEGMENT_START_SYMBOL`),
///   左三角 ``,画在最左段之前。
/// - `right_tail`:右栏末段结束端符(p10k `RIGHT_PROMPT_LAST_SEGMENT_END_SYMBOL`),
///   右三角 ``,画在最右段之后。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Separators {
    pub segment: String,
    pub sub: String,
    pub end: String,
    /// 右段行首端符(左三角,如 ``)。
    pub right_start: String,
    /// 右段**异色**段间箭头(左三角,如 ``)。
    pub right_segment: String,
    /// 右段**同色**内部细线(如 ``)。
    pub right_sub: String,
    /// 左栏首段起始端符(左三角 ``)。
    pub left_tail: String,
    /// 右栏末段结束端符(右三角 ``)。
    pub right_tail: String,
    /// 行内左右栏之间的 gap 填充字符(如 `·`;空=空格)。
    pub gap: String,
    /// gap 填充字符的前景色(`separators gap-foreground=240`;缺省=终端默认色)。
    /// 对齐 p10k `POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_FOREGROUND`。
    pub gap_foreground: Option<Color>,
}

/// 帧的一块(行首/行尾装饰字符):文本 + 可选独立样式。
/// 样式缺省时回退 frame 级 → defaults。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FramePiece {
    pub text: String,
    pub style: Option<Style>,
}

/// 多行帧(行首/行尾装饰,可配字符与颜色)。见 p10k MULTILINE_*_PROMPT_PREFIX/SUFFIX。
/// - `first_*`:第一个 header 行(`╭─`/`─╮`);`newline_*`:中间 header 行(`├─`/`─┤`);
///   `last_*`:输入行(`╰─`/`─╯`)。空 = 不画。
/// - 颜色:`style`(frame 节点属性 `fg`/`bg`/`bold`)是整帧默认,单块可带
///   自己的样式属性覆盖;两者都没配的颜色回退 defaults。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// 帧级默认样式。
    pub style: Style,
    pub first_prefix: FramePiece,
    pub first_suffix: FramePiece,
    pub newline_prefix: FramePiece,
    pub newline_suffix: FramePiece,
    pub last_prefix: FramePiece,
    pub last_suffix: FramePiece,
}

/// 行为属性值(类型化,非字符串)。
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// 单个 state 的覆盖:样式 + 可选字符(prompt_char 之类按字符渲染的段用)。
#[derive(Clone, Debug, PartialEq)]
pub struct StateSpec {
    pub style: Style,
    /// 该 state 下段显示的字符(如 prompt_char 的 ERROR 态);None = 沿用段默认。
    pub char: Option<String>,
}

/// 段上的附加文字槽(左/中/右):仅拼接显示、不独立成块。
/// `fg` 为 `None` 时沿用段样式,配了则覆盖前景(背景/粗体仍跟段走)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttachText {
    pub text: String,
    pub fg: Option<Color>,
}

/// 单个段的配置。
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// 段默认样式。
    pub style: Style,
    /// state 情境覆盖(名字 -> 覆盖,如 dir 的 `SHORTENED`/`ANCHOR`)。
    pub states: BTreeMap<String, StateSpec>,
    /// 内容文本(可选;缺省由段渲染函数生成)。
    pub content: Option<String>,
    /// 段左缘附加文字(icon 之前)。
    pub text_left: Option<AttachText>,
    /// 段中间附加文字(icon 与内容之间;仅当段既有 icon 又有文字时渲染)。
    pub text_middle: Option<AttachText>,
    /// 段右缘附加文字(内容之后)。
    pub text_right: Option<AttachText>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// 是否显示(`disabled` 置 false)。
    pub shown: bool,
    /// 其它行为属性(如 `shorten-dir-length`、`threshold-seconds`,由段渲染函数按需读)。
    pub props: BTreeMap<String, Prop>,
}

impl Default for Segment {
    /// 段默认显示（`shown=true`），与 `parse` 缺省一致。
    fn default() -> Self {
        Segment {
            style: Style::default(),
            states: BTreeMap::new(),
            content: None,
            text_left: None,
            text_middle: None,
            text_right: None,
            prefix: None,
            suffix: None,
            shown: true,
            props: BTreeMap::new(),
        }
    }
}

impl Segment {
    /// 取某 state(或段默认)的样式,走 段STATE → 段 → 全局 三段回退。
    pub fn effective_style(&self, state: Option<&str>, globals: &Style) -> Style {
        if let Some(st) = state {
            if let Some(spec) = self.states.get(st) {
                return merge_style(&spec.style, &self.style);
            }
        }
        merge_style(&self.style, globals)
    }

    /// 段在当前 state 下渲染的字符:state.char → 段 `char` 属性 → `default_char`。
    pub fn char_for<'a>(&'a self, state: Option<&str>, default_char: &'a str) -> &'a str {
        if let Some(st) = state {
            if let Some(spec) = self.states.get(st) {
                if let Some(c) = &spec.char {
                    return c;
                }
            }
        }
        match &self.props.get("char") {
            Some(Prop::Str(c)) => c,
            _ => default_char,
        }
    }

    /// 读一个行为属性。
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.get(name)
    }
}

/// 图标/字体模式。决定内置段图标的字符集。
/// `nerdfont-complete` 与 `nerdfont-fontconfig` 共用同一套 Nerd Font 字形，
/// `compatible` 用标准 Unicode + Powerline 字体,
/// 不依赖 Nerd Font;`ascii` 纯 ASCII。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum IconMode {
    /// Nerd Font(完整集)。缺省。
    #[default]
    NerdfontComplete,
    /// Nerd Font(fontconfig 变体,字形码点与 complete 一致)。
    NerdfontFontconfig,
    /// 兼容模式:标准 Unicode + Powerline 字体,不依赖 Nerd Font。
    Compatible,
    /// 纯 ASCII。
    Ascii,
}

impl IconMode {
    fn from_str(s: &str) -> IconMode {
        match s {
            "ascii" => IconMode::Ascii,
            "compatible" => IconMode::Compatible,
            "nerdfont-fontconfig" => IconMode::NerdfontFontconfig,
            _ => IconMode::NerdfontComplete, // 未知/缺省按 nerdfont-complete
        }
    }
}

/// 顶层 `icon {}` 里一个图标名的覆盖。
///
/// 字段语义:
/// - `all`:字符被引擎识别类别后自动落到它能显示的档位， NF 私有区字符
///   只落 nf 档,标准 Unicode 落 nf + compat,纯 ASCII 三档全落。
/// - `nf` / `compat` / `ascii`:覆盖对应档位。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconOverride {
    pub all: Option<String>,
    pub nf: Option<String>,
    pub compat: Option<String>,
    pub ascii: Option<String>,
}

/// 顶层配置。
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub layout: Layout,
    pub segments: BTreeMap<String, Segment>,
    /// 全局回退样式。
    pub defaults: Style,
    /// 分隔符/端符族。
    pub separators: Separators,
    /// 多行帧。
    pub frame: Frame,
    /// vcs 图标按远端域名选择(domain 子串 → icon 字符);按序匹配,未命中用 git 默认。
    pub vcs_remote_icons: Vec<(String, String)>,
    /// 图标/字体模式。
    pub mode: IconMode,
    /// 顶层 `icon{}` 覆盖表(图标名 → 覆盖;段渲染时按它引用的图标名查)。
    pub icon_overrides: BTreeMap<String, IconOverride>,
}

impl Default for Config {
    /// 空配置:`vcs_remote_icons` 用内置默认表(与 `parse` 缺省一致)。
    fn default() -> Self {
        Config {
            layout: Layout::default(),
            segments: BTreeMap::new(),
            defaults: Style::default(),
            separators: Separators::default(),
            frame: Frame::default(),
            vcs_remote_icons: default_remote_icons(),
            mode: IconMode::NerdfontComplete,
            icon_overrides: BTreeMap::new(),
        }
    }
}

impl Config {
    /// 解析 KDL 文本为配置。
    pub fn parse(src: &str) -> Result<Config, String> {
        let doc = KdlDocument::parse(src).map_err(|e| format!("KDL 解析失败: {e}"))?;
        Self::parse_doc(&doc)
    }

    /// 内置 lean 主题。
    pub fn default_lean() -> Result<Config, String> {
        Ok(crate::presets::build(crate::presets::PresetKind::Lean))
    }

    /// 序列化为 KDL 文本;`parse(to_kdl(cfg))` 应得到等价配置(round-trip)。
    /// 用 kdl-rs 的 autoformat 排版(缩进/空格),否则构造出的文档是紧贴的;
    /// 再把字符串值改成带引号的表示(见 [`quote_string_values`])。
    pub fn to_kdl(&self) -> String {
        let mut doc = self.to_document();
        doc.autoformat();
        quote_string_values(&mut doc);
        doc.to_string()
    }

    /// 构建等价 KDL 文档。节点顺序 mode/layout/defaults/separators/frame/
    /// segments/vcs-remote-icons/icon;与 `parse` 缺省值一致的字段省略。
    pub fn to_document(&self) -> KdlDocument {
        let mut doc = KdlDocument::new();
        // mode 必须显式输出:parse 缺省随 locale,省略则 round-trip 不稳。
        doc.nodes_mut().push(leaf("mode", mode_str(&self.mode)));
        doc.nodes_mut().push(self.layout_node());
        if self.defaults != Style::default() {
            let mut n = KdlNode::new("defaults");
            n.entries_mut().extend(style_entries(&self.defaults));
            doc.nodes_mut().push(n);
        }
        if self.separators != Separators::default() {
            doc.nodes_mut().push(self.separators_node());
        }
        if self.frame != Frame::default() {
            doc.nodes_mut().push(self.frame_node());
        }
        if !self.segments.is_empty() {
            let mut n = KdlNode::new("segments");
            let ch = n.ensure_children();
            for (name, seg) in &self.segments {
                ch.nodes_mut().push(segment_node(name, seg));
            }
            doc.nodes_mut().push(n);
        }
        // vcs-remote-icons 省略时 parse 用内置默认表,与默认值等价。
        if self.vcs_remote_icons != default_remote_icons() {
            doc.nodes_mut().push(self.remote_icons_node());
        }
        if !self.icon_overrides.is_empty() {
            doc.nodes_mut().push(self.icon_node());
        }
        doc
    }

    fn layout_node(&self) -> KdlNode {
        let mut n = KdlNode::new("layout");
        let ch = n.ensure_children();
        let mut left = KdlNode::new("left");
        for row in &self.layout.left {
            left.ensure_children().nodes_mut().push(line_node(row));
        }
        ch.nodes_mut().push(left);
        let mut right = KdlNode::new("right");
        for row in &self.layout.right {
            right.ensure_children().nodes_mut().push(line_node(row));
        }
        ch.nodes_mut().push(right);
        if self.layout.add_newline {
            ch.nodes_mut().push(leaf("add-newline", true));
        }
        if self.layout.prompt_add_newline {
            ch.nodes_mut().push(leaf("prompt-add-newline", true));
        }
        if self.layout.transient_prompt {
            ch.nodes_mut().push(leaf("transient-prompt", true));
        }
        n
    }

    fn separators_node(&self) -> KdlNode {
        let s = &self.separators;
        let mut n = KdlNode::new("separators");
        if let Some(fg) = &s.gap_foreground {
            n.entries_mut()
                .push(KdlEntry::new_prop("gap-foreground", color_value(fg)));
        }
        let ch = n.ensure_children();
        for (name, val) in [
            ("segment", &s.segment),
            ("sub", &s.sub),
            ("end", &s.end),
            ("right-start", &s.right_start),
            ("right-segment", &s.right_segment),
            ("right-sub", &s.right_sub),
            ("left-tail", &s.left_tail),
            ("right-tail", &s.right_tail),
            ("gap", &s.gap),
        ] {
            if !val.is_empty() {
                ch.nodes_mut().push(leaf(name, val.clone()));
            }
        }
        n
    }

    fn frame_node(&self) -> KdlNode {
        let mut n = KdlNode::new("frame");
        n.entries_mut().extend(style_entries(&self.frame.style));
        let ch = n.ensure_children();
        for (name, piece) in [
            ("first-prefix", &self.frame.first_prefix),
            ("first-suffix", &self.frame.first_suffix),
            ("newline-prefix", &self.frame.newline_prefix),
            ("newline-suffix", &self.frame.newline_suffix),
            ("last-prefix", &self.frame.last_prefix),
            ("last-suffix", &self.frame.last_suffix),
        ] {
            if piece.text.is_empty() && piece.style.is_none() {
                continue;
            }
            let mut pn = KdlNode::new(name);
            pn.entries_mut().push(KdlEntry::new(piece.text.clone()));
            if let Some(st) = &piece.style {
                pn.entries_mut().extend(style_entries(st));
            }
            ch.nodes_mut().push(pn);
        }
        n
    }

    fn remote_icons_node(&self) -> KdlNode {
        let mut n = KdlNode::new("vcs-remote-icons");
        let ch = n.ensure_children();
        for (domain, icon) in &self.vcs_remote_icons {
            ch.nodes_mut().push(leaf(domain.as_str(), icon.clone()));
        }
        n
    }

    fn icon_node(&self) -> KdlNode {
        let mut n = KdlNode::new("icon");
        let ch = n.ensure_children();
        for (name, ov) in &self.icon_overrides {
            let mut inode = KdlNode::new(name.as_str());
            let ich = inode.ensure_children();
            for (k, v) in [
                ("all", &ov.all),
                ("nf", &ov.nf),
                ("compat", &ov.compat),
                ("ascii", &ov.ascii),
            ] {
                if let Some(val) = v {
                    ich.nodes_mut().push(leaf(k, val.clone()));
                }
            }
            ch.nodes_mut().push(inode);
        }
        n
    }

    /// 取某段配置(缺省返回默认空段)。
    pub fn segment(&self, name: &str) -> &Segment {
        self.segments.get(name).unwrap_or(&EMPTY_SEG)
    }

    /// 某帧块的实际样式:块级属性 → frame 级 → defaults 回退。
    pub fn frame_piece_style(&self, piece: &FramePiece) -> Style {
        let base = merge_style(&self.frame.style, &self.defaults);
        match &piece.style {
            Some(s) => merge_style(s, &base),
            None => base,
        }
    }

    fn parse_doc(doc: &KdlDocument) -> Result<Config, String> {
        let mut layout = Layout::default();
        let mut segments: BTreeMap<String, Segment> = BTreeMap::new();
        let mut defaults = Style::default();
        let mut separators = Separators::default();
        let mut frame = Frame::default();
        let mut vcs_remote_icons = default_remote_icons();
        // 缺省 mode 看 locale,非 UTF-8 终端无法
        // 显示 Unicode ，自动降级 ascii;用户显式写 `mode` 时覆盖。
        let mut mode = if locale_is_utf8() {
            IconMode::NerdfontComplete
        } else {
            IconMode::Ascii
        };
        let mut icon_overrides: BTreeMap<String, IconOverride> = BTreeMap::new();

        for node in doc.nodes() {
            match node.name().value() {
                "layout" => layout = parse_layout(node)?,
                "segments" => {
                    if let Some(ch) = node.children() {
                        for seg in ch.nodes() {
                            let name = seg.name().value().to_string();
                            let parsed = parse_segment(seg)?;
                            // 同段名多个节点(如 os icon=… 与 os fg=… 两行)合并,
                            // 后者覆盖字段,states/props 累积。
                            match segments.get_mut(&name) {
                                Some(existing) => merge_segment(existing, parsed),
                                None => {
                                    segments.insert(name, parsed);
                                }
                            }
                        }
                    }
                }
                "defaults" => defaults = Style::from_entries(node.entries()),
                "separators" => separators = parse_separators(node),
                "frame" => frame = parse_frame(node),
                "vcs-remote-icons" => vcs_remote_icons = parse_remote_icons(node),
                "mode" => {
                    if let Some(s) = first_value(node).and_then(str_val) {
                        mode = IconMode::from_str(&s);
                    }
                }
                "icon" => icon_overrides = parse_icon_table(node),
                _ => {} // 未知顶层忽略(向前兼容)
            }
        }
        Ok(Config {
            layout,
            segments,
            defaults,
            separators,
            frame,
            vcs_remote_icons,
            mode,
            icon_overrides,
        })
    }
}

// ---- 序列化辅助(Config → KDL)----

/// 给所有字符串值设带引号的表示。kdl-rs 对"能当裸标识符"的字符串不加引号
/// (如 `mode nerdfont-complete`、`segment <U+E0B0>`,后者看起来像空值),
/// 而 p11k 的预设/文档一律带引号,这里统一。`autoformat_keep` 让 autoformat
/// 不覆盖这个表示(它只保留 value_repr 与 leading,其余重排)。
fn quote_string_values(doc: &mut KdlDocument) {
    for node in doc.nodes_mut() {
        quote_node_strings(node);
    }
}

fn quote_node_strings(node: &mut KdlNode) {
    for e in node.entries_mut() {
        if let KdlValue::String(s) = e.value() {
            let repr = quote_kdl_string(s);
            let mut fmt = e.format().cloned().unwrap_or(kdl::KdlEntryFormat {
                leading: " ".into(),
                ..Default::default()
            });
            fmt.value_repr = repr;
            fmt.autoformat_keep = true;
            e.set_format(fmt);
        }
    }
    if let Some(children) = node.children_mut() {
        quote_string_values(children);
    }
}

/// KDL v2 字符串字面量(转义规则对齐 kdl-rs 的 `write_string`)。
fn quote_kdl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 单一位置值节点(`<name> "value"` / `<name> #true`)。
fn leaf(name: &str, value: impl Into<KdlValue>) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(value));
    n
}

fn mode_str(m: &IconMode) -> &'static str {
    match m {
        IconMode::NerdfontComplete => "nerdfont-complete",
        IconMode::NerdfontFontconfig => "nerdfont-fontconfig",
        IconMode::Compatible => "compatible",
        IconMode::Ascii => "ascii",
    }
}

fn color_value(c: &Color) -> KdlValue {
    match c {
        Color::Default => KdlValue::Null,
        Color::Xterm(n) => KdlValue::from(*n as i128),
        Color::Rgb(r, g, b) => KdlValue::from(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Named(n) => KdlValue::from(n.clone()),
    }
}

/// 样式 → KDL 属性(fg/bg/bold 仅输出非默认项)。
fn style_entries(style: &Style) -> Vec<KdlEntry> {
    let mut v = Vec::new();
    if style.fg != Color::Default {
        v.push(KdlEntry::new_prop("fg", color_value(&style.fg)));
    }
    if style.bg != Color::Default {
        v.push(KdlEntry::new_prop("bg", color_value(&style.bg)));
    }
    if style.bold {
        v.push(KdlEntry::new_prop("bold", true));
    }
    v
}

fn prop_value(p: &Prop) -> KdlValue {
    match p {
        Prop::Str(s) => KdlValue::from(s.clone()),
        Prop::Int(n) => KdlValue::from(*n as i128),
        Prop::Float(f) => KdlValue::from(*f),
        Prop::Bool(b) => KdlValue::from(*b),
    }
}

/// 一行布局元素 → `line { … }` 节点。
fn line_node(row: &[Element]) -> KdlNode {
    let mut n = KdlNode::new("line");
    let ch = n.ensure_children();
    for el in row {
        match el {
            Element::Seg(name) => {
                ch.nodes_mut().push(KdlNode::new(name.as_str()));
            }
            Element::Joined(name) => {
                ch.nodes_mut().push(KdlNode::new(format!("{name}_joined")));
            }
            Element::Text(t) => {
                ch.nodes_mut().push(leaf("text", t.clone()));
            }
        }
    }
    n
}

/// 单个段 → `<name> … { state … text-* … }` 节点。
fn segment_node(name: &str, seg: &Segment) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().extend(style_entries(&seg.style));
    if let Some(c) = &seg.content {
        n.entries_mut()
            .push(KdlEntry::new_prop("content", c.clone()));
    }
    if let Some(p) = &seg.prefix {
        n.entries_mut()
            .push(KdlEntry::new_prop("prefix", p.clone()));
    }
    if let Some(s) = &seg.suffix {
        n.entries_mut()
            .push(KdlEntry::new_prop("suffix", s.clone()));
    }
    if !seg.shown {
        n.entries_mut().push(KdlEntry::new_prop("disabled", true));
    }
    for (k, v) in &seg.props {
        n.entries_mut()
            .push(KdlEntry::new_prop(k.clone(), prop_value(v)));
    }
    if !seg.states.is_empty()
        || seg.text_left.is_some()
        || seg.text_middle.is_some()
        || seg.text_right.is_some()
    {
        let ch = n.ensure_children();
        for (sname, spec) in &seg.states {
            let mut sn = KdlNode::new("state");
            sn.entries_mut().push(KdlEntry::new(sname.clone()));
            sn.entries_mut().extend(style_entries(&spec.style));
            if let Some(c) = &spec.char {
                sn.entries_mut().push(KdlEntry::new_prop("char", c.clone()));
            }
            ch.nodes_mut().push(sn);
        }
        if let Some(t) = &seg.text_left {
            ch.nodes_mut().push(attach_node("text-left", t));
        }
        if let Some(t) = &seg.text_middle {
            ch.nodes_mut().push(attach_node("text-middle", t));
        }
        if let Some(t) = &seg.text_right {
            ch.nodes_mut().push(attach_node("text-right", t));
        }
    }
    n
}

/// 附加文字槽 → `text-<side> "…" [fg=…]` 节点。
fn attach_node(name: &str, a: &AttachText) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(a.text.clone()));
    if let Some(fg) = &a.fg {
        n.entries_mut()
            .push(KdlEntry::new_prop("fg", color_value(fg)));
    }
    n
}

/// 解析顶层 `icon` 节点:每个子节点 = 一个图标名的覆盖;覆盖字段是它的子节点
/// (如 `ok { all "✔" ascii "V" }`,all/nf/compat/ascii 各一)。
fn parse_icon_table(node: &KdlNode) -> BTreeMap<String, IconOverride> {
    let mut out = BTreeMap::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let name = child.name().value().to_string();
            let mut ov = IconOverride::default();
            if let Some(fields) = child.children() {
                for f in fields.nodes() {
                    let Some(v) = first_value(f).and_then(str_val) else {
                        continue;
                    };
                    match f.name().value() {
                        "all" => ov.all = Some(v),
                        "nf" => ov.nf = Some(v),
                        "compat" => ov.compat = Some(v),
                        "ascii" => ov.ascii = Some(v),
                        _ => {}
                    }
                }
            }
            out.insert(name, ov);
        }
    }
    out
}

/// 内置 vcs 远端图标表。
fn default_remote_icons() -> Vec<(String, String)> {
    vec![
        ("github".into(), "\u{f113}".into()),            // 
        ("gitlab".into(), "\u{f296}".into()),            // 
        ("bitbucket".into(), "\u{f171}".into()),         // 
        ("aur.archlinux.org".into(), "\u{f303}".into()), // 
        ("archlinux.org".into(), "\u{f303}".into()),     // 
    ]
}

fn locale_is_utf8() -> bool {
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return locale_str_is_utf8(v);
            }
        }
    }
    false // 全未设 → C locale → 非 UTF-8
}

/// 单个 locale 串(如 `en_US.UTF-8`、`C.UTF-8`、`C`)的 codeset 是否 UTF-8。
fn locale_str_is_utf8(locale: &str) -> bool {
    let code = locale
        .rsplit('.')
        .next()
        .unwrap_or("")
        .split('@') // 去 modifier,如 UTF-8@euro
        .next()
        .unwrap_or("");
    code.eq_ignore_ascii_case("utf-8") || code.eq_ignore_ascii_case("utf8")
}

/// 同段名多节点的合并:样式非 Default 覆盖、文本/图标后者覆盖、states/props 累积。
fn merge_segment(a: &mut Segment, b: Segment) {
    if b.style.fg != Color::Default {
        a.style.fg = b.style.fg;
    }
    if b.style.bg != Color::Default {
        a.style.bg = b.style.bg;
    }
    if b.style.bold {
        a.style.bold = true;
    }
    if b.content.is_some() {
        a.content = b.content;
    }
    if b.text_left.is_some() {
        a.text_left = b.text_left;
    }
    if b.text_middle.is_some() {
        a.text_middle = b.text_middle;
    }
    if b.text_right.is_some() {
        a.text_right = b.text_right;
    }
    if b.prefix.is_some() {
        a.prefix = b.prefix;
    }
    if b.suffix.is_some() {
        a.suffix = b.suffix;
    }
    if !b.shown {
        a.shown = false;
    }
    for (k, v) in b.states {
        a.states.insert(k, v);
    }
    for (k, v) in b.props {
        a.props.insert(k, v);
    }
}

/// 解析 `vcs-remote-icons` 节点:每个子节点 = domain→icon 字符串。
fn parse_remote_icons(node: &KdlNode) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            if let Some(KdlValue::String(icon)) = first_value(child) {
                out.push((child.name().value().to_string(), icon.clone()));
            }
        }
    }
    out
}

/// 解析 `frame` 节点:frame 级样式(节点属性 fg/bg/bold)+ first/newline/last 的
/// prefix/suffix(子节点,值字符串;子节点自己的 fg/bg/bold 覆盖帧级)。
fn parse_frame(node: &KdlNode) -> Frame {
    let mut f = Frame {
        style: Style::from_entries(node.entries()),
        ..Frame::default()
    };
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let Some(v) = first_value(child) else {
                continue;
            };
            let Some(text) = str_val(v) else { continue };
            let st = Style::from_entries(child.entries());
            // 子节点没写样式属性 → None(回退帧级);写了 → Some(覆盖)。
            let style = if st == Style::default() {
                None
            } else {
                Some(st)
            };
            let piece = FramePiece { text, style };
            match child.name().value() {
                "first-prefix" => f.first_prefix = piece,
                "first-suffix" => f.first_suffix = piece,
                "newline-prefix" => f.newline_prefix = piece,
                "newline-suffix" => f.newline_suffix = piece,
                "last-prefix" => f.last_prefix = piece,
                "last-suffix" => f.last_suffix = piece,
                _ => {}
            }
        }
    }
    f
}

/// 解析 `layout`:{ `left`/`right`、`add-newline` }。
fn parse_layout(node: &KdlNode) -> Result<Layout, String> {
    let mut layout = Layout::default();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "left" => layout.left = parse_lines(child),
                "right" => layout.right = parse_lines(child),
                "add-newline" => {
                    layout.add_newline = first_value(child).map(bool_val).unwrap_or(false);
                }
                "prompt-add-newline" => {
                    layout.prompt_add_newline = first_value(child).map(bool_val).unwrap_or(false);
                }
                "transient-prompt" => {
                    layout.transient_prompt = first_value(child).map(bool_val).unwrap_or(false);
                }
                _ => {}
            }
        }
    }
    Ok(layout)
}

/// 解析一个侧:每个子节点是一行。
fn parse_lines(node: &KdlNode) -> Vec<Vec<Element>> {
    let mut rows = Vec::new();
    if let Some(ch) = node.children() {
        for row in ch.nodes() {
            rows.push(parse_line(row));
        }
    }
    rows
}

/// 解析一行:行内每个子节点是一个段(布尔启用),或 `text "…"` 静态文本。
fn parse_line(node: &KdlNode) -> Vec<Element> {
    let mut out = Vec::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            // 值是字符串 → 静态文本(如 `text "some text"`);否则视为段。
            if let Some(KdlValue::String(s)) = first_value(child) {
                out.push(Element::Text(s.clone()));
                continue;
            }
            let name = child.name().value();
            // 值缺省视为启用;显式 #false 禁用。
            let enabled = first_value(child).map(bool_val).unwrap_or(true);
            if !enabled {
                continue;
            }
            if let Some(base) = name.strip_suffix("_joined") {
                out.push(Element::Joined(base.to_string()));
            } else {
                out.push(Element::Seg(name.to_string()));
            }
        }
    }
    out
}

/// 解析一个段节点:属性(样式/文本/显隐/行为)+ `state <NAME> …` 子节点覆盖。
fn parse_segment(node: &KdlNode) -> Result<Segment, String> {
    let mut seg = Segment {
        style: Style::from_entries(node.entries()),
        shown: true,
        ..Default::default()
    };
    for e in node.entries() {
        let Some(name) = e.name() else { continue };
        match name.value() {
            "fg" | "bg" | "bold" => {}
            "content" => seg.content = str_val(e.value()),
            "prefix" => seg.prefix = str_val(e.value()),
            "suffix" => seg.suffix = str_val(e.value()),
            "disabled" => seg.shown = !bool_val(e.value()),
            _ => {
                seg.props
                    .insert(name.value().to_string(), prop_val(e.value()));
            }
        }
    }
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "state" => {
                    let Some(nm) = first_state_name(child) else {
                        return Err("state 需要名字(位置字符串)".into());
                    };
                    // state 覆盖 = 样式 + 可选 char。
                    let st = Style::from_entries(child.entries());
                    let ch = named_str(child, "char");
                    seg.states.insert(
                        nm,
                        StateSpec {
                            style: st,
                            char: ch,
                        },
                    );
                }
                // 附加文字槽:左/中/右。位置字符串是文本,`fg` 可选(缺省跟段走)。
                "text-left" | "text-middle" | "text-right" => {
                    let Some(text) = first_state_name(child) else {
                        return Err(format!("{} 需要文本(位置字符串)", child.name().value()));
                    };
                    let attach = AttachText {
                        text,
                        fg: named_color(child, "fg"),
                    };
                    match child.name().value() {
                        "text-left" => seg.text_left = Some(attach),
                        "text-middle" => seg.text_middle = Some(attach),
                        "text-right" => seg.text_right = Some(attach),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    Ok(seg)
}

/// 节点的命名字符串属性(如 state 节点的 `char="✘"`)。
fn named_str(node: &KdlNode, name: &str) -> Option<String> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
        .and_then(|e| str_val(e.value()))
}

/// 节点的命名颜色属性(如附加文字的 `fg=196`)。
fn named_color(node: &KdlNode, name: &str) -> Option<Color> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
        .map(|e| Color::from_value(e.value()))
}

/// 节点的首个位置参数值(条目无 name 的那个)。
fn first_value(node: &KdlNode) -> Option<&KdlValue> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| e.value())
}

/// state 节点的名字 = 首个位置参数。
fn first_state_name(node: &KdlNode) -> Option<String> {
    match first_value(node) {
        Some(KdlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// 解析 `separators` 节点:`segment`/`sub`/`end` 子节点,值=字符串(首字符)。
fn parse_separators(node: &KdlNode) -> Separators {
    let mut s = Separators::default();
    // `gap-foreground` 是 separators 节点的属性(不是子节点)。
    if let Some(c) = named_color(node, "gap-foreground") {
        s.gap_foreground = Some(c);
    }
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let Some(v) = first_value(child) else {
                continue;
            };
            let Some(ch) = str_val(v) else { continue };
            match child.name().value() {
                "segment" => s.segment = ch,
                "sub" => s.sub = ch,
                "end" => s.end = ch,
                "right-start" => s.right_start = ch,
                "right-segment" => s.right_segment = ch,
                "right-sub" => s.right_sub = ch,
                "left-tail" => s.left_tail = ch,
                "right-tail" => s.right_tail = ch,
                "gap" => s.gap = ch,
                _ => {}
            }
        }
    }
    s
}

static EMPTY_SEG: Segment = Segment {
    style: Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
    },
    states: BTreeMap::new(),
    content: None,
    text_left: None,
    text_middle: None,
    text_right: None,
    prefix: None,
    suffix: None,
    shown: true,
    props: BTreeMap::new(),
};

/// 合并:上层非 Default 字段覆盖下层(三段回退末端)。
fn merge_style(over: &Style, base: &Style) -> Style {
    Style {
        fg: if over.fg == Color::Default {
            base.fg.clone()
        } else {
            over.fg.clone()
        },
        bg: if over.bg == Color::Default {
            base.bg.clone()
        } else {
            over.bg.clone()
        },
        bold: over.bold || base.bold,
    }
}

fn bool_val(v: &KdlValue) -> bool {
    matches!(v, KdlValue::Bool(true))
}
fn str_val(v: &KdlValue) -> Option<String> {
    match v {
        KdlValue::String(s) => Some(s.clone()),
        KdlValue::Integer(n) => Some(n.to_string()),
        KdlValue::Float(f) => Some(f.to_string()),
        KdlValue::Bool(b) => Some(b.to_string()),
        KdlValue::Null => None,
    }
}
fn prop_val(v: &KdlValue) -> Prop {
    match v {
        KdlValue::String(s) => Prop::Str(s.clone()),
        KdlValue::Integer(n) => Prop::Int((*n) as i64),
        KdlValue::Float(f) => Prop::Float(*f),
        KdlValue::Bool(b) => Prop::Bool(*b),
        KdlValue::Null => Prop::Str(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_kdl_roundtrips_presets() {
        for kind in crate::presets::PresetKind::ALL {
            let cfg = crate::presets::build(kind);
            let out = cfg.to_kdl();
            let cfg2 = Config::parse(&out).unwrap();
            assert_eq!(cfg, cfg2, "{} round-trip 失败,输出:\n{out}", kind.name());
        }
    }

    #[test]
    fn to_kdl_roundtrips_rich_config() {
        let src = r#"
mode "ascii"
layout {
    left {
        line { os_icon; dir_joined; text "|" }
        line { prompt_char }
    }
    right {
        line { status }
    }
    add-newline #true
    prompt-add-newline #true
    transient-prompt #true
}
defaults fg=200 bold=#true
separators gap-foreground=240 {
    segment "\u{e0b0}"
    sub "\u{e0b1}"
    end ""
    right-start "\u{e0b2}"
    right-segment "\u{e0b2}"
    right-sub "\u{e0b3}"
    left-tail "\u{e0b2}"
    right-tail "\u{e0b0}"
    gap "·"
}
frame fg=240 {
    first-prefix "╭─"
    first-suffix "─╮" fg=241
    newline-prefix "├─"
    newline-suffix "─┤"
    last-prefix "╰─"
    last-suffix ""
}
segments {
    dir fg=39 shorten-strategy="truncate_to_unique" shorten-dir-length=3 {
        state SHORTENED fg=103
        state ANCHOR fg=39 bold=#true char="~"
        text-left "L" fg=100
        text-right "R"
    }
    vcs clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status ok-foreground=70 error-foreground=160 verbose=#true
    prompt_char fg=76 {
        state ERROR fg=196 char="✘"
    }
}
vcs-remote-icons {
    github "\u{f113}"
    gitlab "\u{f296}"
}
icon {
    ok { all "✔" ascii "V" }
    folder { nf "\u{f07c}" compat "" }
}
"#;
        let cfg = Config::parse(src).unwrap();
        let out = cfg.to_kdl();
        let cfg2 = Config::parse(&out).unwrap();
        assert_eq!(cfg, cfg2, "rich round-trip 失败,输出:\n{out}");
    }

    #[test]
    fn parses_layout_lines() {
        let c = Config::parse(
            "layout {\n  left {\n    line { dir #true; vcs #true }\n    line { prompt_char #true }\n  }\n  right {\n    line { status #true }\n  }\n  add-newline #true\n}",
        )
        .unwrap();
        // 左:两行
        assert_eq!(
            c.layout.left,
            vec![
                vec![Element::Seg("dir".into()), Element::Seg("vcs".into())],
                vec![Element::Seg("prompt_char".into())],
            ]
        );
        // 右:一行
        assert_eq!(c.layout.right, vec![vec![Element::Seg("status".into())]]);
        assert!(c.layout.add_newline);
    }

    #[test]
    fn parses_icon_mode() {
        // mode 顶层节点:ascii/compatible/nerdfont-fontconfig 分别映射。
        let c = Config::parse("mode \"ascii\"\nlayout { left { line { dir #true } } }").unwrap();
        assert_eq!(c.mode, IconMode::Ascii);
        let c =
            Config::parse("mode \"compatible\"\nlayout { left { line { dir #true } } }").unwrap();
        assert_eq!(c.mode, IconMode::Compatible);
        let c =
            Config::parse("mode \"nerdfont-fontconfig\"\nlayout { left { line { dir #true } } }")
                .unwrap();
        assert_eq!(c.mode, IconMode::NerdfontFontconfig);
        // 未写 mode → 跟随 locale:UTF-8 → nerdfont,否则(控制台/C locale)→ ascii。
        let c = Config::parse("layout { left { line { dir #true } } }").unwrap();
        let expect = if locale_is_utf8() {
            IconMode::NerdfontComplete
        } else {
            IconMode::Ascii
        };
        assert_eq!(c.mode, expect);
    }

    #[test]
    fn locale_codeset_utf8_detection() {
        // locale 串的 codeset 判断(对齐 p10k langinfo[CODESET]):UTF-8 家族是,其余否。
        assert!(locale_str_is_utf8("en_US.UTF-8"));
        assert!(locale_str_is_utf8("C.UTF-8"));
        assert!(locale_str_is_utf8("zh_CN.utf8"));
        assert!(locale_str_is_utf8("en_US.UTF-8@euro"));
        assert!(!locale_str_is_utf8("C"));
        assert!(!locale_str_is_utf8("POSIX"));
        assert!(!locale_str_is_utf8("ja_JP.EUC-JP"));
        assert!(!locale_str_is_utf8("en_US"));
    }

    #[test]
    fn parses_joined_element_and_disabled() {
        // "dir_joined" 同底贴合; 显式 #false 的段被跳过。
        let c = Config::parse(
            "layout {\n  left {\n    line { dir_joined #true; vcs #true; time #false }\n  }\n}",
        )
        .unwrap();
        assert_eq!(c.layout.left[0][0], Element::Joined("dir".into()));
        assert_eq!(c.layout.left[0].len(), 2, "#false 的 time 应被跳过");
    }

    #[test]
    fn parses_segment_attrs_states_and_props() {
        let c = Config::parse(
            r#"layout {}
               segments { dir fg=39 shorten-strategy="truncate_to_unique" {
                   state SHORTENED fg=103
                   state ANCHOR fg=39 bold=#true
               } }"#,
        )
        .unwrap();
        let d = c.segment("dir");
        assert_eq!(d.style.fg, Color::Xterm(39));
        assert_eq!(
            d.props["shorten-strategy"],
            Prop::Str("truncate_to_unique".into())
        );
        assert_eq!(d.states["SHORTENED"].style.fg, Color::Xterm(103));
        assert!(d.states["ANCHOR"].style.bold);
    }

    #[test]
    fn effective_style_three_way_fallback() {
        let c = Config::parse(
            r#"layout {}
               defaults fg=200
               segments { dir fg=39 { state SHORTENED fg=103 } }"#,
        )
        .unwrap();
        assert_eq!(
            c.segment("dir")
                .effective_style(Some("SHORTENED"), &c.defaults)
                .fg,
            Color::Xterm(103)
        );
        assert_eq!(
            c.segment("dir").effective_style(None, &c.defaults).fg,
            Color::Xterm(39)
        );
        assert_eq!(
            c.segment("nope").effective_style(None, &c.defaults).fg,
            Color::Xterm(200)
        );
    }

    #[test]
    fn default_lean_parses() {
        let c = Config::default_lean().unwrap();
        assert!(!c.segments.is_empty());
        assert_eq!(c.layout.left.len(), 1, "lean 左侧 header 只一行");
        assert_eq!(
            c.layout.left[0],
            vec![Element::Seg("dir".into()), Element::Seg("vcs".into())]
        );
        assert_eq!(
            c.layout.right[0],
            vec![
                Element::Seg("status".into()),
                Element::Seg("command_execution_time".into()),
                Element::Seg("background_jobs".into())
            ]
        );
        assert!(c.segment("dir").props.contains_key("shorten-strategy"));
        // prompt_char 段仍在(供输入行前缀上色),但不在 header 布局里。
        assert_eq!(c.segment("prompt_char").style.fg, Color::Xterm(76));
        assert!(
            !c.layout
                .left
                .iter()
                .flatten()
                .any(|e| matches!(e, Element::Seg(s) if s == "prompt_char"))
        );
        assert!(
            !c.layout.add_newline && !c.layout.prompt_add_newline,
            "lean 紧凑,无空行"
        );
    }

    #[test]
    fn bool_and_hex() {
        let c = Config::parse(
            r##"layout {}
defaults fg="#ffffff"
segments {
  a bold=#true x=3
}"##,
        )
        .unwrap();
        assert_eq!(c.defaults.fg, Color::Rgb(255, 255, 255));
        assert!(c.segment("a").style.bold);
        assert_eq!(c.segment("a").props["x"], Prop::Int(3));
    }
}
