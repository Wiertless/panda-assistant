//! Changing files in the home folder, the way a coding agent does: write a
//! file, replace exact text in it, move or rename, make a folder, move to the
//! Trash, and search inside text files. The same rules as reading apply
//! (inside home, never hidden files), every change can be undone, and the
//! tools that change something ask first (Risk::Confirm, set in mod.rs).

use std::fs;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::files::{hidden, home_path, real_home, short};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

/// Bigger files are not edited as text (they're not notes or code).
const MAX_EDIT_BYTES: u64 = 2_000_000;
const MAX_HITS: usize = 40;
const MAX_SCANNED: usize = 20_000;

fn flag(args: &Args, key: &str) -> bool {
    match args.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => {
            matches!(s.to_lowercase().as_str(), "true" | "yes" | "1")
        }
        _ => false,
    }
}

fn read_text(path: &Path) -> Result<String, ToolError> {
    let meta = fs::metadata(path).map_err(|e| ToolError(format!("{}: {e}", short(path))))?;
    if !meta.is_file() {
        return Err(format!("{} isn't a file", short(path)).into());
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(ToolError::cannot(format!(
            "{} is too big to edit as text",
            short(path)
        )));
    }
    let data = fs::read(path).map_err(|e| ToolError(e.to_string()))?;
    if data.iter().take(4096).any(|&b| b == 0) {
        return Err(ToolError::cannot(format!(
            "{} isn't a text file, so it can't be edited here; open it in its app",
            short(path)
        )));
    }
    String::from_utf8(data)
        .map_err(|_| ToolError::cannot(format!("{} isn't UTF-8 text", short(path))))
}

/// Write `text` to `path`, keeping what was there for undo.
fn write_with_undo(ctx: &Context, path: &Path, text: &str, what: &str) -> Result<(), ToolError> {
    let before = fs::read(path).ok();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| ToolError(e.to_string()))?;
    }
    fs::write(path, text).map_err(|e| ToolError(format!("couldn't write {}: {e}", short(path))))?;
    let p = path.to_path_buf();
    ctx.can_undo(what, move || match &before {
        Some(old) => {
            fs::write(&p, old).map_err(|e| ToolError(e.to_string()))?;
            Ok(format!("{} is back as it was.", short(&p)))
        }
        None => {
            fs::remove_file(&p).map_err(|e| ToolError(e.to_string()))?;
            Ok(format!("Removed {} again.", short(&p)))
        }
    });
    Ok(())
}

pub fn write_file(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), false)?;
    if path.is_dir() {
        return Err(format!("{} is a folder; give a file name inside it", short(&path)).into());
    }
    let content = arg_str(args, "content");
    let exists = path.exists();
    if exists && !flag(args, "overwrite") {
        return Err(format!(
            "{} already exists; to replace it, call again with overwrite true, or use edit_file to change part of it",
            short(&path)
        )
        .into());
    }
    if exists {
        read_text(&path)?; // never replace a binary file wholesale
    }
    write_with_undo(ctx, &path, &content, &format!("Write {}", short(&path)))?;
    Ok(format!(
        "{} {} ({} lines).",
        if exists { "Replaced" } else { "Created" },
        short(&path),
        content.lines().count()
    ))
}

/// Lines of `text` that share the most words with `wanted`, to show when the
/// exact text isn't there.
fn near_lines(text: &str, wanted: &str) -> Vec<String> {
    let words: Vec<String> = wanted
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() > 2)
        .collect();
    let mut scored: Vec<(usize, usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let low = l.to_lowercase();
            (
                words.iter().filter(|w| low.contains(w.as_str())).count(),
                i,
                l,
            )
        })
        .filter(|(n, _, _)| *n > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(3)
        .map(|(_, i, l)| format!("line {}: {}", i + 1, l.trim()))
        .collect()
}

