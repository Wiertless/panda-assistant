//! What the assistant keeps between chats: memory, skills and chat history.
//!
//! Plain files under ~/.local/share/panda-assistant, in the same formats as
//! the Python app, so either can read what the other wrote:
//!
//! - `memory.md`: one remembered fact per "- " line
//! - `skills/<name>.md`: saved workflows: "# Title", a description line, steps
//! - `chats/<id>.json`: saved conversations

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Value, json};

use crate::paths::data_dir;

pub const MAX_FACTS: usize = 50;
pub const MAX_FACT_LEN: usize = 200;
pub const MAX_SKILL_LEN: usize = 4000;
pub const MAX_CHATS: usize = 200;
pub const IMAGE_MARKER: &str = "[image]";
const SKILL_PROMPT_HEAD: &str = "Follow my saved skill “";

/// Write atomically, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn norm(text: &str) -> String {
    squash(&text.to_lowercase())
        .trim_matches(|c| c == ' ' || c == '.' || c == '!')
        .to_string()
}

fn chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

// -- memory -----------------------------------------------------------------------

pub fn memory_file() -> PathBuf {
    data_dir().join("memory.md")
}

pub fn facts() -> Vec<String> {
    fs::read_to_string(memory_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            l.strip_prefix("- ")
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(String::from)
        })
        .collect()
}

fn save_facts(items: &[String]) -> std::io::Result<()> {
    let body: String = items.iter().map(|f| format!("- {f}\n")).collect();
    write_atomic(
        &memory_file(),
        &format!("# Things the assistant remembers\n\n{body}"),
    )
}

pub fn remember(fact: &str) -> Result<String, String> {
    let fact = squash(fact);
    if fact.is_empty() {
        return Err("there's nothing to remember".into());
    }
    if fact.chars().count() > MAX_FACT_LEN {
        return Err(format!("keep it under {MAX_FACT_LEN} characters"));
    }
    let mut items = facts();
    if items.iter().any(|f| norm(f) == norm(&fact)) {
        return Ok("Already remembered.".into());
    }
    items.push(fact.clone());
    let start = items.len().saturating_sub(MAX_FACTS); // oldest go first
    save_facts(&items[start..]).map_err(|e| e.to_string())?;
    Ok(format!("Remembered: {fact}"))
}

/// Remove the facts that match; returns what was removed.
pub fn forget(text: &str) -> Vec<String> {
    let key = norm(text);
    if key.is_empty() {
        return vec![];
    }
    let items = facts();
    let mut gone: Vec<String> = items.iter().filter(|f| norm(f) == key).cloned().collect();
    if gone.is_empty() {
        gone = items
            .iter()
            .filter(|f| norm(f).contains(&key))
            .cloned()
            .collect();
    }
    if !gone.is_empty() {
        let keep: Vec<String> = items.into_iter().filter(|f| !gone.contains(f)).collect();
        let _ = save_facts(&keep);
    }
    gone
}

// -- skills -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub steps: String,
    pub path: PathBuf,
}

pub fn skills_dir() -> PathBuf {
    data_dir().join("skills")
}

pub fn slugify(name: &str) -> String {
    static NON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    let slug = NON
        .replace_all(&name.to_lowercase(), "-")
        .trim_matches('-')
        .to_string();
    chars(&slug, 40)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn parse_skill(path: &Path) -> Option<Skill> {
    static STEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d+[.)]|[-*])\s").unwrap());
    let text = fs::read_to_string(path).ok()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let mut lines: Vec<&str> = text.trim().lines().collect();
    let mut title = capitalize(&stem.replace('-', " "));
    if lines.first().is_some_and(|l| l.starts_with("# ")) {
        let t = lines.remove(0)[2..].trim();
        if !t.is_empty() {
            title = t.to_string();
        }
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    let mut description = String::new();
    if lines.first().is_some_and(|l| !STEP.is_match(l)) {
        description = lines.remove(0).trim().to_string();
    }
    let mut steps = lines.join("\n").trim().to_string();
    if steps.is_empty() && !description.is_empty() {
        steps = std::mem::take(&mut description);
    }
    Some(Skill {
        slug: stem,
        title,
        description,
        steps,
        path: path.to_path_buf(),
    })
}

