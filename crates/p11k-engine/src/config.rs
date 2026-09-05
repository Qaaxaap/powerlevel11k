//! 主题配置:KDL 解析 + 内置 lean 主题。
//!
//! 用成品 crate [`kdl`](kdl-rs,KDL 官方参考实现)解析;**KDL v2**,布尔字面量 =
//! [`#true`/`#false`](v2 规范),数字/字符串/颜色原生。
//!
//! # 泛用配置模型(对齐 p10k 抽象,用 KDL 表达)
//!
//! 布局用**行结构**表达多行(不是 p10k 元素序列里插 `newline` 标记):
//! - **布局**:`layout { left { line {…} line {…} } right { line {…} } }`。
//!   `left`/`right` 下每个 `line` 节点即一行;行内是段节点(`dir #true` 等,布尔原生,
//!   `#false`/未列出则不启用,顺序=children 顺序);行与行之间就是换行。
//! - **段**:`segments { dir fg=39 bold=#true shorten-strategy="t" … }`。
//!   段节点带**属性**:`fg`、`bg`(颜色)、`bold`(布尔)、
//!   `content`/`icon`/`prefix`/`suffix`(文本)、`disabled`(显隐);其余进
//!   [`Segment::props`](行为,段渲染函数按需读)。
//! - **state 覆盖**:段节点下 `state <NAME> fg=…` 子节点;其样式覆盖段默认,
//!   即 p10k `SEG[_STATE]_ATTR` 三段回退(段STATE → 段 → [`Config::defaults`] 全局兜底)。
//! - **默认**:顶层 `defaults { … }`(属性)是全局回退样式。
//!
//! 段功能由代码实现,外观/布局全由配置驱动——换配置即换主题,不硬编码视觉。

use std::collections::BTreeMap;
use std::fmt;

use kdl::{KdlDocument, KdlNode, KdlValue};

/// 颜色:数字=256 调色板、`#rrggbb`=24 位、`"default"`/缺省=继承终端。
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Default,
    Xterm(u8),
    Rgb(u8, u8, u8),
    Named(String),
}

impl Default for Color {
    fn default() -> Self {
        Color::Default
    }
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

/// 一段的视觉样式(颜色 + 粗体)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Style {
    /// 从一组 KDL 属性读样式键(`fg`、`bg`、`bold`)。
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

/// 布局:左右各是一组行;每行一组元素(顺序即显示顺序),行与行=换行。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub left: Vec<Vec<Element>>,
    pub right: Vec<Vec<Element>>,
    pub add_newline: bool,
}

/// 分隔符/端符族(可配置字符,powerline 风格)。
///
/// - `segment`:异底段间箭头(如 powerline ``)。空=不画(纯文本空格)。
/// - `sub`:同底段间细线(如 ``)。
/// - `end`:左栏末尾端符(右三角 ``,指向前方/行尾)。
#[derive(Clone, Debug, PartialEq)]
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
    /// 行内左右栏之间的 gap 填充字符(如 `·`;空=空格)。
    pub gap: String,
}

impl Default for Separators {
    fn default() -> Self {
        Separators {
            segment: String::new(),
            sub: String::new(),
            end: String::new(),
            right_start: String::new(),
            right_segment: String::new(),
            right_sub: String::new(),
            gap: String::new(),
        }
    }
}

/// 帧的一块(行首/行尾装饰字符):文本 + 可选独立样式。
/// 样式缺省(`None`)时回退 frame 级 → defaults。
#[derive(Clone, Debug, PartialEq)]
pub struct FramePiece {
    pub text: String,
    pub style: Option<Style>,
}

impl Default for FramePiece {
    fn default() -> Self {
        FramePiece {
            text: String::new(),
            style: None,
        }
    }
}

/// 多行帧(行首/行尾装饰,可配字符与颜色)。见 p10k MULTILINE_*_PROMPT_PREFIX/SUFFIX。
/// - `first_*`:第一个 header 行(`╭─`/`─╮`);`newline_*`:中间 header 行(`├─`/`─┤`);
///   `last_*`:输入行(`╰─`/`─╯`)。空 = 不画。
/// - 颜色:`style`(frame 节点属性 `fg`/`bg`/`bold`)是整帧默认,单块可带
///   自己的样式属性覆盖;两者都没配的颜色回退 defaults。
#[derive(Clone, Debug, PartialEq)]
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

