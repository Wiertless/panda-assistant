//! A short description of this computer for the system prompt.
//!
//! Kept to a few lines: local models run with small contexts. Live values the
//! user may ask about later (battery, free disk) stay behind tools.

use std::fs;
use std::path::Path;

fn read(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn meminfo_kib(key: &str) -> Option<u64> {
    read("/proc/meminfo")
        .lines()
        .find(|l| l.starts_with(&format!("{key}:")))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
}

pub fn cpu_name() -> String {
    read("/proc/cpuinfo")
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

pub fn os_name() -> String {
    read("/etc/os-release")
        .lines()
        .find_map(|l| {
            l.strip_prefix("PRETTY_NAME=")
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "Linux".into())
}

pub fn kernel() -> String {
    read("/proc/sys/kernel/osrelease")
}

pub fn system_context() -> String {
    let mut lines = vec![];
    let (vendor, product) = (
        read("/sys/class/dmi/id/sys_vendor"),
        read("/sys/class/dmi/id/product_name"),
    );
    if !product.is_empty() {
        lines.push(format!("Computer: {vendor} {product}").trim().to_string());
    }
    let threads = std::thread::available_parallelism()
        .map(|n| format!(" ({n} threads)"))
        .unwrap_or_default();
    lines.push(format!("CPU: {}{threads}", cpu_name()));
    if let Some(total) = meminfo_kib("MemTotal") {
        let avail = meminfo_kib("MemAvailable").unwrap_or(0);
        lines.push(format!(
            "Memory: {:.0} GiB total, {:.1} GiB free right now",
            total as f64 / 1048576.0,
            avail as f64 / 1048576.0
        ));
    }
    if Path::new("/dev/accel/accel0").exists() {
        lines.push("NPU: AMD XDNA (you are running on it through FastFlowLM)".into());
    }
    lines.push(format!(
        "OS: {}, kernel {}, desktop {}",
        os_name(),
        kernel(),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into())
    ));
    if let Some(bat) = crate::tools::system::battery_dir() {
        let b = bat.to_string_lossy();
        lines.push(format!(
            "Battery: {}% ({})",
            read(&format!("{b}/capacity")),
            read(&format!("{b}/status")).to_lowercase()
        ));
    }
    let home = crate::paths::home();
    let folders: Vec<String> = crate::tools::files::user_dirs()
        .into_iter()
        .filter_map(|(role, path)| {
            let label = match role.as_str() {
                "download" => "Downloads",
                "documents" => "Documents",
                "pictures" => "Pictures",
                "music" => "Music",
                "videos" => "Videos",
                "desktop" => "Desktop",
                _ => return None,
            };
            Some(format!("{label} = {}", crate::tools::files::short(&path)))
        })
        .collect();
    lines.push(format!(
        "Home folder: {}{}",
        home.display(),
        if folders.is_empty() {
            String::new()
        } else {
            format!(" (folders: {})", folders.join(", "))
        }
    ));
    let apps = crate::tools::apps::default_apps();
    if !apps.is_empty() {
        let list: Vec<String> = apps.iter().map(|(r, n)| format!("{r}: {n}")).collect();
        lines.push(format!("Default apps: {}", list.join(", ")));
    }
    // Devices and networks by name (as Home Assistant lists a home's devices),
    // asked in the background so a slow service can't hold up the answer.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::tools::connect::known_names());
    });
    lines.extend(
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .unwrap_or_default(),
    );
    if let Ok(today) = glib::DateTime::now_local().and_then(|d| d.format("%A %Y-%m-%d")) {
        lines.push(format!("Today: {today}"));
    }
    format!(
        "About this computer:\n{}",
        lines
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
