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
    ("desktop", glib::UserDirectory::Desktop),
    ("documents", glib::UserDirectory::Documents),
    ("download", glib::UserDirectory::Downloads),
    ("music", glib::UserDirectory::Music),
    ("pictures", glib::UserDirectory::Pictures),
    ("public_share", glib::UserDirectory::PublicShare),
    ("templates", glib::UserDirectory::Templates),
    ("videos", glib::UserDirectory::Videos),
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

pub fn real_home() -> PathBuf {
    fs::canonicalize(home()).unwrap_or_else(|_| home())
}

/// {role: folder} for the XDG user folders that exist.
pub fn user_dirs() -> Vec<(String, PathBuf)> {
    let home = real_home();
    USER_DIRS
        .iter()
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
        Ok(rel) => rel
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.')),
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
    let word = word
        .trim_start_matches("my ")
        .trim_start_matches("~/")
        .trim();
    let dirs = user_dirs();
    let role_dir = dir_alias(word).and_then(|r| {
        dirs.iter()
            .find(|(role, _)| role == r)
            .map(|(_, p)| p.clone())
    });
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
    let Some(dir) = path.ancestors().skip(1).find(|d| d.is_dir()) else {
        return msg;
    };
    let wanted = path
        .strip_prefix(dir)
        .ok()
        .and_then(|r| r.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .unwrap_or_default();
    let names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
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
    let key = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let want = key(&path.file_name()?.to_string_lossy());
    if want.is_empty() {
        return None;
    }
    let mut hits = fs::read_dir(path.parent()?).ok()?.flatten().filter(|e| {
        !e.file_name().to_string_lossy().starts_with('.')
            && key(&e.file_name().to_string_lossy()) == want
    });
    let hit = hits.next()?;
    hits.next().is_none().then(|| hit.path())
}

pub fn size(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{v:.0} {unit}")
            } else {
                format!("{v:.1} {unit}")
            };
        }
        v /= 1024.0;
    }
    n.to_string()
}

fn date(secs: u64) -> String {
    glib::DateTime::from_unix_local(secs as i64)
        .and_then(|d| d.format("%Y-%m-%d"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn list_folder(_: &Context, args: &Args) -> ToolResult {
    let raw = arg_str(args, "path");
    let folder = home_path(if raw.is_empty() { "~" } else { &raw }, true)?;
    if !folder.is_dir() {
        return Err(format!("{} isn't a folder", short(&folder)).into());
    }
    let mut entries: Vec<(bool, String, Option<fs::Metadata>)> = fs::read_dir(&folder)
        .map_err(|e| e.to_string())?
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
            (false, Some(m)) => {
                lines.push(format!("{name}  ({}, {})", size(m.len()), date(mtime(m))))
            }
            _ => {}
        }
    }
    if entries.len() > 60 {
        lines.push(format!("… and {} more", entries.len() - 60));
    }
    Ok(lines.join("\n"))
}

/// Words that say which file by when, not by name ("the screenshot from
/// today", "my latest invoice"). Results come newest first with their dates,
/// so these are left out of the match.
const NOT_IN_NAMES: &str = "from today today's todays yesterday yesterday's latest newest last \
    recent most my the a an of i took z ze dzisiaj dziś dzisiejszy dzisiejsze dzisiejszego wczoraj \
    wczorajszy wczorajszego ostatni ostatnie ostatniego najnowszy najnowsze najnowszego mój moje mojego";

fn not_in_names(word: &str) -> bool {
    NOT_IN_NAMES.split_whitespace().any(|w| w == word)
}

fn is_shot_word(word: &str) -> bool {
    word.starts_with("screenshot") || word.starts_with("zrzut")
}

/// GNOME Shell's Screenshots folder in Pictures, under its name in the user's
/// language ("Zrzuty ekranu"). Its files are named in that language too
/// ("Zrzut ekranu z …"), so "screenshot" matches them by their folder.
fn screenshots_dir() -> Option<PathBuf> {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)?;
    shell_translation("Screenshots")
        .into_iter()
        .chain(["Screenshots".to_string()])
        .find_map(|name| {
            fs::canonicalize(pictures.join(name))
                .ok()
                .filter(|p| p.is_dir())
        })
}

