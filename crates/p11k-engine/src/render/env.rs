//! Environment and system data: user, host, ssh, memory, swap, disk, battery, network and the
//! values the environment segments render.
//!
//! The lookups are cached where p10k caches them. The memory segments share one
//! `/proc/meminfo` snapshot; command output is cached by [`run_cmd`] and cleared per prompt.

use std::cell::RefCell;

use super::segments::{run_cmd, run_cmd_with_timeout};

#[derive(Clone)]
pub(super) struct EnvInfo {
    pub(super) user: String,
    pub(super) host: String,
    pub(super) ssh: bool,
    pub(super) root: bool,
}

thread_local! {
    static ENV: RefCell<Option<EnvInfo>> = const { RefCell::new(None) };
}

/// Environment access: the process environment in production, an injected map in tests.
pub(super) trait Env: Sync {
    fn var(&self, name: &str) -> Option<String>;
}

/// The process environment.
pub(super) struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

#[cfg(test)]
thread_local! {
    static INJECTED: RefCell<Option<Box<dyn Env>>> = const { RefCell::new(None) };
}

/// Reads one variable through the current environment access.
fn var(name: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(value) = INJECTED.with(|env| env.borrow().as_ref().map(|env| env.var(name))) {
        return value;
    }
    ProcessEnv.var(name)
}

/// Runs `f` with the given environment: the injected map replaces the process environment, so a
/// test does not depend on the variables the machine ships. Also clears the memoised
/// [`EnvInfo`] before and after.
#[cfg(test)]
pub(super) fn with_env<R>(vars: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
    struct MapEnv(std::collections::BTreeMap<String, String>);

    impl Env for MapEnv {
        fn var(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    let map = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    INJECTED.with(|env| *env.borrow_mut() = Some(Box::new(MapEnv(map))));
    ENV.with(|c| *c.borrow_mut() = None);
    let out = f();
    INJECTED.with(|env| *env.borrow_mut() = None);
    ENV.with(|c| *c.borrow_mut() = None);
    out
}

pub(super) fn env() -> EnvInfo {
    ENV.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(detect_env());
        }
        c.borrow().as_ref().unwrap().clone()
    })
}

pub(super) fn detect_env() -> EnvInfo {
    let user = var("USER").or_else(|| var("LOGNAME")).unwrap_or_default();
    let host = var("HOSTNAME").or_else(|| var("HOST")).unwrap_or_default();
    let ssh = ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|k| var(k).is_some_and(|v| !v.is_empty()));
    let root = unsafe { libc::geteuid() } == 0;
    EnvInfo {
        user,
        host,
        ssh,
        root,
    }
}

pub(super) fn user_name() -> String {
    env().user
}

/// context: SSH → `user@host`; local root → `user`; local regular user → empty (hidden).
pub(super) fn context_text() -> String {
    let e = env();
    if e.ssh {
        format!("{}@{}", e.user, e.host)
    } else if e.root {
        e.user
    } else {
        String::new()
    }
}

pub(super) fn host_text() -> String {
    let e = env();
    if e.ssh || e.root {
        e.host
    } else {
        String::new()
    }
}

pub(super) fn root_indicator_text() -> String {
    if env().root {
        "#".into()
    } else {
        String::new()
    }
}

/// Reads an env var; missing or empty → None.
pub(super) fn env_var(name: &str) -> Option<String> {
    var(name).filter(|v| !v.is_empty())
}

/// Level segments (ranger/nnf/lf/yazi): value 0 or empty → empty (not inside that program).
pub(super) fn level_text(name: &str) -> String {
    env_var(name).filter(|v| v != "0").unwrap_or_default()
}

pub(super) fn level_icon(name: &str, icon: &str) -> Option<String> {
    env_var(name).filter(|v| v != "0").map(|_| icon.into())
}

/// proxy segment: host:port of the first non-empty proxy (scheme/user@/path stripped).
pub(super) fn proxy_text() -> String {
    for key in ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
    {
        let Some(v) = env_var(key) else { continue };
        let v = v.split_once("://").map(|(_, rest)| rest).unwrap_or(&v);
        let v = v.rsplit('@').next().unwrap_or(v);
        let v = v.split(['/', '?']).next().unwrap_or(v);
        if !v.is_empty() {
            return v.to_string();
        }
    }
    String::new()
}

/// openfoam segment: p10k shows `OF: <version>`.
pub(super) fn openfoam_text() -> String {
    env_var("WM_PROJECT_VERSION")
        .map(|v| format!("OF: {v}"))
        .unwrap_or_default()
}

// nix_shell: as in p10k only pure/impure in `IN_NIX_SHELL` count; any other value is inactive.
pub(super) fn nix_shell_text() -> String {
    env_var("IN_NIX_SHELL")
        .filter(|v| v == "pure" || v == "impure")
        .unwrap_or_default()
}

// System resource segments: read /proc and /sys (battery), or run `df`; hidden when the data
// source is missing.
pub(super) fn human_bytes(bytes: u64) -> String {
    let mut val = bytes as f64;
    let mut i = 0;
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    while val >= 1024.0 && i < UNITS.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{val}B")
    } else {
        format!("{val:.1}{}", UNITS[i])
    }
}

