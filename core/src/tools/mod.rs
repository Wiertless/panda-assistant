//! The built-in tools the model can call. Arguments come from a model, so
//! every tool validates them, and nothing goes through a shell.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::tool::{Args, Risk, Tool, ToolError, arg_str, obj};

pub mod apps;
pub mod brains;
pub mod files;
pub mod system;

/// Asked every time, never "Always Allow": a command can do anything.
pub const ALWAYS_ASK: [&str; 1] = ["run_command"];

pub fn norm_name(text: &str) -> String {
    static POSSESSIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[’'`]s\b").unwrap());
    static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w]+").unwrap());
    let lower = text.to_lowercase();
    NON_WORD.replace_all(&POSSESSIVE.replace_all(&lower, ""), " ").trim().to_string()
}

/// The one name the query means: exact, then prefix, then substring, then all words.
pub fn pick(query: &str, names: &[String], what: &str) -> Result<String, ToolError> {
    static FILLER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(my|the|a|an|to|moje?|m[oó]j)\s+").unwrap());
    let q = norm_name(&FILLER.replace(query.trim(), ""));
    if q.is_empty() {
        return Err(format!("say which {what}").into());
    }
    let normed: Vec<(&String, String)> = names.iter().map(|n| (n, norm_name(n))).collect();
    let tests: [&dyn Fn(&str) -> bool; 4] = [
        &|n| n == q, &|n| n.starts_with(&q), &|n| n.contains(&q),
        &|n| q.split(' ').all(|w| n.contains(w)),
    ];
    for test in tests {
        let mut hits: Vec<&String> = normed.iter().filter(|(_, n)| test(n)).map(|(name, _)| *name).collect();
        hits.dedup();
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0].clone()),
            _ => return Err(format!("more than one {what} matches '{query}': {}",
                                    hits.iter().take(8).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")).into()),
        }
    }
    let known = if names.is_empty() { "none".to_string() } else { names.iter().take(12).cloned().collect::<Vec<_>>().join(", ") };
    Err(format!("no {what} called '{query}' (known: {known})").into())
}

fn text(key: &'static str, f: impl Fn(String) -> String + Send + Sync + 'static) -> impl Fn(&Args) -> String + Send + Sync {
    move |a| f(arg_str(a, key))
}

fn or(s: String, default: &str) -> String {
    if s.is_empty() { default.to_string() } else { s }
}

/// Every built-in tool (the agent filters by settings and model abilities).
pub fn all() -> Vec<Tool> {
    use Risk::*;
    let path = || obj(json!({"path": {"type": "string"}}), &["path"]);
    vec![
        Tool::new("open_app", "Open an app by its name ('Zed') or by what it is: browser, email, files, calendar, \
music, video, photos, text editor, pdf, terminal or settings opens the user's default app for it.",
                  obj(json!({"name": {"type": "string", "description": "App name or kind of app"}}), &["name"]),
                  Safe, apps::open_app, text("name", |n| format!("Open {}", or(n, "an app")))),
        Tool::new("open_url", "Open a web address in the default browser.", obj(json!({"url": {"type": "string"}}), &["url"]),
                  Safe, apps::open_url, text("url", |u| format!("Open {}", or(u, "a link")))),
        Tool::new("open_file", "Open a file or folder with its default app. Paths can be relative to the home \
folder, start with ~, or start with a folder name like Downloads or Documents.", path(), Safe, files::open_file,
                  text("path", |p| format!("Open {}", or(p, "a file")))),
        Tool::new("list_folder", "List what's in a folder (default: the home folder).",
                  obj(json!({"path": {"type": "string", "description": "e.g. Downloads, Documents/work, ~"}}), &[]),
                  Safe, files::list_folder, text("path", |p| format!("Look in {}", or(p, "your home folder")))),
        Tool::new("find_files", "Find files and folders by part of their name, newest first.",
                  obj(json!({"name": {"type": "string"}, "folder": {"type": "string", "description": "Where to look (optional)"}}), &["name"]),
                  Safe, files::find_files, text("name", |n| format!("Find files named “{n}”"))),
        Tool::new("read_file", "Read a text file (notes, code, configs, CSV…) to answer from it.", path(), Safe,
                  files::read_file, text("path", |p| format!("Read {}", or(p, "a file")))),
        Tool::new("get_battery", "Get the battery charge level and charging state.", obj(json!({}), &[]), Safe,
                  system::get_battery, |_| "Read battery".into()),
        Tool::new("get_system_info", "Get OS, kernel, uptime, memory and disk space.", obj(json!({}), &[]), Safe,
                  system::get_system_info, |_| "Read system info".into()),
        Tool::new("get_datetime", "Get the current local date and time.", obj(json!({}), &[]), Safe,
                  system::get_datetime, |_| "Read the time".into()),
        Tool::new("remember", "Save a lasting fact or preference about the user for future chats \
(e.g. 'Prefers metric units'). Only when the user asks or it's clearly useful later.",
                  obj(json!({"fact": {"type": "string"}}), &["fact"]), Safe, brains::remember, |_| "Remember this".into()),
        Tool::new("forget", "Forget a remembered fact.", obj(json!({"fact": {"type": "string"}}), &["fact"]), Safe,
                  brains::forget, |_| "Forget this".into()),
        Tool::new("use_skill", "Get the steps of one of the user's saved skills, then follow them.",
                  obj(json!({"name": {"type": "string"}}), &["name"]), Safe, brains::use_skill,
                  text("name", |n| format!("Use the skill {n}"))),
        Tool::new("save_skill", "Save a reusable workflow as a skill the user can run again by name. \
steps: short numbered steps.",
                  obj(json!({"name": {"type": "string"}, "steps": {"type": "string"},
                             "description": {"type": "string", "description": "One line, optional"}}), &["name", "steps"]),
                  Safe, brains::save_skill, text("name", |n| format!("Save the skill {n}"))),
        Tool::new("list_app_actions", "List the actions an app offers, e.g. New Private Window, Compose Message.",
                  obj(json!({"app": {"type": "string"}}), &["app"]), Safe, apps::list_app_actions,
                  text("app", |a| format!("Look at what {} can do", or(a, "the app")))),
        Tool::new("run_app_action", "Run one of an app's actions, e.g. app firefox, action New Private Window.",
                  obj(json!({"app": {"type": "string"}, "action": {"type": "string"}}), &["app", "action"]), Safe,
                  apps::run_app_action, |a| format!("{}: {}", or(arg_str(a, "app"), "App"), arg_str(a, "action"))),
    ]
}

/// File name of a path argument, for summaries.
pub fn base_name(raw: &str) -> String {
    Path::new(raw).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_by_everyday_name() {
        let names: Vec<String> = ["Wiertless's AirPods Pro", "MX Master 3", "JBL Flip 5"].map(String::from).to_vec();
        assert_eq!(pick("my airpods", &names, "device").unwrap(), "Wiertless's AirPods Pro");
        assert_eq!(pick("mx master", &names, "device").unwrap(), "MX Master 3");
        assert_eq!(pick("flip", &names, "device").unwrap(), "JBL Flip 5");
        assert!(pick("headphones", &names, "device").is_err());
        let nets: Vec<String> = ["Home 5G", "Home 2.4"].map(String::from).to_vec();
        assert!(pick("home", &nets, "network").unwrap_err().0.contains("more than one"));
    }

    #[test]
    fn schemas_are_valid_function_definitions() {
        for t in all() {
            let s = t.schema();
            assert_eq!(s["type"], "function");
            assert_eq!(s["function"]["parameters"]["type"], "object", "{}", t.name);
            assert!(!t.description.is_empty());
        }
    }
}