impl Default for Frame {
    fn default() -> Self {
        // 默认无帧(纯文本 lean)。经典帧(╭─/╰─)由配置 frame 块开启。
        Frame {
            style: Style::default(),
            first_prefix: FramePiece::default(),
            first_suffix: FramePiece::default(),
            newline_prefix: FramePiece::default(),
            newline_suffix: FramePiece::default(),
            last_prefix: FramePiece::default(),
            last_suffix: FramePiece::default(),
        }
    }
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    /// 段默认样式。
    pub style: Style,
    /// state 情境覆盖(名字 -> 覆盖,如 dir 的 `SHORTENED`/`ANCHOR`)。
    pub states: BTreeMap<String, StateSpec>,
    /// 内容文本(可选;缺省由段渲染函数生成)。
    pub content: Option<String>,
    /// 图标字符(可选)。
    pub icon: Option<String>,
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

    /// 读一个行为属性(按名)。
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.get(name)
    }
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
}

impl Config {
    /// 解析 KDL 文本为配置(用 kdl crate)。
    pub fn parse(src: &str) -> Result<Config, String> {
        let doc = KdlDocument::parse(src).map_err(|e| format!("KDL 解析失败: {e}"))?;
        Self::parse_doc(&doc)
    }

    /// 内置 lean 主题。
    pub fn default_lean() -> Result<Config, String> {
        Self::parse(DEFAULT_LEAN)
    }

    /// 取某段配置(缺省返回默认空段)。
    pub fn segment(&self, name: &str) -> &Segment {
        self.segments.get(name).unwrap_or(&EMPTY_SEG)
    }

    /// 某帧块(prefix/suffix)的实际样式:块级属性 → frame 级 → defaults 回退。
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
        })
    }
}

/// 内置 vcs 远端图标表(对齐 p10k 默认 `VCS_GIT_REMOTE_ICONS`;aur/archlinux 用 )。
fn default_remote_icons() -> Vec<(String, String)> {
    vec![
        ("github".into(), "\u{f113}".into()),            // 
        ("gitlab".into(), "\u{f296}".into()),            // 
        ("bitbucket".into(), "\u{f171}".into()),         // 
        ("aur.archlinux.org".into(), "\u{f303}".into()), // 
        ("archlinux.org".into(), "\u{f303}".into()),     // 
    ]
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
    if b.icon.is_some() {
        a.icon = b.icon;
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

/// 解析 `layout`:{ `left`/`right`(行组)、`add-newline` }。
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
                _ => {}
            }
        }
    }
    Ok(layout)
}

/// 解析一个侧(row 组):每个子节点是一行。
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
            "icon" => seg.icon = str_val(e.value()),
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
                    // state 覆盖 = 样式(fg/bg/bold)+ 可选 char(该态下的显示字符)。
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

/// state 节点的名字 = 首个位置参数(字符串)。
fn first_state_name(node: &KdlNode) -> Option<String> {
    match first_value(node) {
        Some(KdlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// 解析 `separators` 节点:`segment`/`sub`/`end` 子节点,值=字符串(首字符)。
fn parse_separators(node: &KdlNode) -> Separators {
    let mut s = Separators::default();
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
                "gap" => s.gap = ch,
                _ => {}
            }
        }
    }
    s
}

/// 内置 lean 主题(KDL v2)。放在仓库里,不硬编码进渲染逻辑。
pub const DEFAULT_LEAN: &str = r#"
// p11k 内置 lean 主题(默认)。换文件即换主题。
layout {
    left {
        line { dir; vcs }
    }
    right {
        line { status; command_execution_time; background_jobs }
    }
    add-newline #true
}

segments {
    dir fg=39 shorten-strategy="truncate_to_unique" shorten-dir-length=1 {
        state SHORTENED fg=103
        state ANCHOR fg=39 bold=#true
    }
    vcs clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status ok-foreground=70 error-foreground=160 verbose=#true
    command_execution_time threshold-seconds=3 precision=0 fg=101
    background_jobs fg=70 verbose=#false
    prompt_char fg=76 {
        state ERROR fg=196
    }
}
"#;

static EMPTY_SEG: Segment = Segment {
    style: Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
    },
    states: BTreeMap::new(),
    content: None,
    icon: None,
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
        assert!(c.layout.add_newline);
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