/// The fields the memory segments need, read from one /proc/meminfo snapshot.
struct MemInfo {
    available_kb: u64,
    swap_total_kb: u64,
    swap_free_kb: u64,
}

impl MemInfo {
    /// Parse the snapshot; missing fields read as 0.
    fn parse(text: &str) -> MemInfo {
        let field = |name: &str| -> u64 {
            text.lines()
                .find_map(|l| {
                    l.strip_prefix(name)
                        .and_then(|rest| rest.trim_start().strip_prefix(':'))
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|v| v.parse().ok())
                })
                .unwrap_or(0)
        };
        MemInfo {
            available_kb: field("MemAvailable"),
            swap_total_kb: field("SwapTotal"),
            swap_free_kb: field("SwapFree"),
        }
    }
}

/// One snapshot of /proc/meminfo: ram and swap read it together instead of three times.
fn meminfo() -> MemInfo {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    MemInfo::parse(&text)
}

/// Load: current system load (the first value of /proc/loadavg).
pub(super) fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
        .unwrap_or_default()
}

/// Free memory (MemAvailable, human readable).
pub(super) fn ram() -> String {
    let kb = meminfo().available_kb;
    if kb == 0 {
        String::new()
    } else {
        human_bytes(kb * 1024)
    }
}

/// Used swap.
pub(super) fn swap() -> String {
    let info = meminfo();
    let used = info.swap_total_kb.saturating_sub(info.swap_free_kb);
    if used == 0 {
        String::new()
    } else {
        human_bytes(used * 1024)
    }
}

/// Used percentage of the partition holding the current directory.
pub(super) fn disk_usage(cwd: &str) -> String {
    run_cmd("df", &["-P", cwd])
        .and_then(|s| {
            s.lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(4))
                .map(|v| v.trim_end_matches('%').to_string())
        })
        .unwrap_or_default()
}

/// Battery: percentage + status (e.g. `Charging 87%`); hidden when no battery file exists.
pub(super) fn battery() -> String {
    let bat = std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .find(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        });
    let Some(bat) = bat else { return String::new() };
    let bat = bat.path();
    let cap = std::fs::read_to_string(bat.join("capacity"))
        .ok()
        .map(|s| s.trim().to_string());
    let status = std::fs::read_to_string(bat.join("status"))
        .ok()
        .map(|s| s.trim().to_string());
    match (cap, status) {
        (Some(c), Some(s)) => format!("{s} {c}%"),
        (Some(c), None) => format!("{c}%"),
        _ => String::new(),
    }
}

pub(crate) fn home() -> String {
    var("HOME").unwrap_or_default()
}

pub(super) fn detect_virt() -> String {
    run_cmd("systemd-detect-virt", &[])
        .filter(|v| v != "none")
        .unwrap_or_default()
}

/// First non-loopback local IPv4 (`ip -4 addr show`, skipping 127.0.0.1).
pub(super) fn ip_text() -> String {
    run_cmd("ip", &["-4", "addr", "show"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("inet ") && !l.contains("127.0.0.1"))
        .find_map(|l| l.split("inet ").nth(1)?.split('/').next())
        .unwrap_or_default()
        .to_string()
}

/// VPN interface IP (the IPv4 of tailscale/wg/tun/zt).
pub(super) fn vpn_ip_text() -> String {
    let out = run_cmd("ip", &["addr", "show"]).unwrap_or_default();
    let mut ifname = String::new();
    for line in out.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("inet ") {
            if is_vpn_if(&ifname) {
                let ip = rest.split('/').next().unwrap_or("");
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        } else if let Some(name) = line.split(": ").nth(1).and_then(|s| s.split(':').next()) {
            ifname = name.to_string();
        }
    }
    String::new()
}

pub(super) fn is_vpn_if(name: &str) -> bool {
    ["tailscale", "wg", "tun", "zt"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// WiFi: interface name + link quality from /proc/net/wireless.
pub(super) fn wifi_text() -> String {
    std::fs::read_to_string("/proc/net/wireless")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.contains(':')) // data line (`iface: ...`)
                .map(|l| l.to_string())
        })
        .and_then(|l| {
            let mut p = l.split_whitespace();
            let ifn = p.next()?.trim_end_matches(':').to_string();
            let _ = p.next(); // flags
            let q = p.next()?; // link quality
            Some(format!("{ifn} {}", q.trim_end_matches('.')))
        })
        .unwrap_or_default()
}

/// Public IP: curl query (cached by run_cmd; synchronous on the first call, cached afterwards).
pub(super) fn public_ip_text() -> String {
    // The public address is optional; a slow network must not hold the first prompt for
    // longer than the command timeout (the first prompt is painted synchronously).
    run_cmd_with_timeout(
        "curl",
        &["-s", "--max-time", "4", "https://v4.ident.me/"],
        std::time::Duration::from_secs(1),
    )
    .unwrap_or_default()
}
