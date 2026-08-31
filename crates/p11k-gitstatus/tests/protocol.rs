//! 协议层测试（golden tests，逐字节对拍）。
//!
//! 测试数据来源：romkatv/gitstatus v1.5.5 的实际字节流语义。
//! 所有 fixture 字节与调研报告 `.cache/p11k-research/gitstatus-report.md`
//! 一致——协议要求**逐字节兼容**，golden test 是唯一可靠的验证手段。

use p11k_gitstatus::protocol::{self, Request, Response, field};

/// 喂入一条请求（不含 MSG_SEP），断言解析成功。
fn parse(bytes: &[u8]) -> Request {
    protocol::parse_request(bytes)
}

/// 请求解析：三字段普通请求（id + dir + diff='1' → skip_index）。
#[test]
fn parse_request_plain() {
    let r = parse(b"123\x1f/home/u/repo\x1f1");
    assert_eq!(r.id, b"123");
    assert_eq!(r.dir, b"/home/u/repo");
    assert!(!r.dir_is_gitdir);
    assert!(r.skip_index);
}

/// 请求解析：GIT_DIR 形式（dir 带 ':' 前缀 → dir_is_gitdir=true，dir 不含 ':'）。
#[test]
fn parse_request_gitdir_prefix() {
    let r = parse(b"123\x1f:/path/.git");
    assert!(r.dir_is_gitdir);
    assert_eq!(r.dir, b"/path/.git");
}

/// 请求解析：diff 字段缺失时 skip_index=false（对齐原版 Request.diff 默认 true=全算）。
#[test]
fn parse_request_missing_diff() {
    let r = parse(b"123\x1f/home/u/repo");
    assert!(!r.skip_index);
}

/// 请求解析：diff='0' 与缺省等价（都全算）。
#[test]
fn parse_request_diff_zero() {
    let r = parse(b"123\x1f/home/u/repo\x1f0");
    assert!(!r.skip_index);
}

/// 握手请求：id=}hello、dir 为空。原版此处是 `*begin` 解引用的 UB，
/// p11k 安全处理为空 dir 且不触发 from_dotgit。
#[test]
fn parse_request_hello_handshake() {
    let r = parse(b"}hello\x1f");
    assert_eq!(r.id, b"}hello");
    assert_eq!(r.dir, b"");
    assert!(!r.dir_is_gitdir);
}

/// 空输入：畸形（EOF 由 daemon 层 read 0 字节检测，不会传入本函数）。
#[test]
#[should_panic(expected = "empty message")]
fn parse_request_empty_input() {
    protocol::parse_request(b"");
}

/// 畸形：缺少字段分隔符（原版 VERIFY → abort；p11k panic 对齐）。
#[test]
#[should_panic(expected = "missing dir field")]
fn malformed_no_field_sep() {
    parse(b"123");
}

/// 畸形：diff 字段非 '0'/'1'。
#[test]
#[should_panic(expected = "bad diff field")]
fn malformed_bad_diff() {
    parse(b"123\x1fdir\x1f2");
}

/// 畸形：diff 字段多于 1 字节。
#[test]
#[should_panic(expected = "bad diff field")]
fn malformed_two_byte_diff() {
    parse(b"123\x1fdir\x1f10");
}

/// 畸形：超过 3 个字段。
#[test]
#[should_panic(expected = "too many fields")]
fn malformed_too_many_fields() {
    parse(b"123\x1fdir\x1f1\x1fx");
}

/// 握手响应：非仓库响应只有 id 与 '0' 两个字段。
#[test]
fn serialize_hello_response() {
    let r = Response {
        id: b"}hello".to_vec(),
        is_repo: false,
        fields: None,
    };
    assert_eq!(protocol::serialize_response(&r), b"}hello\x1f0\x1e");
}

/// 仓库响应：27 个数据字段的精确顺序（与 field 常量逐一核对）。
#[test]
fn serialize_repo_response_field_order() {
    let fields: [Vec<u8>; field::COUNT] = std::array::from_fn(|i| format!("f{i:02}").into_bytes());
    let r = Response {
        id: b"id".to_vec(),
        is_repo: true,
        fields: Some(fields),
    };
    let out = protocol::serialize_response(&r);
    let mut expect = b"id\x1f1".to_vec();
    for i in 0..field::COUNT {
        expect.push(protocol::FIELD_SEP);
        expect.extend(format!("f{i:02}").bytes());
    }
    expect.push(protocol::MSG_SEP);
    assert_eq!(out, expect);
}

/// id 也过 SafePrint（对齐 ResponseWriter 构造）：控制字符 → '?'。
#[test]
fn serialize_id_is_safe_printed() {
    let r = Response {
        id: b"a\x01b".to_vec(),
        is_repo: false,
        fields: None,
    };
    assert_eq!(protocol::serialize_response(&r), b"a?b\x1f0\x1e");
}

/// SafePrint：控制字符（<0x20）与 DEL（0x7F）替换为 '?'。
#[test]
fn safe_print_control_chars() {
    assert_eq!(protocol::safe_print(b"a\x01b\x1fc\x7fd"), b"a?b?c?d");
}

/// SafePrint：字节 >0x7F 原样保留（作者意图；"分支" 的 UTF-8 是 E5 88 86）。
#[test]
fn safe_print_utf8_passthrough() {
    assert_eq!(protocol::safe_print("分支".as_bytes()), "分支".as_bytes());
}

/// SafePrint：纯可打印 ASCII 原样返回。
#[test]
fn safe_print_ascii_passthrough() {
    assert_eq!(protocol::safe_print(b"hello world"), b"hello world");
}
