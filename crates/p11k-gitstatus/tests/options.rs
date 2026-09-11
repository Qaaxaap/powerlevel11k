//! Integration tests for argument parsing and `-G` version checking.
//!
//! Case semantics mirror the original `options.cc`: the getopt_long option table
//! plus glibc `fnmatch` flags=0 behavior (`*`/`?` crossing `/`, `\` escapes,
//! character classes).

use p11k_gitstatus::options::{
    PROTOCOL_VERSION, ParseError, ParseOutcome, parse_args, version_matches,
};

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// fnmatch equivalence cases: (glob, version, expected).
#[test]
fn version_matches_cases() {
    let cases = [
        ("v1.5.5", "v1.5.5", true),
        ("v1.*", "v1.5.5", true), // * crosses '.'
        ("v1.*", "v2.0.0", false),
        ("*", "", true),    // * matches the empty string
        ("v?", "v", false), // ? must match exactly one character
        ("v?.*", "v1.5.5", true),
        ("[a-z]1.*", "v1.5.5", true),
        ("[!0-9]*", "v1.5.5", true), // ! negation
        ("[^0-9]*", "v1.5.5", true), // ^ negation (GNU extension)
        ("[]]x", "]x", true),        // leading ] in a class is a literal
        ("", "", true),
        ("v1\\.5", "v1.5", true), // escape
        ("\\", "\\", true),       // trailing escape is a literal
        ("[ab", "[ab", true),     // an invalid class makes [ a literal
        ("v1*", "v1", true), // * matches empty (note the dot in "v1.*" is a literal, so it cannot match "v1")
        ("[a-]", "-", true), // '-' at the end of a class is a literal
        ("[-a]", "-", true), // '-' at the start of a class is a literal
    ];
    for (glob, version, expected) in cases {
        assert_eq!(
            version_matches(version, glob),
            expected,
            "glob={glob:?} version={version:?}"
        );
    }
}

/// `-G` matches the current protocol version: parses and records version_glob.
#[test]
fn parse_g_matching_glob() {
    let r = parse_args(&args(&["-G", PROTOCOL_VERSION])).unwrap();
    let ParseOutcome::Run(o) = r else {
        panic!("expected Run, got {r:?}")
    };
    assert_eq!(o.version_glob.as_deref(), Some(PROTOCOL_VERSION));
}

/// `-G` mismatch: returns VersionMismatch (maps to exit code 11, not 10).
#[test]
fn parse_g_mismatching_glob() {
    let r = parse_args(&args(&["-G", "v0.*"]));
    match r {
        Err(ParseError::VersionMismatch { pattern }) => assert_eq!(pattern, "v0.*"),
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

/// `--version-glob=...` takes the same validation path.
#[test]
fn parse_long_version_glob_mismatch() {
    let r = parse_args(&args(&["--version-glob=v0.*"]));
    assert!(matches!(r, Err(ParseError::VersionMismatch { .. })));
}

/// `-G` validation happens at parse time: even when an invalid argument follows, the version
/// mismatch is reported first (mirrors the original's immediate exit 11 in case 'G').
#[test]
fn parse_g_mismatch_takes_precedence() {
    let r = parse_args(&args(&["-G", "v0.*", "-x"]));
    assert!(matches!(r, Err(ParseError::VersionMismatch { .. })));
}
