//! Application intents (freedesktop.org Application Intents Specification):
//! apps declare `Implements=` in their desktop file, often a D-Bus interface
//! such as org.freedesktop.FileManager1, and `intentapps.list` picks the
//! default app per intent, the way mimeapps.list does for file types.

use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::files::{home_path, short};
use super::run::dbus_call;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

pub const FILE_MANAGER: &str = "org.freedesktop.FileManager1";

fn desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':')
        .filter(|d| !d.is_empty()).map(|d| d.to_ascii_lowercase()).collect()
}

fn env_dirs(var: &str, default: &str) -> Vec<PathBuf> {
    std::env::var(var).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into())
        .split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
}

/// The intentapps.list files, most important first (spec section 2).
pub fn list_files() -> Vec<PathBuf> {
    let config_home = crate::paths::config_dir().parent().map(Path::to_path_buf).unwrap_or_default();
    let config_dirs = env_dirs("XDG_CONFIG_DIRS", "/etc/xdg");
    let data_dirs = env_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share");
    let desks = desktops();
    let mut out = vec![];
    let mut push = |dir: &Path, sub: &str| {
        for d in &desks {
            out.push(dir.join(sub).join(format!("{d}-intentapps.list")));
        }
        out.push(dir.join(sub).join("intentapps.list"));
    };
    push(&config_home, "");
    for d in &config_dirs {
        push(d, "");
    }
    for d in &data_dirs {
        push(d, "applications");
    }
    out
}

/// Desktop IDs listed for an intent under [Default Applications] in one file.
pub fn defaults_in(text: &str, intent: &str) -> Vec<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Default Applications]";
        } else if in_group
            && let Some((k, v)) = line.split_once('=')
                && k.trim() == intent {
                    return v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
                }
    }
    vec![]
}

fn implements(app: &gio::DesktopAppInfo, intent: &str) -> bool {
    app.string("Implements").is_some_and(|v| v.split(';').any(|i| i.trim() == intent))
}

/// The user's app for an intent: the first listed one that's installed and
/// really implements it, else the first implementation by desktop ID.
pub fn default_for(intent: &str) -> Option<gio::DesktopAppInfo> {
    for file in list_files() {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for id in defaults_in(&text, intent) {
            if let Some(app) = gio::DesktopAppInfo::new(&id).filter(|a| implements(a, intent)) {
                return Some(app);
            }
        }
    }
    let mut all = gio::DesktopAppInfo::implementations(intent);
    all.sort_by_key(|a| a.id().map(|s| s.to_string()).unwrap_or_default());
    all.into_iter().next()
}

/// Open the file manager on a file's folder with the file selected.
pub fn show_in_files(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    let uri = gio::File::for_path(&path).uri().to_string();
    dbus_call(false, FILE_MANAGER, "/org/freedesktop/FileManager1", FILE_MANAGER, "ShowItems",
              Some(&(vec![uri], "").to_variant()), None, 10000)
        .map_err(|e| ToolError(format!("the file manager didn't answer: {}", e.message())))?;
    Ok(format!("Showing {} in Files.", short(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_default_applications() {
        let text = "[Default Applications]\norg.freedesktop.FileManager1=org.gnome.Nautilus.desktop;nemo.desktop;\n\
                    [org.freedesktop.FileManager1]\nhttp=x.desktop;\n";
        assert_eq!(defaults_in(text, FILE_MANAGER), ["org.gnome.Nautilus.desktop", "nemo.desktop"]);
        assert!(defaults_in(text, "com.example.Calculator1").is_empty());
        assert!(defaults_in("[org.freedesktop.FileManager1]\norg.freedesktop.FileManager1=a;", FILE_MANAGER).is_empty());
    }

    #[test]
    fn lookup_order_follows_the_spec() {
        let _h = crate::store::testenv::temp_home();
        // SAFETY: tests that touch the environment hold the temp_home lock.
        unsafe {
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME:Unity");
            std::env::set_var("XDG_CONFIG_DIRS", "/etc/xdg");
            std::env::set_var("XDG_DATA_DIRS", "/usr/share");
        }
        let files: Vec<String> = list_files().iter().map(|p| p.display().to_string()).collect();
        let home = crate::paths::home();
        assert_eq!(files[0], format!("{}/config/gnome-intentapps.list", home.display()));
        assert_eq!(files[2], format!("{}/config/intentapps.list", home.display()));
        assert_eq!(files[3], "/etc/xdg/gnome-intentapps.list");
        assert_eq!(files.last().unwrap(), "/usr/share/applications/intentapps.list");
    }
}
