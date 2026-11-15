//! Files in the home folder. Paths may be absolute, ~/…, relative to home,
//! or start with a folder name such as "Downloads", which maps to the real
//! folder in any language (~/Pobrane in Polish). Everything stays inside home
//! and away from hidden files, which hold keys, tokens and browser profiles.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use gio::prelude::*;

use crate::paths::home;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const USER_DIRS: [(&str, glib::UserDirectory); 8] = [
    ("desktop", glib::UserDirectory::Desktop), ("documents", glib::UserDirectory::Documents),
    ("download", glib::UserDirectory::Downloads), ("music", glib::UserDirectory::Music),
    ("pictures", glib::UserDirectory::Pictures), ("public_share", glib::UserDirectory::PublicShare),
    ("templates", glib::UserDirectory::Templates), ("videos", glib::UserDirectory::Videos),
];

fn dir_alias(word: &str) -> Option<&'static str> {
    Some(match word {
        "downloads" | "download" => "download",
        "documents" | "docs" => "documents",
        "desktop" => "desktop",
        "music" => "music",
        "pictures" | "photos" => "pictures",
        "videos" | "movies" => "videos",
        "public" => "public_share",
        "templates" => "templates",
        _ => return None,
    })
}

fn real_home() -> PathBuf {
    fs::canonicalize(home()).unwrap_or_else(|_| home())
}

/// {role: folder} for the XDG user folders that exist.
pub fn user_dirs() -> Vec<(String, PathBuf)> {
    let home = real_home();
    USER_DIRS.iter()
        .filter_map(|(role, dir)| {
            let p = glib::user_special_dir(*dir)?;
            (p.is_dir() && fs::canonicalize(&p).ok()? != home).then(|| (role.to_string(), p))
        })
        .collect()
}

/// Normalize `..` and `.` without touching the disk (for paths that may not exist).
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn resolve(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| lexical(path))
}

pub fn hidden(path: &Path, home: &Path) -> bool {
    match path.strip_prefix(home) {
        Ok(rel) => rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')),
        Err(_) => true,
    }
}

pub fn short(path: &Path) -> String {
    let home = real_home();
    match resolve(path).strip_prefix(&home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".into(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn home_path(raw: &str, must_exist: bool) -> Result<PathBuf, ToolError> {
    let home = real_home();
    let text = raw.trim().trim_matches(|c| c == '\'' || c == '"');
    let normalized = text.replace('\\', "/");
    let (first, rest) = normalized.split_once('/').unwrap_or((&normalized, ""));
    let word = first.to_lowercase();
    let word = word.trim_start_matches("my ").trim_start_matches("~/").trim();
    let dirs = user_dirs();
    let role_dir = dir_alias(word).and_then(|r| dirs.iter().find(|(role, _)| role == r).map(|(_, p)| p.clone()));
    let path = if let Some(dir) = role_dir {
        dir.join(rest)
    } else if matches!(text, "" | "~" | "home" | "home folder") {
        home.clone()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else if Path::new(text).is_absolute() {
        PathBuf::from(text)
    } else {
        home.join(text)
    };
    let path = resolve(&path);
    if path != home && !path.starts_with(&home) {
        return Err("only files in your home folder are allowed".into());
    }
    if hidden(&path, &home) {
        return Err("hidden files and folders are off-limits".into());
    }
    if must_exist && !path.exists() {
        return loosely(&path).ok_or_else(|| missing(&path).into());
    }
    Ok(path)
}

/// "X doesn't exist", plus what is there: similar names in the nearest folder
/// that does exist, so the model can correct itself.
fn missing(path: &Path) -> String {
    let mut msg = format!("{} doesn't exist", short(path));
    let Some(dir) = path.ancestors().skip(1).find(|d| d.is_dir()) else { return msg };
    let wanted = path.strip_prefix(dir).ok().and_then(|r| r.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
    let names: Vec<String> = fs::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.')).collect();
    let close = super::close_to(&wanted, &names, 6);
    if !close.is_empty() {
        msg += &format!(". Similar in {}: {}", short(dir), close.join(", "));
    } else {
        msg += ". find_files searches for a file by name";
    }
    msg
}

/// Small models retype long names ("Zrzut ekranu z 2026-11-15 01-02-03.png")
/// with the case, spaces or punctuation a bit off: take the one file in the
/// same folder whose name matches when only letters and digits count.
fn loosely(path: &Path) -> Option<PathBuf> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let want = key(&path.file_name()?.to_string_lossy());
    if want.is_empty() {
        return None;
    }
    let mut hits = fs::read_dir(path.parent()?).ok()?.flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && key(&e.file_name().to_string_lossy()) == want);
    let hit = hits.next()?;
    hits.next().is_none().then(|| hit.path())
}

pub fn size(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    n.to_string()
}

fn date(secs: u64) -> String {
    glib::DateTime::from_unix_local(secs as i64).and_then(|d| d.format("%Y-%m-%d")).map(|s| s.to_string())
        .unwrap_or_default()
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn list_folder(_: &Context, args: &Args) -> ToolResult {
    let raw = arg_str(args, "path");
    let folder = home_path(if raw.is_empty() { "~" } else { &raw }, true)?;
    if !folder.is_dir() {
        return Err(format!("{} isn't a folder", short(&folder)).into());
    }
    let mut entries: Vec<(bool, String, Option<fs::Metadata>)> = fs::read_dir(&folder).map_err(|e| e.to_string())?
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().to_string(), e))
        .filter(|(n, _)| !n.starts_with('.'))
        .map(|(n, e)| (e.path().is_dir(), n, e.metadata().ok()))
        .collect();
    entries.sort_by_key(|a| (!a.0, a.1.to_lowercase()));
    let mut lines = vec![format!("{} ({} items):", short(&folder), entries.len())];
    for (is_dir, name, meta) in entries.iter().take(60) {
        match (is_dir, meta) {
            (true, _) => lines.push(format!("{name}/")),
            (false, Some(m)) => lines.push(format!("{name}  ({}, {})", size(m.len()), date(mtime(m)))),
            _ => {}
        }
    }
    if entries.len() > 60 {
        lines.push(format!("… and {} more", entries.len() - 60));
    }
    Ok(lines.join("\n"))
}

