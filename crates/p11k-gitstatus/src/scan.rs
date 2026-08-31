//! 工作区遍历（对齐 `index.cc` 的 ScanDirs 与 `dir.cc` 的 ListDir）。
//!
//! # 遍历要求（与原版逐点对齐）
//!
//! - **从父目录 fd 出发**：`openat(fd, name)` + `fstatat(fd, name,
//!   AT_SYMLINK_NOFOLLOW)`，避免逐级绝对路径查找。
//! - **目录栈**：`fds[d-1]` 为深度 d 的目录 fd；前序访问保证父目录 fd
//!   在栈顶，`truncate(depth)` 后 push 新 fd（等价原版 rotate，openat 次数相同）。
//! - **StatFiles**：index 文件逐个 fstatat + is_modified（无论 untracked
//!   缓存如何都做，对齐原版）。
//! - **merge join**：readdir 条目（排序）与 index 文件/子目录三方合并，
//!   产生 modified / deleted / new（untracked）候选。
//! - **untracked cache 剪枝**：目录 mtime 未变时跳过 readdir，复用
//!   unmatched（对齐 index.cc 的 StatEq 检查）。
//! - **d_type**：直接信任 readdir 的 d_type（DT_DIR 判定）；DT_UNKNOWN
//!   按普通文件处理（对齐原版：DirentDup 存 d_type，`entry[-1] == DT_DIR`
//!   判定，不做 fallback stat）。

use crate::index::{IndexDir, IndexEntry, RepoCaps, is_modified};
use std::os::fd::RawFd;

pub struct ScanOpts {
    /// 是否收集 untracked 候选（`-d` 上限 > 0）。
    pub include_untracked: bool,
    /// untracked cache 探针结论。
    pub untracked_cache_enabled: bool,
}

/// 一个 readdir 条目。名字含尾部 NUL（fstatat 零分配直传）。
struct Dirent {
    name: Vec<u8>,
    is_dir: bool,
}

