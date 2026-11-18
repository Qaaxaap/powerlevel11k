//! Workspace paths and the fields the tasks read from the manifest.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cargo_metadata::MetadataCommand;

/// The package whose version, MSRV and release artifacts the tasks work with.
const ENGINE: &str = "p11k-engine";

/// The workspace root, the directory that holds the top-level `Cargo.toml`.
///
/// `CARGO_MANIFEST_DIR` is `crates/p11k-xtask`, so two levels up is the root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("p11k-xtask lives at crates/p11k-xtask")
        .to_path_buf()
}

/// The manifest entry of one workspace crate.
fn package(name: &str) -> Result<cargo_metadata::Package> {
    let metadata = MetadataCommand::new()
        .manifest_path(root().join("Cargo.toml"))
        .no_deps()
        .exec()
        .context("failed to read the workspace metadata")?;
    match metadata
        .packages
        .into_iter()
        .find(|p| p.name.as_str() == name)
    {
        Some(package) => Ok(package),
        None => bail!("{name} is not a workspace member"),
    }
}

/// The engine package's manifest entry.
fn engine() -> Result<cargo_metadata::Package> {
    package(ENGINE)
}

/// The runtime dependency cargo-deb writes into a package's control file.
///
/// `variant` selects `[package.metadata.deb.variants.<variant>]`. The value is checked against
/// the target rather than trusted: the glibc floor has to stay the one the build used.
pub fn deb_depends(crate_name: &str, variant: Option<&str>) -> Result<Option<String>> {
    let package = package(crate_name)?;
    let mut node = &package.metadata["deb"];
    if let Some(variant) = variant {
        node = &node["variants"][variant];
    }
    Ok(node["depends"].as_str().map(ToOwned::to_owned))
}

/// The version every workspace crate inherits from `[workspace.package]`.
pub fn version() -> Result<String> {
    Ok(engine()?.version.to_string())
}

/// The minimum supported Rust version, as `rust-version` declares it.
pub fn msrv() -> Result<String> {
    engine()?
        .rust_version
        .map(|v| v.to_string())
        .context("p11k-engine does not declare rust-version")
}