pub fn skills() -> Vec<Skill> {
    let mut paths: Vec<PathBuf> = fs::read_dir(skills_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "md"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
        .iter()
        .filter_map(|p| parse_skill(p))
        .filter(|s| !s.steps.is_empty())
        .collect()
}

pub fn find_skill(name: &str) -> Option<Skill> {
    let slug = slugify(name.trim_start_matches('/'));
    if slug.is_empty() {
        return None;
    }
    let all = skills();
    if let Some(s) = all
        .iter()
        .find(|s| s.slug == slug || slugify(&s.title) == slug)
    {
        return Some(s.clone());
    }
    let hits: Vec<&Skill> = all
        .iter()
        .filter(|s| s.slug.contains(&slug) || slugify(&s.title).contains(&slug))
        .collect();
    if hits.len() == 1 {
        Some(hits[0].clone())
    } else {
        None
    }
}

pub fn save_skill(title: &str, steps: &str, description: &str) -> Result<Skill, String> {
    let title = squash(title);
    let slug = slugify(&title);
    if slug.is_empty() {
        return Err("the skill needs a name".into());
    }
    let steps = steps.trim();
    if steps.is_empty() {
        return Err("the skill needs steps".into());
    }
    if steps.chars().count() > MAX_SKILL_LEN {
        return Err(format!("keep the steps under {MAX_SKILL_LEN} characters"));
    }
    let description = squash(description);
    let path = skills_dir().join(format!("{slug}.md"));
    let desc_block = if description.is_empty() {
        String::new()
    } else {
        format!("{description}\n\n")
    };
    write_atomic(&path, &format!("# {title}\n\n{desc_block}{steps}\n"))
        .map_err(|e| e.to_string())?;
    Ok(Skill {
        slug,
        title,
        description,
        steps: steps.to_string(),
        path,
    })
}

pub fn delete_skill(slug: &str) -> bool {
    fs::remove_file(skills_dir().join(format!("{}.md", slugify(slug)))).is_ok()
}

/// The message that runs a skill.
pub fn skill_prompt(skill: &Skill, details: &str) -> String {
    let mut text = format!("{SKILL_PROMPT_HEAD}{}”:\n{}", skill.title, skill.steps);
    if !details.trim().is_empty() {
        text.push_str(&format!("\n\nDetails: {}", details.trim()));
    }
    text
}

// -- chat history ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ChatInfo {
    pub id: String,
    pub title: String,
    pub updated: f64,
    pub path: PathBuf,
}