pub fn edit_file(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    let old = arg_str(args, "old_text");
    let new = arg_str(args, "new_text");
    if old.is_empty() {
        return Err(
            "old_text is empty: give the exact text to replace (read_file shows it)".into(),
        );
    }
    if old == new {
        return Err("old_text and new_text are the same".into());
    }
    let text = read_text(&path)?;
    let count = text.matches(&old).count();
    if count == 0 {
        let near = near_lines(&text, &old);
        return Err(format!(
            "that exact text isn't in {}{}",
            short(&path),
            if near.is_empty() {
                "; read_file shows what's there".to_string()
            } else {
                format!(". Closest lines:\n{}", near.join("\n"))
            }
        )
        .into());
    }
    let all = flag(args, "all");
    if count > 1 && !all {
        return Err(format!(
            "that text is in {} {count} times; add more of the surrounding text so it's found once, or pass all true",
            short(&path)
        )
        .into());
    }
    let changed = if all {
        text.replace(&old, &new)
    } else {
        text.replacen(&old, &new, 1)
    };
    write_with_undo(ctx, &path, &changed, &format!("Edit {}", short(&path)))?;
    Ok(format!(
        "Changed {} ({} {}).",
        short(&path),
        if all { count } else { 1 },
        if all && count > 1 { "places" } else { "place" }
    ))
}

pub fn move_file(ctx: &Context, args: &Args) -> ToolResult {
    let from = home_path(&arg_str(args, "from"), true)?;
    let mut to = home_path(&arg_str(args, "to"), false)?;
    if from == real_home() {
        return Err(ToolError::cannot("the home folder itself can't be moved"));
    }
    // Into a folder that exists: keep the name, as `mv` does.
    if to.is_dir() {
        to = to.join(from.file_name().unwrap_or_default());
    }
    if to == from {
        return Err("that's where it already is".into());
    }
    if to.exists() {
        return Err(format!("{} already exists; pick another name", short(&to)).into());
    }
    if to.starts_with(&from) {
        return Err("a folder can't go inside itself".into());
    }
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|e| ToolError(e.to_string()))?;
    }
    fs::rename(&from, &to)
        .map_err(|e| ToolError(format!("couldn't move {}: {e}", short(&from))))?;
    let (a, b) = (from.clone(), to.clone());
    ctx.can_undo(&format!("Move {}", short(&from)), move || {
        fs::rename(&b, &a).map_err(|e| ToolError(e.to_string()))?;
        Ok(format!("{} is back at {}.", short(&b), short(&a)))
    });
    let renamed = from.parent() == to.parent();
    Ok(format!(
        "{} {} to {}.",
        if renamed { "Renamed" } else { "Moved" },
        short(&from),
        short(&to)
    ))
}

pub fn make_folder(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), false)?;
    if path.is_dir() {
        return Ok(format!("{} already exists.", short(&path)));
    }
    if path.exists() {
        return Err(format!("{} is a file", short(&path)).into());
    }
    // The first missing folder, so undo removes everything this made.
    let top = path
        .ancestors()
        .take_while(|p| !p.exists())
        .last()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.clone());
    fs::create_dir_all(&path).map_err(|e| ToolError(e.to_string()))?;
    ctx.can_undo(&format!("Make {}", short(&path)), move || {
        fs::remove_dir(&top)
            .or_else(|_| remove_empty_tree(&top))
            .map_err(|_| ToolError::cannot(format!("{} isn't empty any more", short(&top))))?;
        Ok(format!("Removed {} again.", short(&top)))
    });
    Ok(format!("Made the folder {}.", short(&path)))
}

/// Remove a tree of empty folders (and nothing else).
fn remove_empty_tree(dir: &Path) -> std::io::Result<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            remove_empty_tree(&p)?;
        } else {
            return Err(std::io::Error::other("not empty"));
        }
    }
    fs::remove_dir(dir)
}

