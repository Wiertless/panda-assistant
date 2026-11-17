//! What the assistant did: every action, when, from which chat, and how it
//! went (activity.jsonl in the state folder, newest last, the last 500 kept).
//! Preferences shows it, so running without asking stays easy to check.

use std::fs;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::paths;

const KEEP: usize = 500;
const RESULT_CHARS: usize = 240;

fn file() -> std::path::PathBuf {
    paths::state_dir().join("activity.jsonl")
}

pub struct Entry<'a> {
    pub chat_id: &'a str,
    pub tool: &'a str,
    pub summary: &'a str,
    pub result: &'a str,
    pub ok: bool,
    /// "window" or "search".
    pub from: &'a str,
    /// For actions that asked: "allowed", "always" or "declined".
    pub decision: Option<&'a str>,
}

pub fn record(e: &Entry) {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut line = json!({"time": time, "chat_id": e.chat_id, "tool": e.tool, "summary": e.summary,
                          "result": e.result.chars().take(RESULT_CHARS).collect::<String>(), "ok": e.ok, "from": e.from});
    if let Some(d) = e.decision {
        line["decision"] = json!(d);
    }
    let path = file();
    let _ = fs::create_dir_all(path.parent().unwrap());
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
    trim(&path);
}

fn trim(path: &std::path::Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > KEEP + 50 {
        let _ = fs::write(path, lines[lines.len() - KEEP..].join("\n") + "\n");
    }
}

/// The newest `n` entries, newest first.
pub fn recent(n: usize) -> Vec<Value> {
    let text = fs::read_to_string(file()).unwrap_or_default();
    text.lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(n)
        .collect()
}

pub fn clear() {
    let _ = fs::remove_file(file());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;

    #[test]
    fn records_newest_first_and_trims() {
        let _h = temp_home();
        for i in 0..(KEEP + 60) {
            record(&Entry {
                chat_id: "c",
                tool: "set_volume",
                summary: &format!("Set volume {i}"),
                result: "ok",
                ok: true,
                from: "window",
                decision: None,
            });
        }
        let all = recent(10_000);
        assert!(all.len() <= KEEP + 50 && all.len() >= KEEP);
        assert_eq!(all[0]["summary"], format!("Set volume {}", KEEP + 59));
        record(&Entry {
            chat_id: "c",
            tool: "run_command",
            summary: "Run ls",
            result: &"x".repeat(1000),
            ok: true,
            from: "search",
            decision: Some("allowed"),
        });
        let last = &recent(1)[0];
        assert_eq!(
            (
                last["decision"].as_str(),
                last["result"].as_str().unwrap().len()
            ),
            (Some("allowed"), RESULT_CHARS)
        );
        clear();
        assert!(recent(5).is_empty());
    }
}