/// `msgid` in gnome-shell's translations for the user's languages, read from
/// the .mo file itself: this service never calls setlocale, so gettext would
/// answer in English.
fn shell_translation(msgid: &str) -> Option<String> {
    glib::language_names().iter().find_map(|lang| {
        let mo = fs::read(format!(
            "/usr/share/locale/{lang}/LC_MESSAGES/gnome-shell.mo"
        ))
        .ok()?;
        mo_lookup(&mo, msgid)
    })
}

/// Looks `msgid` up in a GNU gettext .mo catalog.
fn mo_lookup(mo: &[u8], msgid: &str) -> Option<String> {
    let little = match mo.get(..4)? {
        [0xde, 0x12, 0x04, 0x95] => true,
        [0x95, 0x04, 0x12, 0xde] => false,
        _ => return None,
    };
    let num = |at: usize| -> Option<usize> {
        let b: [u8; 4] = mo.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    let text = |table: usize, i: usize| -> Option<&[u8]> {
        let (len, at) = (num(table + 8 * i)?, num(table + 8 * i + 4)?);
        mo.get(at..at + len)
    };
    let (count, originals, translations) = (num(8)?, num(12)?, num(16)?);
    let i = (0..count).find(|&i| text(originals, i) == Some(msgid.as_bytes()))?;
    let found = std::str::from_utf8(text(translations, i)?).ok()?;
    (!found.is_empty()).then(|| found.to_string())
}

pub fn find_files(_: &Context, args: &Args) -> ToolResult {
    let mut words: Vec<String> = arg_str(args, "name")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        return Err("give part of the file name to look for".into());
    }
    if words.iter().any(|w| !not_in_names(w)) {
        words.retain(|w| !not_in_names(w));
    }
    let shots = words
        .iter()
        .any(|w| is_shot_word(w))
        .then(screenshots_dir)
        .flatten();
    let raw = arg_str(args, "folder");
    let root = match (&shots, raw.is_empty()) {
        // Screenshots live in Pictures: don't spend the time limit on the rest of home.
        (Some(dir), true) => dir.parent().map_or_else(real_home, Path::to_path_buf),
        _ => home_path(if raw.is_empty() { "~" } else { &raw }, true)?,
    };
    let hits = search(&root, &words, shots.as_deref());
    if hits.is_empty() {
        return Ok(format!(
            "No files named like '{}' in {}.",
            words.join(" "),
            short(&root)
        ));
    }
    Ok(hits
        .iter()
        .take(20)
        .map(|(t, p, d, n)| match d {
            true => format!("{}/ ({})", short(p), date(*t)),
            false => format!("{} ({}, {})", short(p), size(*n), date(*t)),
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Files and folders under `root` whose names hold every word, newest first,
/// as (modified, path, is folder, size). A screenshot word also matches every
/// file in `shots`.
fn search(root: &Path, words: &[String], shots: Option<&Path>) -> Vec<(u64, PathBuf, bool, u64)> {
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut hits = vec![];
    let mut stack = vec![root.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        let in_shots = shots == Some(dir.as_path());
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
            if words
                .iter()
                .all(|w| lower.contains(w.as_str()) || (in_shots && !is_dir && is_shot_word(w)))
                // The folder itself: newest whenever a screenshot lands in it, but not one to send.
                && shots != Some(path.as_path())
                && let Ok(m) = entry.metadata()
            {
                hits.push((mtime(&m), path, is_dir, m.len()));
            }
            // Not a cap on hits alone: the walk order isn't by date, so stopping
            // early could leave out the newest (a Screenshots folder has hundreds).
            if Instant::now() > deadline || hits.len() > 20_000 {
                break 'walk;
            }
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits
}

pub fn read_file(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    if !path.is_file() {
        return Err(format!("{} isn't a file", short(&path)).into());
    }
    let mut data = fs::read(&path).map_err(|e| e.to_string())?;
    data.truncate(2_000_000);
    // Documents (PDF, Word, spreadsheets, EPUB…) and other non-text files are read as their text.
    const DOCUMENTS: [&str; 15] = [
        "pdf", "docx", "docm", "pptx", "xlsx", "xlsm", "odt", "ods", "odp", "epub", "rtf", "ipynb",
        "eml", "zip", "html",
    ];
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    let binary = data.iter().take(4096).any(|&b| b == 0);
    let text = if binary || DOCUMENTS.contains(&ext.as_str()) {
        std::borrow::Cow::Owned(super::localsearch::document_text(&path, 300)?)
    } else {
        String::from_utf8_lossy(&data)
    };
    let lines: Vec<&str> = text.lines().collect();
    let from = args
        .get("from_line")
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
        .unwrap_or(1)
        .max(1) as usize;
    if from > lines.len().max(1) {
        return Err(format!("{} has only {} lines", short(&path), lines.len()).into());
    }
    // Whole lines, about 3000 characters' worth: a small model's context is short.
    let mut body = String::new();
    let mut to = from - 1;
    while to < lines.len() && (body.len() < 3000 || to == from - 1) {
        body.push_str(lines[to]);
        body.push('\n');
        to += 1;
    }
    if from == 1 && to >= lines.len() {
        return Ok(format!("{}:\n{}", short(&path), body.trim_end()));
    }
    Ok(format!(
        "{} (lines {from}-{to} of {}):\n{}{}",
        short(&path),
        lines.len(),
        body.trim_end(),
        if to < lines.len() {
            format!("\n[… read on with from_line {}]", to + 1)
        } else {
            String::new()
        }
    ))
}

pub fn open_file(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    gio::AppInfo::launch_default_for_uri(
        &gio::File::for_path(&path).uri(),
        None::<&gio::AppLaunchContext>,
    )
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
        assert_eq!(
            home_path("Projects/app", true).unwrap(),
            home.join("Projects/app")
        );
        assert_eq!(
            home_path("~/Projects", true).unwrap(),
            home.join("Projects")
        );
        assert_eq!(home_path("~", true).unwrap(), home);
        assert!(
            home_path("/etc/passwd", true)
                .unwrap_err()
                .0
                .contains("home folder")
        );
        assert!(home_path("Projects/../../etc", true).is_err());
        assert!(
            home_path(".ssh/id_rsa", true)
                .unwrap_err()
                .0
                .contains("hidden")
        );
        assert!(
            home_path("nope.txt", true)
                .unwrap_err()
                .0
                .contains("doesn't exist")
        );
        assert!(
            home_path("Projects/app/note.txt", true)
                .unwrap_err()
                .0
                .contains("Similar in ~/Projects/app: notes.txt")
        );
        let shots = home.join("Obrazy/Zrzuty ekranu");
        fs::create_dir_all(&shots).unwrap();
        fs::write(shots.join("Zrzut ekranu z 2026-11-15 01-02-03.png"), "png").unwrap();
        assert_eq!(
            home_path(
                "~/Obrazy/Zrzuty ekranu/zrzut_ekranu_z_2026-11-15_01:02:03.PNG",
                true
            )
            .unwrap(),
            shots.join("Zrzut ekranu z 2026-11-15 01-02-03.png")
        );
        assert!(home_path("~/Obrazy/Zrzuty ekranu/zrzut ekranu z 2026-11-16.png", true).is_err());
        assert_eq!(short(&home.join("Projects")), "~/Projects");
        let out = read_file(
            &Context::default(),
            &args(json!({"path": "Projects/app/notes.txt"})),
        )
        .unwrap();
        assert_eq!(out, "~/Projects/app/notes.txt:\nhello");
        let found = find_files(&Context::default(), &args(json!({"name": "NOTES"}))).unwrap();
        assert!(
            found.starts_with("~/Projects/app/notes.txt (5 B, 20"),
            "{found}"
        );
        // "from today" says when, not what: the match leaves it out.
        let found = find_files(
            &Context::default(),
            &args(json!({"name": "the notes from today"})),
        )
        .unwrap();
        assert!(found.starts_with("~/Projects/app/notes.txt ("), "{found}");
        let listing =
            list_folder(&Context::default(), &args(json!({"path": "Projects/app"}))).unwrap();
        assert!(listing.starts_with("~/Projects/app (1 items):\nnotes.txt  (5 B, "));
        assert_eq!(size(1536), "1.5 KB");
    }

    #[test]
    fn screenshots_match_by_their_folder_in_any_language() {
        let _h = temp_home();
        let pictures = real_home().join("Obrazy");
        let shots = pictures.join("Zrzuty ekranu");
        fs::create_dir_all(&shots).unwrap();
        let day = |d: u64| UNIX_EPOCH + Duration::from_secs(1_795_000_000 + d * 86_400);
        for (name, d) in [
            ("Zrzuty ekranu/Zrzut ekranu z 2026-11-24 22-58-34.png", 1),
            ("Zrzuty ekranu/Zrzut ekranu z 2026-11-25 11-57-14.png", 3),
            ("Screenshot-9.png", 2),
            ("wakacje.jpg", 4),
        ] {
            let f = fs::File::create(pictures.join(name)).unwrap();
            f.set_modified(day(d)).unwrap();
        }
        let names = |words: &[&str]| -> Vec<String> {
            let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            search(&pictures, &words, Some(&shots))
                .into_iter()
                .map(|h| h.1.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            names(&["screenshot"]),
            [
                "Zrzut ekranu z 2026-11-25 11-57-14.png",
                "Screenshot-9.png",
                "Zrzut ekranu z 2026-11-24 22-58-34.png"
            ]
        );
        assert_eq!(names(&["zrzuty", "ekranu"]).len(), 2);
        assert_eq!(names(&["screenshot", "2026-11-24"]).len(), 1);
        // Without the folder, only names count.
        let words = vec!["screenshot".to_string()];
        assert_eq!(search(&pictures, &words, None).len(), 1);
    }

    #[test]
    fn mo_catalogs_are_read() {
        // A catalog with "" (the header) and "Screenshots", as msgfmt sorts them.
        let (ids, strs) = (
            ["", "Screenshots"],
            ["Content-Type: text/plain\n", "Zrzuty ekranu"],
        );
        let mut mo: Vec<u8> = vec![];
        let mut put = |v: u32| mo.extend(v.to_le_bytes());
        let strings_at = 28 + 16 * 2;
        for v in [0x950412de, 0, 2, 28, 28 + 16, 0, 0] {
            put(v);
        }
        let mut at = strings_at;
        let mut tables = vec![];
        for s in ids.iter().chain(&strs) {
            tables.push((s.len() as u32, at as u32));
            at += s.len() + 1;
        }
        for (len, off) in &tables {
            put(*len);
            put(*off);
        }
        for s in ids.iter().chain(&strs) {
            mo.extend(s.as_bytes());
            mo.push(0);
        }
        assert_eq!(
            mo_lookup(&mo, "Screenshots").as_deref(),
            Some("Zrzuty ekranu")
        );
        assert_eq!(mo_lookup(&mo, "Screenshot"), None);
        assert_eq!(mo_lookup(b"not a catalog", "Screenshots"), None);
        // The real one, where installed.
        if let Ok(real) = fs::read("/usr/share/locale/pl/LC_MESSAGES/gnome-shell.mo") {
            assert_eq!(
                mo_lookup(&real, "Screenshots").as_deref(),
                Some("Zrzuty ekranu")
            );
        }
    }
}
