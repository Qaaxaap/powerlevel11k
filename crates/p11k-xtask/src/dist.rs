//! Build the release artifacts for one target.
//!
//! These tasks cover what the release workflow used to do in shell: the build, the staged
//! directory, the tarballs, the Debian packages and the dependency list. Shell was the problem.
//! `tar`, `sha256sum`, `ldd` and `dpkg-deb` are either absent or behave differently outside GNU
//! userland, so a second platform would have needed a second script.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufWriter, Write as IoWrite};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use flate2::Compression;
use flate2::write::GzEncoder;
use goblin::Object;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use xshell::Shell;

use crate::target::Target;
use crate::workspace;

/// A crate a release ships: its Cargo name, the binary it builds and the package name.
const PACKAGES: &[(&str, &str, &str)] = &[
    ("p11k-engine", "p11k", "p11k"),
    ("p11k-d", "p11k-d", "p11k-d"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// A gzipped tarball holding one binary
    Tarball,
    /// A Debian package holding one binary
    Deb,
}

#[derive(Debug, Args)]
pub struct DistArgs {
    /// The target triple, such as x86_64-unknown-linux-gnu
    #[arg(long, value_name = "TRIPLE")]
    target: String,

    /// What to produce; repeat the flag to produce more than one
    #[arg(long, value_enum, default_values = ["tarball", "deb"])]
    format: Vec<Format>,
}

#[derive(Debug, Args)]
pub struct AssembleArgs {
    /// The directory `dist` wrote its per-target output into
    #[arg(long, value_name = "DIR", default_value = "target/dist")]
    dir: PathBuf,
}

pub fn run(args: DistArgs) -> Result<()> {
    let target = Target::parse(&args.target)?;
    let root = workspace::root();
    let version = workspace::version()?;
    let out = root.join("target/dist").join(target.triple());
    // Clear the whole directory, not just `stage/`: a re-run after a version bump would
    // otherwise leave the previous version's tarballs and packages for `assemble` to pick up.
    if out.exists() {
        fs::remove_dir_all(&out).with_context(|| format!("cannot clear {}", out.display()))?;
    }
    fs::create_dir_all(&out).with_context(|| format!("cannot create {}", out.display()))?;
    let sh = Shell::new()?;

    build(&sh, &root, target)?;
    let staged = stage(&root, target, &out)?;

    if args.format.contains(&Format::Tarball) {
        for (_, binary, _) in PACKAGES {
            tarball(&out, target, &version, binary, &staged.join(binary))?;
        }
    }
    if args.format.contains(&Format::Deb) {
        for (crate_name, _, package) in PACKAGES {
            deb(&sh, &root, target, &version, crate_name, package, &out)?;
        }
    }

    dependencies(&root, target, &out)?;
    check_glibc_floor(&sh, &root, target)?;
    smoke_test(&staged, &version)?;
    Ok(())
}

/// Build both binaries with the glibc floor the target names.
fn build(sh: &Shell, root: &Path, target: Target) -> Result<()> {
    let zig_target = target.zig_target();
    let mut command = sh
        .cmd("cargo")
        .arg("zigbuild")
        .arg("--release")
        .arg("--locked")
        .arg("--target")
        .arg(&zig_target);
    for (crate_name, _, _) in PACKAGES {
        command = command.arg("-p").arg(crate_name);
    }
    let _dir = sh.push_dir(root);
    command.run()?;
    Ok(())
}

/// Where the build puts the binary for `target`.
fn binary_path(root: &Path, target: Target, binary: &str) -> PathBuf {
    root.join("target")
        .join(target.triple())
        .join("release")
        .join(binary)
}

/// Copy the binaries and the documents into one directory per package.
fn stage(root: &Path, target: Target, out: &Path) -> Result<PathBuf> {
    let staged = out.join("stage");
    if staged.exists() {
        fs::remove_dir_all(&staged)
            .with_context(|| format!("cannot clear {}", staged.display()))?;
    }

    for (_, binary, _) in PACKAGES {
        let dir = staged.join(binary);
        let bin_dir = dir.join("bin");
        fs::create_dir_all(&bin_dir)
            .with_context(|| format!("cannot create {}", bin_dir.display()))?;

        let source = binary_path(root, target, binary);
        if !source.is_file() {
            bail!(
                "{} is missing; the build for {target} did not produce it",
                source.display()
            );
        }
        fs::copy(&source, bin_dir.join(binary))
            .with_context(|| format!("cannot copy {}", source.display()))?;

        for document in ["README.md", "README.zh.md", "COPYING", "COPYING.LESSER"] {
            let from = root.join(document);
            fs::copy(&from, dir.join(document))
                .with_context(|| format!("cannot copy {}", from.display()))?;
        }
    }

    Ok(staged)
}

/// Write one tarball holding one package.
fn tarball(out: &Path, target: Target, version: &str, binary: &str, dir: &Path) -> Result<()> {
    let prefix = format!("{binary}-v{version}-{}", target.triple());
    let path = out.join(format!("{prefix}.tar.gz"));

    let file = File::create(&path).with_context(|| format!("cannot create {}", path.display()))?;
    let encoder = GzEncoder::new(BufWriter::new(file), Compression::best());
    let mut builder = Builder::new(encoder);

    let mut entries: Vec<(String, PathBuf, u32)> = vec![(
        format!("{prefix}/bin/{binary}"),
        dir.join("bin").join(binary),
        0o755,
    )];
    for document in ["README.md", "README.zh.md", "COPYING", "COPYING.LESSER"] {
        entries.push((format!("{prefix}/{document}"), dir.join(document), 0o644));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, source, mode) in entries {
        append(&mut builder, &name, &source, mode)?;
    }
    builder
        .into_inner()
        .context("cannot finish the tarball")?
        .finish()
        .context("cannot finish the gzip stream")?;
    Ok(())
}

/// Append one file with fixed ownership, timestamp and mode.
///
/// The fixed metadata keeps the archive reproducible: the same content produces the same bytes
/// on any machine.
fn append<W: IoWrite>(
    builder: &mut Builder<W>,
    name: &str,
    source: &Path,
    mode: u32,
) -> Result<()> {
    let data = fs::read(source).with_context(|| format!("cannot read {}", source.display()))?;
    let mut header = Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    builder
        .append_data(&mut header, name, &data[..])
        .with_context(|| format!("cannot add {name} to the archive"))?;
    Ok(())
}

/// Build the Debian package for one crate.
///
/// The control file comes from `[package.metadata.deb]` in the crate's manifest, so the package
/// name, the maintainer and the runtime dependency live next to the code they describe.
fn deb(
    sh: &Shell,
    root: &Path,
    target: Target,
    version: &str,
    crate_name: &str,
    package: &str,
    out: &Path,
) -> Result<()> {
    let variant = target.is_musl().then_some("musl");
    let expected = target.depends();
    let declared = workspace::deb_depends(crate_name, variant)?;
    if declared.as_deref() != Some(expected.as_str()) {
        bail!(
            "{crate_name} declares Depends {:?} for variant {variant:?}, but {target} needs {expected:?}; \
             update [package.metadata.deb] in crates/{crate_name}/Cargo.toml",
            declared
        );
    }

    let name = if target.is_musl() {
        format!("{package}-musl")
    } else {
        package.to_owned()
    };
    let semver = semver::Version::parse(version)
        .with_context(|| format!("cannot parse the version {version}"))?;
    let path = out.join(format!(
        "{name}_{}-1_{}.deb",
        deb_version(&semver),
        target.deb_arch()
    ));

    // `--no-build` keeps cargo-deb from rebuilding; `--no-strip` keeps it from stripping a
    // binary the release profile already stripped.
    let mut command = sh
        .cmd("cargo")
        .arg("deb")
        .arg("--no-build")
        .arg("--no-strip")
        .arg("-p")
        .arg(crate_name)
        .arg("--target")
        .arg(target.triple())
        .arg("--output")
        .arg(&path);
    if target.is_musl() {
        command = command.arg("--variant").arg("musl");
    }
    let _dir = sh.push_dir(root);
    command.run()?;
    Ok(())
}

/// Write the dynamic libraries each binary links, as Markdown.
fn dependencies(root: &Path, target: Target, out: &Path) -> Result<()> {
    let path = out.join("runtime-dependencies.md");
    let mut document = String::from("# Runtime dependencies\n\n");
    writeln!(
        document,
        "Generated from the dynamic library table of the release binaries for `{target}`. \
         A static musl build carries none."
    )?;
    for (_, binary, _) in PACKAGES {
        let binary_path = binary_path(root, target, binary);
        let data = fs::read(&binary_path)
            .with_context(|| format!("cannot read {}", binary_path.display()))?;
        let object = Object::parse(&data)
            .with_context(|| format!("cannot parse {}", binary_path.display()))?;
        let libraries: Vec<String> = match object {
            Object::Elf(elf) => elf.libraries.iter().map(|l| l.to_string()).collect(),
            Object::Mach(mach) => match mach {
                goblin::mach::Mach::Binary(binary) => {
                    binary.libs.iter().map(|l| (*l).to_owned()).collect()
                }
                goblin::mach::Mach::Fat(_) => bail!("universal binaries are not a release target"),
            },
            _ => bail!("{} is not an ELF or Mach-O binary", binary_path.display()),
        };
        writeln!(document, "\n## {binary}\n")?;
        if libraries.is_empty() {
            writeln!(document, "Statically linked.")?;
        } else {
            for library in libraries {
                writeln!(document, "- `{library}`")?;
            }
        }
    }
    fs::write(&path, document).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Check that no binary needs a glibc newer than the floor the target declares.
///
/// The floor is only a build argument until this runs. A dependency that pulls in a newer
/// symbol would otherwise ship a binary that fails to start on the distribution the floor names.
fn check_glibc_floor(sh: &Shell, root: &Path, target: Target) -> Result<()> {
    let Some(floor) = target.glibc_floor() else {
        return Ok(());
    };
    let floor = parse_version(floor).expect("the floor is a version");
    for (_, binary, _) in PACKAGES {
        let path = binary_path(root, target, binary);
        let table = sh
            .cmd("objdump")
            .arg("-T")
            .arg(&path)
            .read()
            .with_context(|| format!("cannot read the symbol table of {}", path.display()))?;

        let mut newest: Option<((u32, u32, u32), String)> = None;
        for token in table.split_whitespace() {
            // `objdump -T` writes the version in parentheses, `(GLIBC_2.17)`, so the prefix is
            // not at the start of the token.
            let Some(at) = token.find("GLIBC_") else {
                continue;
            };
            let rest = token[at + "GLIBC_".len()..].trim_end_matches(')');
            if rest == "PRIVATE" {
                bail!("{} requires GLIBC_PRIVATE", path.display());
            }
            let Some(version) = parse_version(rest) else {
                continue;
            };
            if newest.as_ref().is_none_or(|(seen, _)| version > *seen) {
                newest = Some((version, rest.to_owned()));
            }
        }

        match newest {
            Some((version, name)) if version > floor => bail!(
                "{} requires {name}, which is newer than the declared glibc floor {}",
                path.display(),
                target.glibc_floor().unwrap_or_default(),
            ),
            _ => {}
        }
    }
    Ok(())
}

/// The file name of an artifact.
fn file_file_name(path: &Path) -> String {
    path.file_name()
        .expect("an artifact has a file name")
        .to_string_lossy()
        .into_owned()
}

/// The version string cargo-deb writes into the control file.
///
/// cargo-deb spells a pre-release with `~`, so `1.0.0-alpha.1` becomes `1.0.0~alpha.1` and the
/// package sorts below `1.0.0`.
fn deb_version(version: &semver::Version) -> String {
    let base = format!("{}.{}.{}", version.major, version.minor, version.patch);
    if version.pre.is_empty() {
        base
    } else {
        format!("{base}~{}", version.pre)
    }
}

/// Parse `2.28` or `2.2.5` into comparable parts.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = match parts.next() {
        Some(part) => part.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Run each staged binary so a broken artifact fails here, not at a user.
///
/// `p11k` prints the crate version. `p11k-d` prints the gitstatusd protocol version it
/// implements (`v1.5.5`), because a client parses that string, so only its exit status is
/// checked.
fn smoke_test(staged: &Path, version: &str) -> Result<()> {
    run_version(staged, "p11k", Some(version))?;
    run_version(staged, "p11k-d", None)?;
    Ok(())
}

fn run_version(staged: &Path, binary: &str, expected: Option<&str>) -> Result<()> {
    let path = staged.join(binary).join("bin").join(binary);
    let output = std::process::Command::new(&path)
        .arg("--version")
        .output()
        .with_context(|| format!("cannot run {}", path.display()))?;
    if !output.status.success() {
        bail!("{} --version exited with {}", path.display(), output.status);
    }
    let printed = String::from_utf8_lossy(&output.stdout);
    if let Some(expected) = expected
        && !printed.contains(expected)
    {
        bail!(
            "{} --version printed {printed:?}, which does not contain {expected}",
            path.display()
        );
    }
    Ok(())
}

/// Merge every target's checksums and dependency lists into one pair of files.
pub fn assemble(args: AssembleArgs) -> Result<()> {
    let root = workspace::root();
    let dir = if args.dir.is_absolute() {
        args.dir
    } else {
        root.join(args.dir)
    };

    let mut files: Vec<PathBuf> = Vec::new();
    let mut targets: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        for file in
            fs::read_dir(&path).with_context(|| format!("cannot read {}", path.display()))?
        {
            let path = file?.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("gz" | "deb")
            ) {
                files.push(path);
            }
        }
        targets.push((name, path));
    }
    if files.is_empty() {
        bail!("no artifacts under {}", dir.display());
    }
    targets.sort_by(|a, b| a.0.cmp(&b.0));
    files.sort();

    let mut sums = String::new();
    for file in &files {
        let data = fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
        let digest = Sha256::digest(&data);
        // The release uploads the artifacts flat, so a `sha256sum -c` in the download
        // directory must find the file name alone, without the artifact directory.
        let name = file_file_name(file);
        sums.push_str(&format!("{digest:x}  {name}\n"));
    }
    fs::write(dir.join("SHA256SUMS"), sums).context("cannot write SHA256SUMS")?;

    let mut document = String::from("# Runtime dependencies\n\n");
    let mut first = true;
    for (target, path) in targets {
        let source = path.join("runtime-dependencies.md");
        if !source.is_file() {
            continue;
        }
        let text = fs::read_to_string(&source)
            .with_context(|| format!("cannot read {}", source.display()))?;
        if !first {
            document.push('\n');
        }
        first = false;
        let label = target.strip_prefix("output-").unwrap_or(target.as_str());
        writeln!(document, "## {label}\n")?;
        let body = text
            .strip_prefix("# Runtime dependencies\n\n")
            .unwrap_or(&text);
        document.push_str(body.trim_end());
        document.push('\n');
    }
    fs::write(dir.join("runtime-dependencies.md"), document)
        .context("cannot write runtime-dependencies.md")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deb_version_uses_a_tilde_for_prereleases() {
        for (semver, deb) in [
            ("1.2.3", "1.2.3"),
            ("1.0.0-alpha.1", "1.0.0~alpha.1"),
            ("2.0.0-rc.2", "2.0.0~rc.2"),
        ] {
            let version = semver::Version::parse(semver).unwrap();
            assert_eq!(deb_version(&version), deb, "{semver}");
        }
    }

    /// The release uploads artifacts flat, so `sha256sum -c` runs in a directory that holds
    /// only the files. The checksum lines must name those files, not the download directory.
    #[test]
    fn assemble_writes_flat_checksums_and_triple_headings() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("output-x86_64-unknown-linux-gnu");
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("p11k-v1.0.0-x86_64-unknown-linux-gnu.tar.gz"),
            b"tar",
        )
        .unwrap();
        fs::write(target.join("p11k_1.0.0_amd64.deb"), b"deb").unwrap();
        fs::write(
            target.join("runtime-dependencies.md"),
            "# Runtime dependencies\n\n## p11k\n\n- `libc.so.6`\n",
        )
        .unwrap();

        assemble(AssembleArgs {
            dir: dir.path().to_path_buf(),
        })
        .unwrap();

        let sums = fs::read_to_string(dir.path().join("SHA256SUMS")).unwrap();
        assert!(
            sums.contains("  p11k-v1.0.0-x86_64-unknown-linux-gnu.tar.gz\n"),
            "{sums}"
        );
        assert!(sums.contains("  p11k_1.0.0_amd64.deb\n"), "{sums}");
        assert!(!sums.contains("output-"), "{sums}");

        let deps = fs::read_to_string(dir.path().join("runtime-dependencies.md")).unwrap();
        assert!(deps.contains("## x86_64-unknown-linux-gnu\n"), "{deps}");
        assert!(!deps.contains("output-"), "{deps}");
    }
}
