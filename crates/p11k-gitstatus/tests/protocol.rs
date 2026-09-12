//! Protocol-layer tests (golden tests, byte-for-byte comparison).
//!
//! Test data source: the actual byte-stream semantics of romkatv/gitstatus v1.5.5.
//! The protocol requires **byte-for-byte compatibility**, and a golden test is the
//! only reliable way to verify it.

use p11k_gitstatus::protocol::{self, Request, Response, field};

/// Feed one request (without MSG_SEP), asserting it parses.
fn parse(bytes: &[u8]) -> Request {
    protocol::parse_request(bytes)
}

/// Request parsing: plain three-field request (id + dir + diff='1' → skip_index).
#[test]
fn parse_request_plain() {
    let r = parse(b"123\x1f/home/u/repo\x1f1");
    assert_eq!(r.id, b"123");
    assert_eq!(r.dir, b"/home/u/repo");
    assert!(!r.dir_is_gitdir);
    assert!(r.skip_index);
}

/// Request parsing: GIT_DIR form (dir with a ':' prefix → dir_is_gitdir=true, dir without ':').
#[test]
fn parse_request_gitdir_prefix() {
    let r = parse(b"123\x1f:/path/.git");
    assert!(r.dir_is_gitdir);
    assert_eq!(r.dir, b"/path/.git");
}

/// Request parsing: a missing diff field means skip_index=false (mirrors the original Request.diff default true=full computation).
#[test]
fn parse_request_missing_diff() {
    let r = parse(b"123\x1f/home/u/repo");
    assert!(!r.skip_index);
}

/// Request parsing: diff='0' is equivalent to omission (both do the full computation).
#[test]
fn parse_request_diff_zero() {
    let r = parse(b"123\x1f/home/u/repo\x1f0");
    assert!(!r.skip_index);
}

/// Handshake request: id=}hello, empty dir. The original is UB here (`*begin`
/// dereference); p11k handles an empty dir safely and does not trigger from_dotgit.
#[test]
fn parse_request_hello_handshake() {
    let r = parse(b"}hello\x1f");
    assert_eq!(r.id, b"}hello");
    assert_eq!(r.dir, b"");
    assert!(!r.dir_is_gitdir);
}

/// Empty input: malformed (EOF is detected by the daemon layer reading 0 bytes, never passed to this function).
#[test]
#[should_panic(expected = "empty message")]
fn parse_request_empty_input() {
    protocol::parse_request(b"");
}

/// Malformed: missing field separator (the original VERIFY → abort; p11k panics, mirroring it).
#[test]
#[should_panic(expected = "missing dir field")]
fn malformed_no_field_sep() {
    parse(b"123");
}

#[test]
#[should_panic(expected = "bad diff field")]
fn malformed_bad_diff() {
    parse(b"123\x1fdir\x1f2");
}

#[test]
#[should_panic(expected = "bad diff field")]
fn malformed_two_byte_diff() {
    parse(b"123\x1fdir\x1f10");
}

#[test]
#[should_panic(expected = "too many fields")]
fn malformed_too_many_fields() {
    parse(b"123\x1fdir\x1f1\x1fx");
}

/// Handshake response: a non-repo response has only the id and '0' fields.
#[test]
fn serialize_hello_response() {
    let r = Response {
        id: b"}hello".to_vec(),
        is_repo: false,
        fields: None,
    };
    assert_eq!(protocol::serialize_response(&r), b"}hello\x1f0\x1e");
}

/// Repo response: exact order of the 27 data fields (checked one by one against the field constants).
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

/// id also goes through SafePrint (mirrors the ResponseWriter constructor): control chars → '?'.
#[test]
fn serialize_id_is_safe_printed() {
    let r = Response {
        id: b"a\x01b".to_vec(),
        is_repo: false,
        fields: None,
    };
    assert_eq!(protocol::serialize_response(&r), b"a?b\x1f0\x1e");
}

/// SafePrint: control chars (<0x20) and DEL (0x7F) are replaced with '?'.
#[test]
fn safe_print_control_chars() {
    assert_eq!(protocol::safe_print(b"a\x01b\x1fc\x7fd"), b"a?b?c?d");
}

/// SafePrint: bytes above 0x7F pass through unchanged — the `"分支"` literal below is E5 88 86 in UTF-8.
#[test]
fn safe_print_utf8_passthrough() {
    assert_eq!(protocol::safe_print("分支".as_bytes()), "分支".as_bytes());
}

#[test]
fn safe_print_ascii_passthrough() {
    assert_eq!(protocol::safe_print(b"hello world"), b"hello world");
}
