//! untracked cache 探针（对齐 `check_dir_mtime.cc` 的 CheckDirMtime）。
//!
//! # 背景
//!
//! git 的 untracked cache 依赖一个文件系统行为：**子目录内容变化会更新
//! 父目录的 mtime**。并非所有文件系统都保证（部分网络/覆盖文件系统
//! 不保证），错误启用会导致 untracked 文件漏报——正确性优先于性能。
//!
//! 探针做法（原版 CheckDirMtime）：在 gitdir 下 mkdtemp，创建 a、b 两个
//! 子目录并记录 mtime，**sleep 1 秒**（保证 mtime 分辨率），随后在 a 下
//! mkdir、b 下 touch 文件；两个父目录 mtime 均变化 → 支持，否则不支持。
//!
//! 探针在**后台线程**执行：不阻塞首次请求；结论未出时按"支持"处理
//! （与 git 的默认乐观一致）。探针残留目录（前缀 `.gitstatus.`）由
//! 清理逻辑移除，避免上次崩溃的残留干扰。

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct UntrackedCache {
    /// 探针结论；未完成时按 true（git 默认乐观）。
    supported: Arc<AtomicBool>,
}

impl UntrackedCache {
    /// 启动后台探针并立即返回。gitdir = 仓库 .git 目录路径。
    pub fn start_probe(gitdir: &Path) -> UntrackedCache {
        let supported = Arc::new(AtomicBool::new(true));
        let flag = supported.clone();
        let gitdir = gitdir.to_path_buf();
        std::thread::spawn(move || {
            let ok = probe_support(&gitdir);
            flag.store(ok, Ordering::Relaxed);
        });
        UntrackedCache { supported }
    }

    /// 当前探针结论（探针未完成时为 true）。
    pub fn enabled(&self) -> bool {
        self.supported.load(Ordering::Relaxed)
    }

    /// 目录 mtime 与缓存记录相等即"未变"（对齐 scan 层的 StatEq 用法）。
    pub fn is_fresh(&self, cached: Option<(i64, i64)>, cur: (i64, i64)) -> bool {
        self.enabled() && cached == Some(cur)
    }
}

