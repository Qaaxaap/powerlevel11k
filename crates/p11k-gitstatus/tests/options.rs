//! 参数解析与 `-G` 版本校验的集成测试。
//!
//! 用例语义对齐原版 `options.cc`：getopt_long 参数表 + glibc `fnmatch`
//! flags=0 行为（`*`/`?` 跨 `/`、`\` 转义、字符类）。

use p11k_gitstatus::options::{
    PROTOCOL_VERSION, ParseError, ParseOutcome, parse_args, version_matches,
};

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// fnmatch 等价用例：(glob, version, 期望)。
#[test]
fn version_matches_cases() {
    let cases = [
        ("v1.5.5", "v1.5.5", true),
        ("v1.*", "v1.5.5", true), // * 跨 '.'
        ("v1.*", "v2.0.0", false),
        ("*", "", true),    // * 可匹配空串
        ("v?", "v", false), // ? 必须匹配一个字符
        ("v?.*", "v1.5.5", true),
        ("[a-z]1.*", "v1.5.5", true),
        ("[!0-9]*", "v1.5.5", true), // ! 取反
        ("[^0-9]*", "v1.5.5", true), // ^ 取反（GNU 扩展）
        ("[]]x", "]x", true),        // 类首 ] 字面量
        ("", "", true),
        ("v1\\.5", "v1.5", true), // 转义
        ("\\", "\\", true),       // 尾随转义按字面量
        ("[ab", "[ab", true),     // 非合法类按字面量 [
        ("v1*", "v1", true),      // * 匹配空（注意 "v1.*" 的点是字面量，不能匹配 "v1"）
        ("[a-]", "-", true),      // '-' 在类尾是字面量
        ("[-a]", "-", true),      // '-' 在类首是字面量
    ];
    for (glob, version, expected) in cases {
        assert_eq!(
            version_matches(version, glob),
            expected,
            "glob={glob:?} version={version:?}"
        );
    }
}

/// `-G` 匹配当前协议版本：解析成功且 version_glob 被记录。
#[test]
fn parse_g_matching_glob() {
    let r = parse_args(&args(&["-G", PROTOCOL_VERSION])).unwrap();
    let ParseOutcome::Run(o) = r else {
        panic!("expected Run, got {r:?}")
    };
    assert_eq!(o.version_glob.as_deref(), Some(PROTOCOL_VERSION));
}

/// `-G` 不匹配：返回 VersionMismatch（映射退出码 11，不是 10）。
#[test]
fn parse_g_mismatching_glob() {
    let r = parse_args(&args(&["-G", "v0.*"]));
    match r {
        Err(ParseError::VersionMismatch { pattern }) => assert_eq!(pattern, "v0.*"),
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

/// `--version-glob=...` 走同一校验路径。
#[test]
fn parse_long_version_glob_mismatch() {
    let r = parse_args(&args(&["--version-glob=v0.*"]));
    assert!(matches!(r, Err(ParseError::VersionMismatch { .. })));
}

/// `-G` 校验是解析期行为：即使后面跟着非法参数，也先报版本不匹配
/// （对齐原版在 case 'G' 立即 exit 11）。
#[test]
fn parse_g_mismatch_takes_precedence() {
    let r = parse_args(&args(&["-G", "v0.*", "-x"]));
    assert!(matches!(r, Err(ParseError::VersionMismatch { .. })));
}
