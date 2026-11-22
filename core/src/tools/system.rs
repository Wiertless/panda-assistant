//! Reading the computer's state: battery, system info, time.

use std::fs;
use std::path::PathBuf;

use gio::prelude::*;

use crate::context::{kernel, meminfo_kib, os_name};
use crate::tool::{Args, Context, ToolResult};

pub fn battery_dir() -> Option<PathBuf> {
    batteries().into_iter().next()
}

fn batteries() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("BAT"))
        })
        .collect();
    out.sort();
    out
}

fn read(p: PathBuf) -> String {
    fs::read_to_string(p)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into())
}

pub fn get_battery(_: &Context, _: &Args) -> ToolResult {
    let bats = batteries();
    if bats.is_empty() {
        return Ok("No battery found.".into());
    }
    Ok(bats
        .iter()
        .map(|b| {
            format!(
                "{}: {}% ({})",
                b.file_name().unwrap().to_string_lossy(),
                read(b.join("capacity")),
                read(b.join("status"))
            )
        })
        .collect::<Vec<_>>()
        .join("; "))
}

fn disk(path: &str) -> Option<String> {
    let info = gio::File::for_path(path)
        .query_filesystem_info(
            "filesystem::free,filesystem::size",
            None::<&gio::Cancellable>,
        )
        .ok()?;
    let gib = |k: &str| info.attribute_uint64(k) / (1 << 30);
    Some(format!(
        "{} GiB free of {} GiB",
        gib("filesystem::free"),
        gib("filesystem::size")
    ))
}

pub fn get_system_info(_: &Context, _: &Args) -> ToolResult {
    let mut parts = vec![
        format!("os: {}", os_name()),
        format!("kernel: {}", kernel()),
    ];
    let host = glib::host_name();
    parts.push(format!("host: {host}"));
    if let Some(up) = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
    {
        let s = up as u64;
        let days = s / 86400;
        let hms = format!(
            "{}:{:02}:{:02}",
            (s % 86400) / 3600,
            (s % 3600) / 60,
            s % 60
        );
        parts.push(format!(
            "uptime: {}",
            if days > 0 {
                format!("{days} days, {hms}")
            } else {
                hms
            }
        ));
    }
    if let (Some(total), Some(avail)) = (meminfo_kib("MemTotal"), meminfo_kib("MemAvailable")) {
        parts.push(format!(
            "memory: {} MiB used of {} MiB",
            (total - avail) / 1024,
            total / 1024
        ));
    }
    let home = crate::paths::home();
    for (label, path) in [
        ("disk /", "/".to_string()),
        ("disk home", home.to_string_lossy().to_string()),
    ] {
        if let Some(d) = disk(&path) {
            parts.push(format!("{label}: {d}"));
        }
    }
    Ok(parts.join("; "))
}

pub fn get_datetime(_: &Context, _: &Args) -> ToolResult {
    glib::DateTime::now_local()
        .and_then(|d| d.format("%A %Y-%m-%d %H:%M %Z"))
        .map(|s| s.to_string())
        .map_err(|_| "couldn't read the clock".into())
}

/// What some local servers are, by program name (the port alone can change).
const KNOWN_SERVERS: [(&str, &str); 4] = [
    (
        "lemond",
        "Lemonade Server; its OpenAI-compatible API is at /api/v1 on its main port, 13305 by default",
    ),
    (
        "flm",
        "FastFlowLM, serving a model on the NPU; OpenAI-compatible API at /v1",
    ),
    ("ollama", "Ollama; its API is at /api on this port"),
    ("assistantd", "this assistant's own service"),
];

/// A /proc/net address ("0100007F" or 32 hex digits) as text.
fn proc_net_addr(hex: &str) -> String {
    if hex.len() == 8 {
        let n = u32::from_str_radix(hex, 16).unwrap_or(0);
        return std::net::Ipv4Addr::from(n.swap_bytes()).to_string();
    }
    let mut bytes = [0u8; 16];
    for (i, chunk) in hex.as_bytes().chunks(8).take(4).enumerate() {
        let word = u32::from_str_radix(std::str::from_utf8(chunk).unwrap_or("0"), 16).unwrap_or(0);
        bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    let a = std::net::Ipv6Addr::from(bytes);
    match a.to_ipv4_mapped() {
        Some(v4) => v4.to_string(),
        None => format!("[{a}]"),
    }
}

/// Listening TCP sockets as (inode, "address:port").
fn listening() -> Vec<(u64, String)> {
    let mut out = vec![];
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(table) = fs::read_to_string(file) else {
            continue;
        };
        for line in table.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            // 0A: LISTEN
            if f.len() < 10 || f[3] != "0A" {
                continue;
            }
            let Some((addr, port)) = f[1].split_once(':') else {
                continue;
            };
            let port = u16::from_str_radix(port, 16).unwrap_or(0);
            out.push((
                f[9].parse().unwrap_or(0),
                format!("{}:{port}", proc_net_addr(addr)),
            ));
        }
    }
    out
}

/// Which program owns each socket inode (only the user's own programs are readable).
fn socket_owners() -> std::collections::HashMap<u64, String> {
    let mut owners = std::collections::HashMap::new();
    let Ok(procs) = fs::read_dir("/proc") else {
        return owners;
    };
    for p in procs.flatten() {
        let dir = p.path();
        let Ok(fds) = fs::read_dir(dir.join("fd")) else {
            continue;
        };
        let name = fs::read_to_string(dir.join("comm")).unwrap_or_default();
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else {
                continue;
            };
            let link = link.to_string_lossy();
            if let Some(inode) = link
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse().ok())
            {
                owners.insert(inode, name.trim().to_string());
            }
        }
    }
    owners
}

/// Local servers: which programs listen on which address and port.
pub fn list_servers(_: &Context, _: &Args) -> ToolResult {
    let owners = socket_owners();
    let mut lines: Vec<String> = listening()
        .into_iter()
        .map(|(inode, addr)| {
            let name = owners
                .get(&inode)
                .cloned()
                .unwrap_or_else(|| "a system service".into());
            let what = KNOWN_SERVERS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, w)| format!(" ({w})"))
                .unwrap_or_default();
            format!("- {name}: {addr}{what}")
        })
        .collect();
    lines.sort();
    lines.dedup();
    if lines.is_empty() {
        return Ok("No program is listening for connections.".into());
    }
    Ok(format!(
        "Programs listening for connections (127.0.0.1 and [::1]: only this computer; 0.0.0.0 and [::]: \
the network too; use http://localhost:PORT from here):\n{}",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod server_tests {
    use super::*;

    #[test]
    fn proc_net_addresses() {
        assert_eq!(proc_net_addr("0100007F"), "127.0.0.1");
        assert_eq!(proc_net_addr("00000000"), "0.0.0.0");
        assert_eq!(proc_net_addr("00000000000000000000000001000000"), "[::1]");
        assert_eq!(proc_net_addr("00000000000000000000000000000000"), "[::]");
    }
}
