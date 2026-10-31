//! Requests run on their own threads: a burst must come back complete, and each response must
//! belong to its own request.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Build a repo with one commit so every request has a real answer.
fn make_repo(dir: &std::path::Path) -> String {
    let repo = git2::Repository::init(dir).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.email", "t@t").unwrap();
    cfg.set_str("user.name", "t").unwrap();
    let sig = git2::Signature::now("t", "t@t").unwrap();
    std::fs::write(dir.join("a"), b"1").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("a")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &[])
        .unwrap();
    dir.to_str().unwrap().to_string()
}

#[test]
fn a_burst_gets_one_complete_response_per_request() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = make_repo(tmp.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let mut stdin = child.stdin.take().unwrap();
    const N: usize = 64;
    for i in 0..N {
        let req = format!("id{i}\x1f{repo}\x1e");
        stdin.write_all(req.as_bytes()).unwrap();
    }
    // EOF: the daemon answers the in-flight requests, then exits.
    drop(stdin);
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    assert!(child.wait().unwrap().success(), "daemon exit status");

    // A response that interleaved with another would either merge two ids or lose the
    // separator, so parsing the whole stream is the assertion.
    let mut seen = BTreeSet::new();
    for resp in out.split(|&b| b == 0x1e) {
        if resp.is_empty() {
            continue;
        }
        let id = resp.split(|&b| b == 0x1f).next().unwrap();
        let id = std::str::from_utf8(id).unwrap();
        assert!(id.starts_with("id"), "unexpected response id {id}");
        assert!(seen.insert(id.to_string()), "duplicate response for {id}");
    }
    assert_eq!(seen.len(), N, "one response per request");
}
