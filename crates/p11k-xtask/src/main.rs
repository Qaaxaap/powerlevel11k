//! Build and release tasks for p11k.
//!
//! `cargo xtask` reaches this crate through the alias in `.cargo/config.toml`. Every task the
//! CI runs has a matching recipe in `justfile`, so the same implementation runs on a developer
//! machine and on a runner.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commit_msg;
mod dist;
mod msrv;
mod release;
mod target;
mod workspace;

#[derive(Debug, Parser)]
#[command(version, about = "Build and release tasks for p11k")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Build one target's release artifacts into target/dist
    Dist(dist::DistArgs),
    /// Merge the checksums and dependency lists of every built target
    Assemble(dist::AssembleArgs),
    /// Check Conventional Commit subjects
    CommitMsg(commit_msg::Args),
    /// Check the workspace with the MSRV the manifest declares
    Msrv,
    /// Check a release tag against the manifest and CHANGELOG.md
    ReleaseCheck(release::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Commands::Dist(args) => dist::run(args),
        Commands::Assemble(args) => dist::assemble(args),
        Commands::CommitMsg(args) => commit_msg::run(args),
        Commands::Msrv => msrv::run(),
        Commands::ReleaseCheck(args) => release::run(args),
    }
}