pub fn chats_dir() -> PathBuf {
    data_dir().join("chats")
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn new_chat_id() -> String {
    let stamp = glib::DateTime::now_local()
        .and_then(|d| d.format("%Y%m%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "chat".into());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{stamp}-{:06}", nanos % 1_000_000)
}

/// Message content as text; images are kept as a marker only.
pub fn plain_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(String::from),
                Some("image_url") => Some(IMAGE_MARKER.into()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn chat_title(messages: &[Value]) -> String {
    let first = messages
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"));
    let mut text = squash(&first.map(|m| plain_text(&m["content"])).unwrap_or_default());
    if let Some(rest) = text.strip_prefix(SKILL_PROMPT_HEAD) {
        text = format!("Skill: {}", rest.split('”').next().unwrap_or(""));
    }
    if text.chars().count() > 60 {
        format!("{}…", chars(&text, 57))
    } else if text.is_empty() {
        "New Chat".into()
    } else {
        text
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

pub fn save_chat(
    chat_id: &str,
    messages: &[Value],
    model: &str,
    summary: &str,
    compacted: usize,
) -> std::io::Result<()> {
    if !valid_id(chat_id)
        || !messages
            .iter()
            .any(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    {
        return Ok(());
    }
    let stored: Vec<Value> = messages
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if m.get("content").is_some_and(Value::is_array) {
                m["content"] = Value::String(plain_text(&m["content"])); // don't keep screenshots on disk
            }
            m
        })
        .collect();
    let mut data = json!({"id": chat_id, "title": chat_title(messages), "model": model, "updated": now(),
                          "messages": stored});
    if !summary.is_empty() {
        data["summary"] = json!(summary);
        data["compacted"] = json!(compacted);
    }
    write_atomic(
        &chats_dir().join(format!("{chat_id}.json")),
        &data.to_string(),
    )?;
    for old in list_chats("").into_iter().skip(MAX_CHATS) {
        let _ = fs::remove_file(old.path);
    }
    Ok(())
}

pub fn load_chat(chat_id: &str) -> Option<Value> {
    if !valid_id(chat_id) {
        return None;
    }
    let data: Value = serde_json::from_str(
        &fs::read_to_string(chats_dir().join(format!("{chat_id}.json"))).ok()?,
    )
    .ok()?;
    data.get("messages")
        .is_some_and(Value::is_array)
        .then_some(data)
}

pub fn list_chats(query: &str) -> Vec<ChatInfo> {
    let key = norm(query);
    let mut out = Vec::new();
    for entry in fs::read_dir(chats_dir()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(data) = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            continue;
        };
        let title = data
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Chat")
            .to_string();
        if !key.is_empty() {
            let text: Vec<String> = data["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| {
                    matches!(
                        m.get("role").and_then(Value::as_str),
                        Some("user" | "assistant")
                    )
                })
                .map(|m| plain_text(&m["content"]))
                .collect();
            if !norm(&format!("{title} {}", text.join(" "))).contains(&key) {
                continue;
            }
        }
        out.push(ChatInfo {
            id: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            title,
            updated: data.get("updated").and_then(Value::as_f64).unwrap_or(0.0),
            path,
        });
    }
    out.sort_by(|a, b| b.updated.total_cmp(&a.updated));
    out
}

pub fn delete_chat(chat_id: &str) {
    if valid_id(chat_id) {
        let _ = fs::remove_file(chats_dir().join(format!("{chat_id}.json")));
    }
}

#[cfg(test)]
pub(crate) mod testenv {
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    /// Points XDG/HOME at a temporary folder for one test (tests share the env).
    pub struct TempHome {
        pub _dir: tempfile::TempDir,
        _guard: MutexGuard<'static, ()>,
    }

    pub fn temp_home() -> TempHome {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: tests that touch the environment hold LOCK.
        unsafe {
            std::env::set_var("HOME", dir.path());
            std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
            std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
            std::env::set_var("XDG_STATE_HOME", dir.path().join("state"));
        }
        TempHome {
            _dir: dir,
            _guard: guard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testenv::temp_home;
    use super::*;

    #[test]
    fn memory() {
        let _h = temp_home();
        assert!(
            remember("Prefers   metric units")
                .unwrap()
                .starts_with("Remembered")
        );
        assert_eq!(
            remember("prefers metric units.").unwrap(),
            "Already remembered."
        );
        remember("Uses fish shell").unwrap();
        assert_eq!(facts(), ["Prefers metric units", "Uses fish shell"]);
        assert!(
            fs::read_to_string(memory_file())
                .unwrap()
                .starts_with("# Things the assistant remembers")
        );
        assert_eq!(forget("fish"), ["Uses fish shell"]);
        assert!(forget("nothing like this").is_empty());
        assert!(remember(&"x".repeat(500)).is_err());
        for i in 0..MAX_FACTS + 3 {
            remember(&format!("fact {i}")).unwrap();
        }
        assert_eq!(facts().len(), MAX_FACTS);
    }

    #[test]
    fn skills_round_trip() {
        let _h = temp_home();
        let s = save_skill(
            "Morning Routine",
            "1. Dark mode on\n2. Volume 30%",
            "Start the day",
        )
        .unwrap();
        assert_eq!(s.slug, "morning-routine");
        let found = find_skill("/morning").unwrap();
        assert_eq!(
            (found.title.as_str(), found.description.as_str()),
            ("Morning Routine", "Start the day")
        );
        assert!(skill_prompt(&found, "quietly").ends_with("Details: quietly"));
        fs::write(
            skills_dir().join("backup.md"),
            "- copy Documents\n- eject\n",
        )
        .unwrap();
        fs::write(skills_dir().join("empty.md"), "# Empty\n").unwrap();
        let titles: Vec<String> = skills().into_iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Backup", "Morning Routine"]);
        save_skill("Backup music", "1. b", "").unwrap();
        assert!(find_skill("backup").is_some()); // exact slug "backup"
        assert!(find_skill("back").is_none()); // two partial matches
        assert!(delete_skill("morning-routine"));
        assert!(save_skill("!!!", "x", "").is_err());
    }

    #[test]
    fn chats() {
        let _h = temp_home();
        let msgs = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "What is this?"},
                                               {"type": "image_url", "image_url": {"url": "data:x"}}]}),
            json!({"role": "assistant", "content": "A red panda."}),
        ];
        save_chat("a-1", &msgs, "m", "", 0).unwrap();
        save_chat(
            "b-2",
            &[json!({"role": "user", "content": "Weather in Kraków"})],
            "m",
            "sum",
            1,
        )
        .unwrap();
        save_chat(
            "c-3",
            &[json!({"role": "system", "content": "x"})],
            "m",
            "",
            0,
        )
        .unwrap();
        let ids: Vec<String> = list_chats("").into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["b-2", "a-1"]);
        assert_eq!(list_chats("panda")[0].id, "a-1");
        let saved = load_chat("a-1").unwrap();
        assert_eq!(saved["messages"][0]["content"], "What is this?\n[image]");
        assert_eq!(load_chat("b-2").unwrap()["summary"], "sum");
        assert!(load_chat("../../etc/passwd").is_none());
        delete_chat("a-1");
        assert_eq!(list_chats("").len(), 1);
        assert_eq!(
            chat_title(&[json!({"role": "user", "content": "x".repeat(100)})])
                .chars()
                .count(),
            58
        );
    }

    #[test]
    fn python_files_are_readable() {
        let _h = temp_home();
        // A chat written by the Python app.
        fs::create_dir_all(chats_dir()).unwrap();
        fs::write(chats_dir().join("20261113-100000-000001.json"),
                  r#"{"id":"20261113-100000-000001","title":"Battery?","model":"m","updated":1731492000,"messages":[{"role":"user","content":"Battery?"}]}"#).unwrap();
        assert_eq!(list_chats("")[0].title, "Battery?");
        assert!(new_chat_id().len() >= 22);
    }
}
