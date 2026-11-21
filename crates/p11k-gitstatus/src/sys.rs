//! Safe wrappers for the libc calls this crate makes.
//!
//! Every `unsafe` block lives here with its safety argument; call sites use plain Rust values.
//! The wrappers keep the callers' expectations: errno as `io::Error`, and descriptors the
//! caller owns and must close.

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// A NUL-terminated copy of `path`. A path with an interior NUL cannot reach a syscall.
pub fn cpath(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the path contains a NUL byte"))
}

/// `stat(2)`.
pub fn stat(path: &Path) -> io::Result<libc::stat> {
    let path = cpath(path)?;
    let mut st = zeroed_stat();
    // SAFETY: `path` is NUL-terminated and `st` is a valid, writable buffer.
    if unsafe { libc::stat(path.as_ptr(), &mut st) } == 0 {
        Ok(st)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `lstat(2)`.
pub fn lstat(path: &Path) -> io::Result<libc::stat> {
    let path = cpath(path)?;
    let mut st = zeroed_stat();
    // SAFETY: `path` is NUL-terminated and `st` is a valid, writable buffer.
    if unsafe { libc::lstat(path.as_ptr(), &mut st) } == 0 {
        Ok(st)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `fstat(2)`.
pub fn fstat(fd: RawFd) -> io::Result<libc::stat> {
    let mut st = zeroed_stat();
    // SAFETY: `st` is a valid, writable buffer.
    if unsafe { libc::fstat(fd, &mut st) } == 0 {
        Ok(st)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `fstatat(dirfd, path, AT_SYMLINK_NOFOLLOW)`. `path` must be NUL-terminated.
pub fn fstatat(dirfd: RawFd, path: &[u8]) -> io::Result<libc::stat> {
    let mut st = zeroed_stat();
    // SAFETY: index and readdir paths keep their NUL terminator, so `path` is a valid C string;
    // `st` is a valid, writable buffer.
    if unsafe {
        libc::fstatat(
            dirfd,
            path.as_ptr().cast(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        Ok(st)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `open(path, O_DIRECTORY | O_CLOEXEC)`.
pub fn open_dir(path: &Path) -> io::Result<RawFd> {
    let path = cpath(path)?;
    // SAFETY: `path` is NUL-terminated.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd >= 0 {
        Ok(fd)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `openat(dirfd, path, O_DIRECTORY | O_CLOEXEC)`. `path` must be NUL-terminated.
pub fn open_dir_at(dirfd: RawFd, path: &[u8]) -> io::Result<RawFd> {
    // SAFETY: `path` is NUL-terminated (index and readdir paths keep the terminator).
    let fd = unsafe {
        libc::openat(
            dirfd,
            path.as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd >= 0 {
        Ok(fd)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `dup(2)`.
pub fn dup(fd: RawFd) -> io::Result<RawFd> {
    // SAFETY: `dup` takes a descriptor and has no pointer arguments.
    let dup = unsafe { libc::dup(fd) };
    if dup >= 0 {
        Ok(dup)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `close(2)`. A descriptor the process no longer uses is closed even when the call reports an
/// error; callers that care about the error keep it.
pub fn close(fd: RawFd) -> io::Result<()> {
    // SAFETY: `close` takes a descriptor and has no pointer arguments.
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// One `readdir(3)` entry. The name keeps its NUL terminator: the scanner passes it to
/// [`fstatat`] unchanged.
pub struct DirEntry {
    pub name: Vec<u8>,
    pub is_dir: bool,
}

/// `dup` + `fdopendir` + `readdir` + `closedir`, skipping `.` and `..`.
///
/// `fd` stays open; the duplicate is what the directory stream takes.
pub fn read_dir(fd: RawFd) -> io::Result<Vec<DirEntry>> {
    let dup_fd = dup(fd)?;
    // SAFETY: on success `fdopendir` takes ownership of `dup_fd`.
    let dirp = unsafe { libc::fdopendir(dup_fd) };
    if dirp.is_null() {
        let err = io::Error::last_os_error();
        // SAFETY: on failure `fdopendir` did not take the descriptor.
        let _ = unsafe { libc::close(dup_fd) };
        return Err(err);
    }
    let mut entries = Vec::new();
    loop {
        // SAFETY: `dirp` is a valid stream until it is closed below.
        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            break;
        }
        // SAFETY: a non-null `readdir` result points to the stream's current entry, whose name
        // is NUL-terminated.
        let name_bytes = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        // SAFETY: a non-null `readdir` result points to a valid `dirent`.
        let is_dir = unsafe { (*ent).d_type == libc::DT_DIR };
        let mut name = name_bytes.to_vec();
        name.push(0);
        entries.push(DirEntry { name, is_dir });
    }
    // SAFETY: `dirp` came from `fdopendir` and is closed exactly once.
    let _ = unsafe { libc::closedir(dirp) };
    Ok(entries)
}

/// `mkdtemp(3)`: create a unique directory from a template ending in `XXXXXX`.
pub fn mkdtemp(template: &Path) -> io::Result<PathBuf> {
    let mut buf = template.as_os_str().as_bytes().to_vec();
    buf.push(0);
    // SAFETY: `buf` is a mutable, NUL-terminated template; `mkdtemp` rewrites it in place.
    let ptr = unsafe { libc::mkdtemp(buf.as_mut_ptr().cast()) };
    if ptr.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `mkdtemp` returns a pointer into the NUL-terminated template it modified.
    let path = unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec();
    Ok(PathBuf::from(OsStr::from_bytes(&path)))
}

/// `mkdir(2)`.
pub fn mkdir(path: &Path, mode: u32) -> io::Result<()> {
    let path = cpath(path)?;
    // SAFETY: `path` is NUL-terminated.
    if unsafe { libc::mkdir(path.as_ptr(), mode) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `open(path, O_CREAT | O_WRONLY | O_CLOEXEC, mode)`, the probe's touch file.
pub fn create_file(path: &Path, mode: u32) -> io::Result<RawFd> {
    let path = cpath(path)?;
    // SAFETY: `path` is NUL-terminated.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd >= 0 {
        Ok(fd)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `rmdir(2)`.
pub fn rmdir(path: &Path) -> io::Result<()> {
    let path = cpath(path)?;
    // SAFETY: `path` is NUL-terminated.
    if unsafe { libc::rmdir(path.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `unlink(2)`.
pub fn unlink(path: &Path) -> io::Result<()> {
    let path = cpath(path)?;
    // SAFETY: `path` is NUL-terminated.
    if unsafe { libc::unlink(path.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `time(2)`, in seconds.
pub fn time_seconds() -> i64 {
    // SAFETY: a null pointer asks `time` for the current time instead of writing through it.
    unsafe { libc::time(std::ptr::null_mut()) }
}

/// `poll(2)`. `EINTR` surfaces as [`io::ErrorKind::Interrupted`].
pub fn poll(fds: &mut [libc::pollfd], timeout_ms: i32) -> io::Result<usize> {
    // SAFETY: `fds` is a valid slice of `pollfd`; `poll` does not keep the pointer.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if ready >= 0 {
        Ok(ready as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `read(2)`.
pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `buf` is a valid, writable slice.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n >= 0 {
        Ok(n as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `write(2)`.
pub fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: `buf` is a valid slice.
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    if n >= 0 {
        Ok(n as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `fcntl(fd, F_GETLK, &mut fl)`.
pub fn fcntl_getlk(fd: RawFd, fl: &mut libc::flock) -> io::Result<()> {
    // SAFETY: `fl` is a valid, writable `flock`.
    if unsafe { libc::fcntl(fd, libc::F_GETLK, fl) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `kill(pid, sig)`.
pub fn kill(pid: i32, sig: i32) -> io::Result<()> {
    // SAFETY: `kill` has no pointer arguments.
    if unsafe { libc::kill(pid, sig) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `pipe(2)`, returning the read and write ends.
pub fn pipe() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is a valid array of two descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
        Ok((fds[0], fds[1]))
    } else {
        Err(io::Error::last_os_error())
    }
}

fn zeroed_stat() -> libc::stat {
    // SAFETY: all-zero is a valid initial value for `stat`; the call fills it.
    unsafe { std::mem::zeroed() }
}
