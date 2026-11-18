//! Release targets.
//!
//! A target is a Linux triple: the architecture, the operating system and the C library. The
//! glibc version is part of the build command rather than a property of the build machine, so
//! the Debian `Depends` field and the binary it describes come from the same constant.

use std::fmt;

use anyhow::{Result, bail};

/// The oldest glibc a gnu binary may require.
///
/// 2.17 is the oldest glibc Rust supports for `x86_64-unknown-linux-gnu`, so that is the floor.
/// The linker enforces it: 2.15 lacks `getauxval`, which `std`'s stack overflow handler calls,
/// and the build fails with `undefined symbol: getauxval`. Pinning the floor lets `cargo-deb`
/// write into `Depends` the same version the build used.
const GLIBC: &str = "2.17";

/// The architectures p11k publishes for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

/// The C library a binary links against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Libc {
    Gnu,
    Musl,
}

/// One release target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    arch: Arch,
    libc: Libc,
}

impl Target {
    /// Parse a triple such as `x86_64-unknown-linux-gnu`.
    ///
    /// # Errors
    ///
    /// Returns an error for a triple that is not Linux, or whose architecture or C library has
    /// no release configuration here.
    pub fn parse(triple: &str) -> Result<Self> {
        let Some((arch, libc)) = triple.split_once("-unknown-linux-") else {
            bail!("{triple} is not a Linux target triple")
        };
        let arch = match arch {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => bail!("unsupported architecture: {other}"),
        };
        let libc = match libc {
            "gnu" => Libc::Gnu,
            "musl" => Libc::Musl,
            other => bail!("unsupported C library: {other}"),
        };
        Ok(Self { arch, libc })
    }

    /// The full triple, which is also the name the artifacts carry.
    pub fn triple(self) -> String {
        format!("{}-unknown-linux-{}", self.arch, self.libc)
    }

    /// The highest glibc version a gnu binary may require.
    pub fn glibc_floor(self) -> Option<&'static str> {
        matches!(self.libc, Libc::Gnu).then_some(GLIBC)
    }

    /// The target passed to `cargo zigbuild`: the triple, with the glibc floor appended for a
    /// gnu build.
    pub fn zig_target(self) -> String {
        match self.libc {
            Libc::Gnu => format!("{}.{GLIBC}", self.triple()),
            Libc::Musl => self.triple(),
        }
    }

    /// The Debian architecture name.
    pub fn deb_arch(self) -> &'static str {
        match self.arch {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
        }
    }

    /// The `Depends` field cargo-deb writes into the control file.
    ///
    /// A musl binary is static and a gnu binary gets compiler-rt from zig, so libc6 is the only
    /// runtime package either one needs. glibc 2.17 keeps libpthread, libdl and libutil in that
    /// same package.
    pub fn depends(self) -> String {
        match self.libc {
            Libc::Gnu => format!("libc6 (>= {GLIBC})"),
            // The binary is static, so there is nothing to declare. `$auto` makes cargo-deb
            // read that from the binary rather than take a guess here.
            Libc::Musl => "$auto".to_owned(),
        }
    }

    /// Whether this target links musl.
    pub fn is_musl(self) -> bool {
        matches!(self.libc, Libc::Musl)
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

impl fmt::Display for Libc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Gnu => "gnu",
            Self::Musl => "musl",
        })
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_release_target() {
        for (triple, zig, deb) in [
            (
                "x86_64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu.2.17",
                "amd64",
            ),
            (
                "aarch64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu.2.17",
                "arm64",
            ),
            (
                "x86_64-unknown-linux-musl",
                "x86_64-unknown-linux-musl",
                "amd64",
            ),
            (
                "aarch64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
                "arm64",
            ),
        ] {
            let target = Target::parse(triple).unwrap();
            assert_eq!(target.triple(), triple);
            assert_eq!(target.zig_target(), zig);
            assert_eq!(target.deb_arch(), deb);
        }
    }

    #[test]
    fn musl_has_no_runtime_dependency() {
        let musl = Target::parse("x86_64-unknown-linux-musl").unwrap();
        let gnu = Target::parse("x86_64-unknown-linux-gnu").unwrap();
        assert!(musl.is_musl());
        assert_eq!(musl.depends(), "$auto");
        assert_eq!(gnu.depends(), "libc6 (>= 2.17)");
    }

    #[test]
    fn rejects_unknown_targets() {
        for triple in [
            "x86_64-apple-darwin",
            "riscv64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnux32",
        ] {
            assert!(Target::parse(triple).is_err(), "{triple}");
        }
    }
}
