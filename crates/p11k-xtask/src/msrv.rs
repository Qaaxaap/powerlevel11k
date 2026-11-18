//! Check the workspace with the MSRV the manifest declares.
//!
//! The devShell pins the same toolchain, so this task is the one CI job that runs outside it:
//! the point is to exercise the real rustc from rustup, not the one nix built.

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

use crate::workspace;

pub fn run() -> Result<()> {
    let msrv = workspace::msrv()?;
    let root = workspace::root();
    let sh = Shell::new()?;
    let _dir = sh.push_dir(&root);
    // Installing is idempotent, so a developer machine that already has the toolchain pays
    // nothing and a runner does not need a separate setup step.
    cmd!(sh, "rustup toolchain install {msrv} --profile minimal")
        .run()
        .with_context(|| format!("cannot install Rust {msrv} with rustup"))?;
    cmd!(sh, "cargo +{msrv} check --workspace --all-targets --locked")
        .run()
        .with_context(|| format!("cannot check with Rust {msrv}; is rustup on PATH?"))?;
    Ok(())
}