/// Find a trashed item by where it came from, newest first.
fn in_trash(orig: &Path) -> Option<gio::File> {
    let trash = gio::File::for_uri("trash:///");
    let children = trash
        .enumerate_children(
            "standard::name,trash::orig-path,trash::deletion-date",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let mut best: Option<(String, gio::File)> = None;
    for info in children.flatten() {
        let from = info.attribute_byte_string("trash::orig-path");
        if from.as_deref().map(Path::new) != Some(orig) {
            continue;
        }
        let when = info
            .attribute_string("trash::deletion-date")
            .map(|s| s.to_string())
            .unwrap_or_default();
        if best.as_ref().is_none_or(|(w, _)| when > *w) {
            best = Some((when, trash.child(info.name())));
        }
    }
    best.map(|(_, f)| f)
}

pub fn trash_file(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    if path == real_home() {
        return Err(ToolError::cannot(
            "the home folder itself can't go to the Trash",
        ));
    }
    gio::File::for_path(&path)
        .trash(gio::Cancellable::NONE)
        .map_err(|e| ToolError(format!("couldn't move {} to the Trash: {e}", short(&path))))?;
    let p = path.clone();
    ctx.can_undo(&format!("Trash {}", short(&path)), move || {
        let item = in_trash(&p).ok_or_else(|| {
            ToolError::cannot(format!("{} isn't in the Trash any more", short(&p)))
        })?;
        item.move_(
            &gio::File::for_path(&p),
            gio::FileCopyFlags::NONE,
            gio::Cancellable::NONE,
            None,
        )
        .map_err(|e| ToolError(e.to_string()))?;
        Ok(format!("{} is back from the Trash.", short(&p)))
    });
    Ok(format!(
        "Moved {} to the Trash (Undo, or Files → Trash, brings it back).",
        short(&path)
    ))
}

/// Text files under `dir`, skipping hidden ones, at most MAX_SCANNED.
fn text_files(dir: &Path, home: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= MAX_SCANNED {
            return;
        }
        let p = e.path();
        if hidden(&p, home) {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => {
                // Build output and dependencies: huge, and never what's meant.
                let name = e.file_name();
                if !matches!(
                    name.to_str(),
                    Some("node_modules" | "target" | "__pycache__")
                ) {
                    text_files(&p, home, out);
                }
            }
            Ok(t) if t.is_file() && e.metadata().is_ok_and(|m| m.len() <= MAX_EDIT_BYTES) => {
                out.push(p);
            }
            _ => {}
        }
    }
}

/// Search text files for words, like grep: "path:line: text".
pub fn search_text(_: &Context, args: &Args) -> ToolResult {
    let query = arg_str(args, "query");
    let wanted = query.trim().to_lowercase();
    if wanted.is_empty() {
        return Err("say what to look for".into());
    }
    let folder = arg_str(args, "folder");
    let dir = home_path(
        if folder.trim().is_empty() {
            "~"
        } else {
            &folder
        },
        true,
    )?;
    if !dir.is_dir() {
        return Err(format!("{} isn't a folder", short(&dir)).into());
    }
    let home = real_home();
    let mut files = vec![];
    text_files(&dir, &home, &mut files);
    files.sort();
    let mut hits = vec![];
    let mut matched_files = 0;
    'files: for f in &files {
        let Ok(data) = fs::read(f) else { continue };
        if data.iter().take(4096).any(|&b| b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&data);
        let mut found = false;
        for (i, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(&wanted) {
                found = true;
                let line: String = line.trim().chars().take(160).collect();
                hits.push(format!("{}:{}: {line}", short(f), i + 1));
                if hits.len() >= MAX_HITS {
                    break 'files;
                }
            }
        }
        matched_files += usize::from(found);
    }
    if hits.is_empty() {
        return Ok(format!(
            "No text files in {} contain “{query}” ({} looked at). For PDFs and office documents use search_file_contents.",
            short(&dir),
            files.len()
        ));
    }
    Ok(format!(
        "“{query}” in {} ({matched_files} files{}):\n{}",
        short(&dir),
        if hits.len() >= MAX_HITS {
            ", first 40 lines"
        } else {
            ""
        },
        hits.join("\n")
    ))
}

/// What a change will do, for the Allow card.
pub fn edit_detail(args: &Args) -> String {
    let cut = |s: String| -> String {
        if s.chars().count() > 300 {
            format!("{}…", s.chars().take(300).collect::<String>())
        } else {
            s
        }
    };
    format!(
        "In {}:\n− {}\n+ {}",
        arg_str(args, "path"),
        cut(arg_str(args, "old_text")),
        cut(arg_str(args, "new_text"))
    )
}