/// 探针主体（对齐 CheckDirMtime）。失败一律返回 false（安全降级）。
fn probe_support(gitdir: &Path) -> bool {
    // 清理 10 秒以上的残留探针目录（对齐 RemoveStaleDirs）
    remove_stale_dirs(gitdir);

    // mkdtemp：gitdir/.gitstatus.XXXXXX
    let Some(tmp) = mkdtemp(gitdir.join(".gitstatus.XXXXXX")) else {
        return false;
    };
    // RAII 清理句柄：函数结束（无论成功/失败路径）时删除探针目录。
    let _cleanup = Cleanup(tmp.clone());

    let a_dir = tmp.join("a");
    let b_dir = tmp.join("b");
    // SAFETY: 路径经 nul() 保证 NUL 结尾，mkdir 语义标准。
    if unsafe { libc::mkdir(nul(&a_dir).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    let Some(a_st) = lstat(&a_dir) else {
        return false;
    };
    // SAFETY: 同上。
    if unsafe { libc::mkdir(nul(&b_dir).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    let Some(b_st) = lstat(&b_dir) else {
        return false;
    };

    // 保证 mtime 分辨率（对齐原版 while (sleep(1))）
    std::thread::sleep(std::time::Duration::from_secs(1));

    // a/1 子目录：检查"子目录创建更新父目录 mtime"
    let a1 = a_dir.join("1");
    // SAFETY: 同上。
    if unsafe { libc::mkdir(nul(&a1).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    if !stat_changed(&a_dir, &a_st) {
        return false;
    }

    // b/1 文件：检查"子文件创建更新父目录 mtime"（对齐 Touch = creat 0444）
    let b1 = b_dir.join("1");
    // SAFETY: 同上。
    let fd = unsafe {
        libc::open(
            nul(&b1).as_ptr().cast(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_CLOEXEC,
            0o444,
        )
    };
    if fd < 0 {
        return false;
    }
    // SAFETY: close 语义标准。
    unsafe { libc::close(fd) };
    stat_changed(&b_dir, &b_st)
}

/// 路径字节 + NUL 结尾（C 字符串直传 helper，避免每次 lstat/mkdir 时
/// 遗漏 NUL 导致读越界）。
fn nul(path: &Path) -> Vec<u8> {
    let mut b = path.as_os_str().as_bytes().to_vec();
    b.push(0);
    b
}

/// RAII 清理：Drop 时递归删除探针目录（rmdir 链：a/1、a、b/1、b、根）。
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let root = &self.0;
        let a1 = root.join("a/1");
        let a = root.join("a");
        let b1 = root.join("b/1");
        let b = root.join("b");
        for p in [&b1, &b, &a1, &a, root] {
            let is_dir = p != &b1;
            // SAFETY: 路径经 nul() 保证 NUL 结尾。
            let r = if is_dir {
                unsafe { libc::rmdir(nul(p).as_ptr().cast()) }
            } else {
                unsafe { libc::unlink(nul(p).as_ptr().cast()) }
            };
            let _ = r; // 清理失败不致命
        }
    }
}

/// mkdtemp 封装：模板末尾 XXXXXX 由 libc 填充。
fn mkdtemp(template: PathBuf) -> Option<PathBuf> {
    let mut bytes = template.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: bytes 为 NUL 结尾的可变缓冲，末 6 字节是 X。
    let p = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: mkdtemp 成功时 p 指向填充后的 NUL 结尾路径。
    let cstr = unsafe { std::ffi::CStr::from_ptr(p) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes())))
}

/// lstat 取 (mtime_sec, mtime_nsec)。
fn lstat(path: &Path) -> Option<(i64, i64)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: 路径经 nul() 保证 NUL 结尾，st 合法缓冲。
    if unsafe { libc::lstat(nul(path).as_ptr().cast(), &mut st) } != 0 {
        return None;
    }
    Some((st.st_mtime, st.st_mtime_nsec))
}

/// mtime 是否变化（对齐 StatChanged：StatEq 取反）。
fn stat_changed(path: &Path, prev: &(i64, i64)) -> bool {
    lstat(path).map(|cur| &cur != prev).unwrap_or(false)
}

/// 清理 10 秒以上的残留探针目录（对齐 RemoveStaleDirs）。
/// 实现要点：readdir gitdir，名字前缀 `.gitstatus.` 且 mtime 早于 now-10s
/// 的目录，按 a/1、a、b/1、b、根的逆序删除。
fn remove_stale_dirs(gitdir: &Path) {
    // SAFETY: 路径经 nul() 保证 NUL 结尾。
    let dir_fd = unsafe {
        libc::open(
            nul(gitdir).as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if dir_fd < 0 {
        return;
    }
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut names: Vec<Vec<u8>> = Vec::new();
    let dup_fd = unsafe { libc::dup(dir_fd) };
    if dup_fd >= 0 {
        let dirp = unsafe { libc::fdopendir(dup_fd) };
        if !dirp.is_null() {
            loop {
                let ent = unsafe { libc::readdir(dirp) };
                if ent.is_null() {
                    break;
                }
                let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
                if name.starts_with(b".gitstatus.") {
                    names.push(name.to_vec());
                }
            }
            unsafe { libc::closedir(dirp) };
        } else {
            unsafe { libc::close(dup_fd) };
        }
    }
    for name in names {
        let mut path = gitdir.to_path_buf();
        path.push(std::ffi::OsStr::from_bytes(&name));
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: fstatat 语义标准。
        let ok = unsafe {
            libc::fstatat(
                dir_fd,
                std::ffi::CString::new(name.clone())
                    .unwrap()
                    .as_bytes_with_nul()
                    .as_ptr()
                    .cast(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if !ok || st.st_mtime + 10 > now {
            continue;
        }
        // 逆序删除：a/1、a、b/1、b、根（路径经 nul() 保证 NUL 结尾）
        let mut sub = path.clone();
        sub.push("a/1");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("a");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("b/1");
        unsafe { libc::unlink(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("b");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        unsafe { libc::rmdir(nul(&path).as_ptr().cast()) };
    }
    // SAFETY: close 语义标准。
    unsafe { libc::close(dir_fd) };
}