/// 扫描 dirs[from..to] 目录片，返回候选路径（相对仓库根的字节串，无 NUL）。
/// 对齐 ScanDirs 的完整流程（见模块文档）。
pub fn scan_dirs(
    dirs: &mut [IndexDir],
    entries: &[IndexEntry],
    root_fd: RawFd,
    caps: &RepoCaps,
    opts: &ScanOpts,
) -> Vec<Vec<u8>> {
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    // fds[d-1] = 深度 d 的目录 fd
    let mut fds: Vec<RawFd> = Vec::new();

    for idx in 0..dirs.len() {
        // 打开当前目录（父 fd 来自栈，栈截断后 push）
        let fd = match open_dir(&mut fds, root_fd, dirs, idx) {
            Some(fd) => fd,
            None => {
                // 目录打不开：清 untracked 缓存、无候选（对齐原版 AddUnmached("")）
                dirs[idx].st = None;
                dirs[idx].unmatched.clear();
                continue;
            }
        };
        let dir_path_len = dirs[idx].path.len() - 1;

        // StatFiles：index 记录的文件逐个比对（对齐原版，无论缓存如何都做）
        let file_idxs = dirs[idx].files.clone();
        for &ei in &file_idxs {
            let entry = &entries[ei];
            let basename = &entry.path[dir_path_len..entry.path.len() - 1];
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: basename 由条目路径派生且 NUL 结尾；st 为合法 stat 缓冲。
            let r = unsafe {
                libc::fstatat(
                    fd,
                    basename.as_ptr().cast(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if r != 0 {
                let errno = std::io::Error::last_os_error().raw_os_error();
                // deleted（ENOENT）或 unreadable 都进候选（对齐原版）
                candidates.push(entry.path[..entry.path.len() - 1].to_vec());
                let _ = errno;
            } else if is_modified(entry, &st, caps) {
                candidates.push(entry.path[..entry.path.len() - 1].to_vec());
            }
        }

        if !opts.include_untracked {
            continue;
        }

        // untracked cache：目录 mtime 未变 → 复用 unmatched，跳过 readdir
        if opts.untracked_cache_enabled {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: fd 为已打开的目录 fd。
            if unsafe { libc::fstat(fd, &mut st) } == 0 {
                let cur = (st.st_mtime, st.st_mtime_nsec);
                if dirs[idx].st == Some(cur) {
                    for p in dirs[idx].unmatched.clone() {
                        candidates.push(p);
                    }
                    continue;
                }
                dirs[idx].st = Some(cur);
            } else {
                dirs[idx].st = None;
                dirs[idx].unmatched.clear();
                continue;
            }
        }

        // readdir + 排序；读失败 → 清缓存、无候选（对齐原版 ListDir 返回 false）
        let Some(dirents) = list_dir(fd, caps.case_sensitive) else {
            dirs[idx].st = None;
            dirs[idx].unmatched.clear();
            continue;
        };
        dirs[idx].unmatched.clear();

        // merge join：dirents（排序） vs files（entries 已排序） vs subdirs（建树序）
        let files = dirs[idx].files.clone();
        let subdirs = dirs[idx].subdirs.clone();
        let mut fi = 0usize;
        let mut si = 0usize;
        for de in &dirents {
            let name = &de.name[..de.name.len() - 1]; // 去 NUL
            // 与 index 文件合并
            let mut matched = false;
            while fi < files.len() {
                let entry = &entries[files[fi]];
                let base = &entry.path[dir_path_len..entry.path.len() - 1];
                let cmp = cmp_name(base, name, caps.case_sensitive);
                if cmp == std::cmp::Ordering::Less {
                    // index 有、磁盘无 → deleted
                    candidates.push(entry.path[..entry.path.len() - 1].to_vec());
                    fi += 1;
                } else if cmp == std::cmp::Ordering::Equal {
                    let mut st: libc::stat = unsafe { std::mem::zeroed() };
                    // SAFETY: name 来自 readdir 且 NUL 结尾。
                    let r = unsafe {
                        libc::fstatat(
                            fd,
                            de.name.as_ptr().cast(),
                            &mut st,
                            libc::AT_SYMLINK_NOFOLLOW,
                        )
                    };
                    if r != 0 || is_modified(entry, &st, caps) {
                        candidates.push(entry.path[..entry.path.len() - 1].to_vec());
                    }
                    matched = true;
                    fi += 1;
                    break;
                } else {
                    break;
                }
            }
            if matched {
                continue;
            }
            // 与子目录合并（cmp < 0 继续推进，对齐原版循环）
            while si < subdirs.len() {
                let cmp = cmp_name(&dirs[subdirs[si]].basename, name, caps.case_sensitive);
                if cmp == std::cmp::Ordering::Greater {
                    break;
                }
                if cmp == std::cmp::Ordering::Equal {
                    matched = true;
                    si += 1;
                    break;
                }
                si += 1;
            }
            if !matched {
                // untracked：目录名加 '/' 后缀（对齐原版 AddUnmached）
                let mut p = name.to_vec();
                if de.is_dir {
                    p.push(b'/');
                }
                dirs[idx].unmatched.push(p.clone());
                candidates.push(p);
            }
        }
        // 剩余 index 文件 → deleted
        while fi < files.len() {
            let entry = &entries[files[fi]];
            candidates.push(entry.path[..entry.path.len() - 1].to_vec());
            fi += 1;
        }
    }
    candidates
}

/// 打开 dirs[idx] 的目录 fd：父 fd 取栈中 depth-1 层，栈截断到 depth 后
/// push 新 fd；根目录 dup(root_fd)（对齐原版 OpenTail 的栈复用语义）。
fn open_dir(fds: &mut Vec<RawFd>, root_fd: RawFd, dirs: &[IndexDir], idx: usize) -> Option<RawFd> {
    let depth = dirs[idx].depth;
    let parent_fd = if depth == 0 {
        root_fd
    } else {
        *fds.get(depth - 1)?
    };
    fds.truncate(depth);
    let fd = if depth == 0 {
        // SAFETY: dup 语义标准。
        unsafe { libc::dup(root_fd) }
    } else {
        // SAFETY: basename NUL 结尾（见模块文档）。
        unsafe {
            libc::openat(
                parent_fd,
                dirs[idx].basename.as_ptr().cast(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        }
    };
    if fd < 0 {
        return None;
    }
    fds.push(fd);
    Some(fd)
}

/// readdir 收集并排序（对齐 dir.cc ListDir）：dup(fd) + fdopendir +
/// readdir 循环，跳过 "." ".."，名字存 NUL 结尾。
/// 排序：大小写敏感按字节序，不敏感按 ASCII fold（对齐 C locale 的
/// strcasecmp 语义；不依赖进程 locale）。
fn list_dir(fd: RawFd, case_sensitive: bool) -> Option<Vec<Dirent>> {
    // SAFETY: dup 出的 fd 由 fdopendir 接管，closedir 时释放。
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return None;
    }
    // SAFETY: fdopendir 接管 dup_fd。
    let dirp = unsafe { libc::fdopendir(dup_fd) };
    if dirp.is_null() {
        // SAFETY: fdopendir 失败时 fd 未被接管，手动关闭。
        unsafe { libc::close(dup_fd) };
        return None;
    }
    let mut entries: Vec<Dirent> = Vec::with_capacity(128);
    loop {
        // SAFETY: readdir 返回 dirp 内部的 dirent 指针。
        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            break;
        }
        // SAFETY: ent 有效（readdir 非 null 返回）。
        let name_bytes = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let mut name = name_bytes.to_vec();
        name.push(0); // NUL（fstatat 直传）
        // SAFETY: ent 有效。
        let is_dir = unsafe { (*ent).d_type == libc::DT_DIR };
        entries.push(Dirent { name, is_dir });
    }
    // SAFETY: closedir 同时释放 fdopendir 接管的 fd。
    unsafe { libc::closedir(dirp) };
    if case_sensitive {
        entries.sort_by(|a, b| a.name[..a.name.len() - 1].cmp(&b.name[..b.name.len() - 1]));
    } else {
        entries.sort_by(|a, b| {
            let ka = a.name[..a.name.len() - 1]
                .iter()
                .map(|&c| c.to_ascii_lowercase());
            let kb = b.name[..b.name.len() - 1]
                .iter()
                .map(|&c| c.to_ascii_lowercase());
            ka.cmp(kb)
        });
    }
    Some(entries)
}

/// 名字比较（对齐 StrCmp 的 StringView vs char* 语义：逐字节 + 长度决胜）。
fn cmp_name(a: &[u8], b: &[u8], case_sensitive: bool) -> std::cmp::Ordering {
    let n = a.len().min(b.len());
    for i in 0..n {
        let (x, y) = if case_sensitive {
            (a[i], b[i])
        } else {
            (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase())
        };
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}