pub fn write_detail(args: &Args) -> String {
    let content = arg_str(args, "content");
    let preview: String = content.chars().take(400).collect();
    format!(
        "{} ({} lines){}:\n{preview}{}",
        arg_str(args, "path"),
        content.lines().count(),
        if flag(args, "overwrite") {
            ", replacing what's there"
        } else {
            ""
        },
        if content.chars().count() > 400 {
            "…"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use serde_json::json;

    fn args(v: serde_json::Value) -> Args {
        serde_json::from_value(v).unwrap()
    }

    fn undo(ctx: &Context) -> String {
        let step = ctx.undo.lock().unwrap().pop().unwrap();
        (step.run)().unwrap()
    }

    #[test]
    fn write_edit_and_undo() {
        let _h = temp_home();
        let ctx = Context::default();
        let home = real_home();
        let out = write_file(
            &ctx,
            &args(json!({"path": "notes/shop.txt", "content": "milk\neggs\n"})),
        )
        .unwrap();
        assert_eq!(out, "Created ~/notes/shop.txt (2 lines).");
        assert!(
            write_file(
                &ctx,
                &args(json!({"path": "notes/shop.txt", "content": "x"}))
            )
            .is_err()
        );

        let e = edit_file(
            &ctx,
            &args(json!({"path": "notes/shop.txt", "old_text": "egs", "new_text": "bread"})),
        )
        .unwrap_err();
        assert!(e.0.contains("isn't in"), "{}", e.0);
        edit_file(
            &ctx,
            &args(json!({"path": "notes/shop.txt", "old_text": "eggs", "new_text": "bread"})),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(home.join("notes/shop.txt")).unwrap(),
            "milk\nbread\n"
        );
        undo(&ctx);
        assert_eq!(
            fs::read_to_string(home.join("notes/shop.txt")).unwrap(),
            "milk\neggs\n"
        );
        undo(&ctx);
        assert!(!home.join("notes/shop.txt").exists());
    }

    #[test]
    fn edits_must_be_unique_unless_all() {
        let _h = temp_home();
        let ctx = Context::default();
        fs::write(real_home().join("a.txt"), "x y x").unwrap();
        let e = edit_file(
            &ctx,
            &args(json!({"path": "a.txt", "old_text": "x", "new_text": "z"})),
        )
        .unwrap_err();
        assert!(e.0.contains("2 times"));
        edit_file(
            &ctx,
            &args(json!({"path": "a.txt", "old_text": "x", "new_text": "z", "all": true})),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(real_home().join("a.txt")).unwrap(),
            "z y z"
        );
    }

    #[test]
    fn move_rename_folder_and_limits() {
        let _h = temp_home();
        let ctx = Context::default();
        let home = real_home();
        fs::write(home.join("inv.pdf"), b"%PDF").unwrap();
        make_folder(&ctx, &args(json!({"path": "Invoices/2026"}))).unwrap();
        let out = move_file(
            &ctx,
            &args(json!({"from": "inv.pdf", "to": "Invoices/2026"})),
        )
        .unwrap();
        assert_eq!(out, "Moved ~/inv.pdf to ~/Invoices/2026/inv.pdf.");
        let out = move_file(
            &ctx,
            &args(json!({"from": "Invoices/2026/inv.pdf", "to": "Invoices/2026/nov.pdf"})),
        )
        .unwrap();
        assert!(out.starts_with("Renamed"));
        undo(&ctx);
        undo(&ctx);
        assert!(home.join("inv.pdf").exists());
        undo(&ctx);
        assert!(
            !home.join("Invoices").exists(),
            "undo removes the folders it made"
        );
        assert!(write_file(&ctx, &args(json!({"path": ".ssh/config", "content": "x"}))).is_err());
        assert!(write_file(&ctx, &args(json!({"path": "/etc/hosts", "content": "x"}))).is_err());
        assert!(move_file(&ctx, &args(json!({"from": "~", "to": "x"}))).is_err());
    }

    #[test]
    fn search_text_finds_lines() {
        let _h = temp_home();
        let home = real_home();
        fs::create_dir_all(home.join("proj/target")).unwrap();
        fs::write(
            home.join("proj/a.rs"),
            "fn main() {\n    // Kowalski asked\n}\n",
        )
        .unwrap();
        fs::write(home.join("proj/target/b.rs"), "Kowalski").unwrap();
        fs::create_dir_all(home.join(".secret")).unwrap();
        fs::write(home.join(".secret/k"), "Kowalski").unwrap();
        let out = search_text(&Context::default(), &args(json!({"query": "kowalski"}))).unwrap();
        assert!(out.contains("~/proj/a.rs:2: // Kowalski asked"), "{out}");
        assert!(!out.contains("target") && !out.contains(".secret"), "{out}");
    }
}
