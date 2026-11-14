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
    let mut out: Vec<PathBuf> = fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("BAT")))
        .collect();
    out.sort();
    out
}

fn read(p: PathBuf) -> String {
    fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_else(|_| "?".into())
}

pub fn get_battery(_: &Context, _: &Args) -> ToolResult {
    let bats = batteries();
    if bats.is_empty() {
        return Ok("No battery found.".into());
    }
    Ok(bats.iter()
        .map(|b| format!("{}: {}% ({})", b.file_name().unwrap().to_string_lossy(), read(b.join("capacity")), read(b.join("status"))))
        .collect::<Vec<_>>()
        .join("; "))
}

fn disk(path: &str) -> Option<String> {
    let info = gio::File::for_path(path)
        .query_filesystem_info("filesystem::free,filesystem::size", None::<&gio::Cancellable>).ok()?;
    let gib = |k: &str| info.attribute_uint64(k) / (1 << 30);
    Some(format!("{} GiB free of {} GiB", gib("filesystem::free"), gib("filesystem::size")))
}

pub fn get_system_info(_: &Context, _: &Args) -> ToolResult {
    let mut parts = vec![format!("os: {}", os_name()), format!("kernel: {}", kernel())];
    let host = glib::host_name();
    parts.push(format!("host: {host}"));
    if let Some(up) = fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()) {
        let s = up as u64;
        let days = s / 86400;
        let hms = format!("{}:{:02}:{:02}", (s % 86400) / 3600, (s % 3600) / 60, s % 60);
        parts.push(format!("uptime: {}", if days > 0 { format!("{days} days, {hms}") } else { hms }));
    }
    if let (Some(total), Some(avail)) = (meminfo_kib("MemTotal"), meminfo_kib("MemAvailable")) {
        parts.push(format!("memory: {} MiB used of {} MiB", (total - avail) / 1024, total / 1024));
    }
    let home = crate::paths::home();
    for (label, path) in [("disk /", "/".to_string()), ("disk home", home.to_string_lossy().to_string())] {
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