pub fn find_files(_: &Context, args: &Args) -> ToolResult {
    let words: Vec<String> = arg_str(args, "name").split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Err("give part of the file name to look for".into());
    }
    let raw = arg_str(args, "folder");
    let root = home_path(if raw.is_empty() { "~" } else { &raw }, true)?;
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut hits: Vec<(u64, PathBuf, bool)> = vec![];
    let mut stack = vec![root.clone()];
    'walk: while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && !matches!(name.as_str(), "node_modules" | "__pycache__") {
                stack.push(path.clone());
            }
            let lower = name.to_lowercase();
            if words.iter().all(|w| lower.contains(w))
                && let Ok(m) = entry.metadata() {
                    hits.push((mtime(&m), path, is_dir));
                }
            if Instant::now() > deadline || hits.len() > 500 {
                break 'walk;
            }
        }
    }
    if hits.is_empty() {
        return Ok(format!("No files named like '{}' in {}.", words.join(" "), short(&root)));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    Ok(hits.iter().take(20).map(|(_, p, d)| format!("{}{}", short(p), if *d { "/" } else { "" }))
        .collect::<Vec<_>>().join("\n"))
}

pub fn read_file(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    if !path.is_file() {
        return Err(format!("{} isn't a file", short(&path)).into());
    }
    let mut data = fs::read(&path).map_err(|e| e.to_string())?;
    data.truncate(200_000);
    if data.iter().take(4096).any(|&b| b == 0) {
        return Err(format!("{} isn't a text file; use open_file to open it in its app", short(&path)).into());
    }
    let text = String::from_utf8_lossy(&data);
    let body: String = if text.chars().count() > 3000 {
        format!("{}\n[…]", text.chars().take(3000).collect::<String>())
    } else {
        text.to_string()
    };
    Ok(format!("{}:\n{body}", short(&path)))
}

pub fn open_file(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&path).uri(), None::<&gio::AppLaunchContext>)
        .map_err(|e| ToolError(e.to_string()))?;
    Ok(format!("Opened {}.", short(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use serde_json::json;

    fn args(v: serde_json::Value) -> Args {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn paths_stay_in_home_and_skip_hidden() {
        let _h = temp_home();
        let home = real_home();
        fs::create_dir_all(home.join("Projects/app")).unwrap();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/id_rsa"), "secret").unwrap();
        fs::write(home.join("Projects/app/notes.txt"), "hello").unwrap();
        assert_eq!(home_path("Projects/app", true).unwrap(), home.join("Projects/app"));
        assert_eq!(home_path("~/Projects", true).unwrap(), home.join("Projects"));
        assert_eq!(home_path("~", true).unwrap(), home);
        assert!(home_path("/etc/passwd", true).unwrap_err().0.contains("home folder"));
        assert!(home_path("Projects/../../etc", true).is_err());
        assert!(home_path(".ssh/id_rsa", true).unwrap_err().0.contains("hidden"));
        assert!(home_path("nope.txt", true).unwrap_err().0.contains("doesn't exist"));
        assert!(home_path("Projects/app/note.txt", true).unwrap_err().0.contains("Similar in ~/Projects/app: notes.txt"));
        let shots = home.join("Obrazy/Zrzuty ekranu");
        fs::create_dir_all(&shots).unwrap();
        fs::write(shots.join("Zrzut ekranu z 2026-11-15 01-02-03.png"), "png").unwrap();
        assert_eq!(home_path("~/Obrazy/Zrzuty ekranu/zrzut_ekranu_z_2026-11-15_01:02:03.PNG", true).unwrap(),
                   shots.join("Zrzut ekranu z 2026-11-15 01-02-03.png"));
        assert!(home_path("~/Obrazy/Zrzuty ekranu/zrzut ekranu z 2026-11-16.png", true).is_err());
        assert_eq!(short(&home.join("Projects")), "~/Projects");
        let out = read_file(&Context::default(), &args(json!({"path": "Projects/app/notes.txt"}))).unwrap();
        assert_eq!(out, "~/Projects/app/notes.txt:\nhello");
        let found = find_files(&Context::default(), &args(json!({"name": "NOTES"}))).unwrap();
        assert_eq!(found, "~/Projects/app/notes.txt");
        let listing = list_folder(&Context::default(), &args(json!({"path": "Projects/app"}))).unwrap();
        assert!(listing.starts_with("~/Projects/app (1 items):\nnotes.txt  (5 B, "));
        assert_eq!(size(1536), "1.5 KB");
    }
}
