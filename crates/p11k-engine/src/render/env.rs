//! Environment and system data: user, host, ssh, memory, swap, disk, battery, network and the
//! values the environment segments render.
//!
//! The lookups are cached where p10k caches them. `meminfo_kb` reads `/proc/meminfo` once per
//! call; the callers read the same file for related fields.

use std::cell::RefCell;

use super::run_cmd;

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

pub(super) fn env() -> EnvInfo {
    ENV.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(detect_env());
        }
        c.borrow().as_ref().unwrap().clone()
    })
}

pub(super) fn detect_env() -> EnvInfo {
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok())
        .unwrap_or_default();
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("HOST").ok())
        .unwrap_or_default();
    let ssh = ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
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
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Level segments (ranger/nnf/lf/yazi): value 0 or empty → empty (not inside that program).
pub(super) fn level_text(name: &str) -> String {
    env_var(name).filter(|v| v != "0").unwrap_or_default()
}

pub(super) fn level_icon(name: &str, icon: &str) -> Option<String> {
    env_var(name).filter(|v| v != "0").map(|_| icon.into())
}

pub(super) fn env_has_proxy() -> bool {
    ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
        .any(|k| env_var(k).is_some())
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

pub(super) fn in_nix_shell() -> bool {
    env_var("IN_NIX_SHELL").is_some_and(|v| v == "pure" || v == "impure")
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

/// Reads one field of /proc/meminfo (KiB).
pub(super) fn meminfo_kb(name: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(&format!("{name}:"))
                .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        })
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
    meminfo_kb("MemAvailable")
        .map(|kb| human_bytes(kb * 1024))
        .unwrap_or_default()
}

/// Used swap.
pub(super) fn swap() -> String {
    let total = meminfo_kb("SwapTotal").unwrap_or(0);
    let free = meminfo_kb("SwapFree").unwrap_or(0);
    let used = total.saturating_sub(free);
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
    std::env::var("HOME").unwrap_or_default()
}

pub(crate) fn current_dir() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
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
    run_cmd("curl", &["-s", "--max-time", "4", "https://v4.ident.me/"]).unwrap_or_default()
}
