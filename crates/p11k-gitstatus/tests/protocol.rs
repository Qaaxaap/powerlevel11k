//! 协议层测试（golden tests，逐字节对拍）。
//!
//! 测试数据来源：romkatv/gitstatus v1.5.5 的实际字节流语义。
//! 所有 fixture 字节必须与调研报告 `.cache/p11k-research/gitstatus-report.md`
//! 一致——协议要求**逐字节兼容**，golden test 是唯一可靠的验证手段。
//!
//! # 字节样例（实现测试时直接作为常量使用）
//!
//! ```text
//! 握手请求:  \x1ehello\x1f\x1e          (id=}hello, dir 为空)
//! 握手响应:  \x1ehello\x1f0\x1e          (非仓库响应)
//! 普通请求:  123\x1f/home/u/repo\x1f1\x1e  (id=123, diff='1' 跳过 index)
//! GIT_DIR:   123\x1f:/path/.git\x1f\x1e   (dir 带 ':' 前缀)
//! 仓库响应:  <id>\x1f1\x1f<27 字段...>\x1e (字段顺序见 protocol::field)
//! ```
//!
//! 注意：`\x1f` 是字段分隔符（ASCII 31）、`\x1e` 是消息终结符（ASCII 30），
//! 不要与十六进制转义混淆。

use p11k_gitstatus::protocol;

/// 请求解析：三字段普通请求（id + dir + diff='1'）。
#[test]
fn parse_request_plain() {
    let _ = protocol::parse_request;
    todo!("实现：喂入 '123\x1f/home/u/repo\x1f1\x1e'，断言三个字段值")
}

/// 请求解析：GIT_DIR 形式（dir 带 ':' 前缀 → dir_is_gitdir=true，dir 不含 ':'）。
#[test]
fn parse_request_gitdir_prefix() {
    let _ = protocol::parse_request;
    todo!("实现：喂入 '123\x1f:/path/.git\x1f\x1e'，断言前缀剥离")
}

/// 请求解析：diff 字段缺失时按 None（等价 '0' 全算）处理。
#[test]
fn parse_request_missing_diff() {
    let _ = protocol::parse_request;
    todo!("实现：喂入 '123\x1f/home/u/repo\x1e'，断言 diff == None")
}

/// 握手响应：非仓库响应只有 id 与 '0' 两个字段。
#[test]
fn serialize_hello_response() {
    let _ = protocol::serialize_response;
    todo!("实现：非仓库响应 id=}}hello 序列化应为 '}}hello\\x1f0\\x1e'")
}

/// 仓库响应：27 个数据字段的精确顺序（按 protocol::field 索引逐一构造，
/// 与文档顺序表核对）。
#[test]
fn serialize_repo_response_field_order() {
    let _ = protocol::serialize_response;
    todo!("实现：构造 27 字段响应，断言序列化后字段顺序与 field 常量一致")
}

/// SafePrint：ASCII 控制字符（<32、127）替换为 '?'。
#[test]
fn safe_print_control_chars() {
    let _ = protocol::safe_print;
    todo!("实现：'a\x01b\x1fc\x7fd' → 'a?b?c?d'")
}

/// SafePrint：字节 > 127 原样保留（UTF-8 中文/emoji 不受影响）。
#[test]
fn safe_print_utf8_passthrough() {
    let _ = protocol::safe_print;
    todo!("实现：'分支名' 与 emoji 输入逐字节不变")
}

/// SafePrint：空串、纯 ASCII 串原样返回。
#[test]
fn safe_print_ascii_passthrough() {
    let _ = protocol::safe_print;
    todo!("实现：'hello world' → 'hello world'")
}
