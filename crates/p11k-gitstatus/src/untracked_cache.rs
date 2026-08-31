//! untracked cache（对齐 `check_dir_mtime.cc:99-143` 与 `index.cc:231-243`）。
//!
//! # 背景
//!
//! git 的 untracked cache 依赖一个文件系统行为：**子目录内容变化会更新
//! 父目录的 mtime**。并非所有文件系统都保证（部分网络/覆盖文件系统
//! 不保证），错误启用会导致 untracked 文件漏报——正确性优先于性能。
//!
//! 原版的做法：仓库首次构造时**异步**跑一个 CheckDirMtime 探针——
//! 创建临时目录、往其中写子文件，检查父目录 mtime 是否随之变化：
//! - 行为支持 → 目录 mtime 未变即可复用上次"该目录下无 untracked"的
//!   结论，跳过 readdir；
//! - 行为不支持 → 禁用缓存，每次全量遍历。
//!
//! # 复刻要求
//!
//! 探针行为必须复刻（含异步执行、结论缓存），否则在特殊文件系统上
//! 会出现 untracked 漏报。探针结果按"当前文件系统/挂载点"记忆，
//! 换工作区（不同挂载）需重新探测。

/// 一个仓库工作区的 untracked cache 状态。
pub struct UntrackedCache {
    /// 探针结论：当前文件系统是否支持"子目录变化更新父目录 mtime"。
    pub supported: bool,
    // TODO(实现者)：目录 mtime 记录表——
    // HashMap<目录路径, (mtime_sec, mtime_nsec)>，只存"确认无 untracked"的目录。
}

impl UntrackedCache {
    /// 启动探针：验证文件系统行为，构造出带结论的缓存。
    ///
    /// 实现要点：
    /// - 在目标文件系统上创建临时目录（如 `$TMPDIR` 同挂载点），
    ///   记录父目录 mtime → 写入子文件 → 再读父目录 mtime 比较。
    /// - 异步执行：探针不阻塞首次请求；结论未出时先按"支持"处理
    ///   （与 git 的默认一致），结论出来后再纠正。
    /// - 探针失败（无法创建临时文件）时按"不支持"处理，安全降级。
    pub fn probe_support() -> UntrackedCache {
        todo!("实现：临时目录探针，异步或同步均可（注释说明选择）")
    }

    /// 判断某目录"无 untracked"的结论是否仍有效。
    ///
    /// 实现要点：缓存命中且目录当前 mtime 与记录一致 → true；
    /// 未命中 → false（调用方全量 readdir 后可用结果回填缓存）。
    pub fn is_valid(&self, dir_path: &[u8], dir_mtime: (i64, i64)) -> bool {
        let _ = (dir_path, dir_mtime);
        todo!("实现：查表比较 mtime；未命中返回 false")
    }
}
