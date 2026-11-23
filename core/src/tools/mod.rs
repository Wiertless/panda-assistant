//! The built-in tools the model can call. Arguments come from a model, so
//! every tool validates them, and nothing goes through a shell.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::tool::{Args, Risk, Tool, ToolError, arg_str, obj};

pub mod apps;
pub mod brains;
pub mod connect;
pub mod docs;
pub mod edit;
pub mod everything;
pub mod extract;
pub mod files;
pub mod groups;
pub mod images;
pub mod intents;
pub mod localsearch;
pub mod mail;
pub mod music;
pub mod personal;
pub mod research;
pub mod run;
pub mod screen;
pub mod settings;
pub mod system;
pub mod system_actions;
pub mod weather;
pub mod web;

/// Spawn a program and forget it (opening a window, a Settings page…).
pub fn spawn_detached(cmd: &[&str]) -> Result<(), ToolError> {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| ToolError(format!("couldn't start {}: {e}", cmd[0])))
}

/// Asked every time, never "Always Allow": a command can do anything.
pub const ALWAYS_ASK: [&str; 1] = ["run_command"];

pub fn norm_name(text: &str) -> String {
    static POSSESSIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[’'`]s\b").unwrap());
    static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w]+").unwrap());
    let lower = text.to_lowercase();
    NON_WORD
        .replace_all(&POSSESSIVE.replace_all(&lower, ""), " ")
        .trim()
        .to_string()
}

/// The tools that finish what a lookup started ("change it", "connect it"),
/// for requests that ask for an action: a turn that stops after the lookup
/// is asked again with only these (and ask_user).
pub fn completes(lookup: &str) -> &'static [&'static str] {
    match lookup {
        "find_settings" | "get_setting" => &["change_setting"],
        "list_bluetooth" => &["connect_bluetooth", "disconnect_bluetooth"],
        "list_networks" => &["connect_network", "disconnect_network"],
        "list_devices" => &["send_file", "send_text"],
        "find_files" | "search_file_contents" | "list_folder" => &[
            "send_file",
            "ask_document",
            "read_document",
            "read_file",
            "open_file",
            "show_in_files",
        ],
        "list_app_actions" => &["run_app_action"],
        "get_status" => &[
            "set_volume",
            "set_mute",
            "set_brightness",
            "set_dark_mode",
            "set_do_not_disturb",
            "set_night_light",
            "set_power_mode",
            "set_wifi",
            "set_bluetooth",
        ],
        _ => &[],
    }
}

/// Tools that bring text from outside into the chat: a web page, a document,
/// a calendar invite, the clipboard, another app. After one runs, text in
/// the chat may be written to steer the model.
pub fn reads_untrusted(tool: &str) -> bool {
    matches!(
        tool,
        "web_search"
            | "read_web_page"
            | "research"
            | "search_mail"
            | "read_mail"
            | "search_everything"
            | "read_document"
            | "ask_document"
            | "read_file"
            | "search_file_contents"
            | "get_events"
            | "clipboard_history"
            | "read_window"
            | "look_at_image"
            | "find_contact"
            | "rewrite_selection"
    ) || tool.contains("__")
}

/// Tools that send something out, keep something, or change the machine
/// quietly: after untrusted text came in this turn, they ask first.
pub fn guarded_after_untrusted(tool: &str) -> bool {
    matches!(
        tool,
        "send_file"
            | "send_text"
            | "open_url"
            | "remember"
            | "save_skill"
            | "change_setting"
            | "create_event"
            | "draft_email"
            | "run_app_action"
            | "copy_to_clipboard"
            | "connect_network"
            | "write_file"
            | "edit_file"
            | "move_file"
            | "make_folder"
            | "trash_file"
    ) || tool.contains("__")
}

/// Tools whose change `undo` can reverse (the window offers Undo on their pill).
pub const UNDOABLE: [&str; 18] = [
    "write_file",
    "edit_file",
    "move_file",
    "make_folder",
    "trash_file",
    "create_event",
    "set_dark_mode",
    "set_do_not_disturb",
    "set_night_light",
    "set_volume",
    "set_mute",
    "set_brightness",
    "set_wifi",
    "set_bluetooth",
    "set_power_mode",
    "change_setting",
    "set_reminder",
    "remember",
];

/// What to do with a lookup's result, appended to it so the model keeps the
/// thread ("found it" → "now use it"). Only for results that found something.
/// Tools whose results the window shows as a card instead of a pill.
pub const CARDS: [&str; 3] = ["get_weather", "get_events", "list_reminders"];

pub fn next_step(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "find_files" | "search_file_contents" | "list_folder" => {
            "(To send one, pass its exact path to send_file; to answer what it says, ask_document with the question.)"
        }
        "list_devices" => "(To send something, pass one of these names to send_file or send_text.)",
        "list_networks" => "(To connect, pass the network's name to connect_network.)",
        "list_bluetooth" => "(To connect, pass the device's name to connect_bluetooth.)",
        "find_settings" => {
            "(To change one, call change_setting with its schema, key and the new value.)"
        }
        "get_setting" => "(To change it, call change_setting with a new value.)",
        "list_app_actions" => "(To do one, call run_app_action with the app and the action.)",
        "find_contact" => "(Answer with the details asked for.)",
        // The window shows these as a card.
        n if CARDS.contains(&n) => {
            "(Answer in one short sentence at most: the user sees this as a card, so don't repeat it.)"
        }
        _ => return None,
    })
}

/// Names that share a word (or a word's start) with the query, best first, so
/// an error can say "did you mean" instead of leaving the model stuck.
pub fn close_to(query: &str, names: &[String], n: usize) -> Vec<String> {
    let words = |s: &str| {
        norm_name(&s.replace(['_', '-', '.'], " "))
            .split(' ')
            .filter(|w| w.chars().count() >= 3)
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let q = words(query);
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .map(|name| {
            let ws = words(name);
            let score = q
                .iter()
                .filter(|w| {
                    ws.iter()
                        .any(|x| x.starts_with(w.as_str()) || w.starts_with(x.as_str()))
                })
                .count();
            (score, name)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
    let mut out: Vec<String> = vec![];
    for (_, name) in scored {
        if !out.contains(name) && out.len() < n {
            out.push(name.clone());
        }
    }
    out
}

/// The one name the query means: exact, then prefix, then substring, then all words.
pub fn pick(query: &str, names: &[String], what: &str) -> Result<String, ToolError> {
    static FILLER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(my|the|a|an|to|moje?|m[oó]j)\s+").unwrap());
    let q = norm_name(&FILLER.replace(query.trim(), ""));
    if q.is_empty() {
        return Err(format!("say which {what}").into());
    }
    let normed: Vec<(&String, String)> = names.iter().map(|n| (n, norm_name(n))).collect();
    // Last: any real word ("iphone (jakub)" -> the one device with iphone in its name).
    let words: Vec<&str> = q.split(' ').filter(|w| w.chars().count() >= 3).collect();
    let tests: [&dyn Fn(&str) -> bool; 5] = [
        &|n| n == q,
        &|n| n.starts_with(&q),
        &|n| n.contains(&q),
        &|n| q.split(' ').all(|w| n.contains(w)),
        &|n| {
            words.iter().any(|w| {
                n.split(' ')
                    .any(|nw| nw.contains(w) || (nw.len() >= 3 && w.contains(nw)))
            })
        },
    ];
    for test in tests {
        let mut hits: Vec<&String> = normed
            .iter()
            .filter(|(_, n)| test(n))
            .map(|(name, _)| *name)
            .collect();
        hits.dedup();
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0].clone()),
            _ => {
                return Err(format!(
                    "more than one {what} matches '{query}': {}",
                    hits.iter()
                        .take(8)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into());
            }
        }
    }
    let known = if names.is_empty() {
        "none".to_string()
    } else {
        names
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(format!("no {what} called '{query}' (known: {known})").into())
}

fn text(
    key: &'static str,
    f: impl Fn(String) -> String + Send + Sync + 'static,
) -> impl Fn(&Args) -> String + Send + Sync {
    move |a| f(arg_str(a, key))
}

fn enabled() -> serde_json::Value {
    obj(json!({"enabled": {"type": "boolean"}}), &["enabled"])
}

fn onoff(key: &'static str, what: &'static str) -> impl Fn(&Args) -> String + Send + Sync {
    move |a| match crate::tool::arg_bool(a, key) {
        Ok(on) => format!("Turn {what} {}", if on { "on" } else { "off" }),
        Err(_) => format!("Change {what}"),
    }
}

fn or(s: String, default: &str) -> String {
    if s.is_empty() { default.to_string() } else { s }
}

/// Every built-in tool (the agent filters by settings and model abilities).
pub fn all() -> Vec<Tool> {
    use Risk::*;
    let path = || obj(json!({"path": {"type": "string"}}), &["path"]);
    vec![
        Tool::new("open_app", "Open an app. When the user names an app ('Firefox', 'Spotify', 'Zed'), pass that exact \
name, even if it looks misspelled. Only when they name none, pass the kind (browser, email, files, calendar, music, \
video, photos, text editor, pdf, terminal, settings) to open their default app for it.",
                  obj(json!({"name": {"type": "string", "description": "The app's name as the user said it, or a kind of app"}}), &["name"]),
                  Safe, apps::open_app, text("name", |n| format!("Open {}", or(n, "an app")))),
        Tool::new("open_url", "Open a web address in the default browser.", obj(json!({"url": {"type": "string"}}), &["url"]),
                  Safe, apps::open_url, text("url", |u| format!("Open {}", or(u, "a link")))),
        Tool::new("open_file", "Open (\"otwórz\") a file or folder with its default app, to show it to the user. Paths can be relative to the home \
folder, start with ~, or start with a folder name like Downloads or Documents.", path(), Safe, files::open_file,
                  text("path", |p| format!("Open {}", or(p, "a file")))),
        Tool::new("show_in_files", "Show a file or folder in the file manager (opens its folder with it selected).",
                  path(), Safe, intents::show_in_files, text("path", |p| format!("Show {} in Files", or(p, "it")))),
        Tool::new("play_music", "Play a song, album or artist from the user's own music library (not the web), in the \
app they name (Gapless, Rhythmbox, Spotify…) or their default player. query: title, album or artist as said.",
                  obj(json!({"query": {"type": "string"}, "app": {"type": "string", "description": "Only when the user names one"}}), &["query"]),
                  Safe, music::play_music, text("query", |q| format!("Play {q}"))),
        Tool::new("search_apps", "Search inside the user's apps, like the GNOME overview does: Calculator for sums, \
Characters for emoji, Weather, Settings pages, Files, Contacts and more.",
                  obj(json!({"query": {"type": "string"}}), &["query"]), Safe, intents::search_apps,
                  text("query", |q| format!("Search apps for “{q}”"))),
        Tool::new("open_search_result", "Open one of search_apps' results in its app.",
                  obj(json!({"app": {"type": "string", "description": "The app's id from search_apps"},
                             "id": {"type": "string"}, "query": {"type": "string", "description": "The search used"}}), &["app", "id"]),
                  Safe, intents::open_search_result, |_| "Open the result".into()),
        Tool::new("open_terminal", "Open the user's terminal app in a folder (default: home).",
                  obj(json!({"folder": {"type": "string"}}), &[]), Safe, intents::open_terminal,
                  text("folder", |f| format!("Open a terminal in {}", or(f, "your home folder")))),
        Tool::new("list_folder", "List what's in a folder (default: the home folder).",
                  obj(json!({"path": {"type": "string", "description": "e.g. Downloads, Documents/work, ~"}}), &[]),
                  Safe, files::list_folder, text("path", |p| format!("Look in {}", or(p, "your home folder")))),
        Tool::new("find_files", "Find files and folders by part of their name, newest first. Use it whenever the user \
mentions one of their own documents or files (\"my lease\", \"the invoice\", \"my CV\"), to read or send it.",
                  obj(json!({"name": {"type": "string"}, "folder": {"type": "string", "description": "Where to look (optional)"}}), &["name"]),
                  Safe, files::find_files, text("name", |n| format!("Find files named “{n}”"))),
        Tool::new("read_file", "Read any file's text: notes, code, configs, CSV, PDF, Word, Excel, PowerPoint, \
LibreOffice, EPUB, RTF, e-mail, notebooks. Long files come a part at a time: pass from_line to read on. For a \
question about a long document, ask_document is better.",
                  obj(json!({"path": {"type": "string"},
                             "from_line": {"type": "integer", "description": "First line to read (default 1)"}}), &["path"]), Safe,
                  files::read_file, text("path", |p| format!("Read {}", or(p, "a file")))),
        Tool::new("write_file", "Create a text file (a note, list, script, letter…) with the given content; a path \
ending in .pdf makes a PDF from Markdown content (\"put that into a PDF\"). To change an existing file use edit_file; \
overwrite true replaces a whole existing file.",
                  obj(json!({"path": {"type": "string", "description": "e.g. Documents/shopping.txt"},
                             "content": {"type": "string"},
                             "overwrite": {"type": "boolean"}}), &["path", "content"]), Confirm,
                  edit::write_file, text("path", |p| format!("Write {}", or(p, "a file"))))
            .with_detail(edit::write_detail),
        Tool::new("edit_file", "Change part of a text file: replaces old_text, copied exactly from the file (read_file \
it first), with new_text. old_text must appear once, or pass all true to change every place.",
                  obj(json!({"path": {"type": "string"}, "old_text": {"type": "string"}, "new_text": {"type": "string"},
                             "all": {"type": "boolean"}}), &["path", "old_text", "new_text"]), Confirm,
                  edit::edit_file, text("path", |p| format!("Edit {}", or(p, "a file"))))
            .with_detail(edit::edit_detail),
        Tool::new("move_file", "Move or rename a file or folder. to: a new name or path, or a folder to move it into.",
                  obj(json!({"from": {"type": "string"}, "to": {"type": "string"}}), &["from", "to"]), Confirm,
                  edit::move_file, |a| format!("Move {} to {}", arg_str(a, "from"), arg_str(a, "to"))),
        Tool::new("make_folder", "Make a new folder (and any missing folders above it).", path(), Safe,
                  edit::make_folder, text("path", |p| format!("Make the folder {}", or(p, "")))),
        Tool::new("trash_file", "Move a file or folder to the Trash (it can be restored). Use it when the user asks to \
delete or remove a file.", path(), Confirm, edit::trash_file,
                  text("path", |p| format!("Move {} to the Trash", or(p, "it")))),
        Tool::new("search_text", "Search inside text files (notes, code, configs, CSV) in a folder for words, like \
grep: returns file:line: matching line. For PDFs and office documents use search_file_contents.",
                  obj(json!({"query": {"type": "string"},
                             "folder": {"type": "string", "description": "Where to look (default: home)"}}), &["query"]),
                  Safe, edit::search_text, text("query", |q| format!("Search text files for “{q}”"))),
        Tool::new("get_battery", "Get the battery charge level and charging state.", obj(json!({}), &[]), Safe,
                  system::get_battery, |_| "Read battery".into()),
        Tool::new("get_system_info", "Get OS, kernel, uptime, memory and disk space.", obj(json!({}), &[]), Safe,
                  system::get_system_info, |_| "Read system info".into()),
        Tool::new("list_servers", "List the programs on this computer that listen for connections (local \
servers: Lemonade, FastFlowLM, Ollama, web dev servers, databases) with their address and port. Use it for which \
port or URL a program or service is on, or whether a server is running.", obj(json!({}), &[]), Safe,
                  system::list_servers, |_| "List local servers".into()),
        Tool::new("get_datetime", "Get the current local date and time.", obj(json!({}), &[]), Safe,
                  system::get_datetime, |_| "Read the time".into()),
        Tool::new("calculate", "Work out arithmetic exactly; use it for any sum, percentage, average or \
conversion instead of working it out yourself. expression: numbers with + - * / ^ and brackets; 15% means 0.15 \
(\"15% of 240\" is \"15% * 240\").",
                  obj(json!({"expression": {"type": "string"}}), &["expression"]), Safe,
                  system::calculate, |a| format!("Calculate {}", crate::tool::arg_str(a, "expression"))),
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
        Tool::new("get_status", "Get dark mode, Do Not Disturb, Wi-Fi, volume and brightness state.", obj(json!({}), &[]),
                  Safe, settings::get_status, |_| "Read settings".into()),
        Tool::new("web_search", "Search the web for current information, facts, news, prices or how-tos. \
Returns titles, links and short snippets. Not for the user's own files, settings or devices: use the tools for those. \
When the user asks to research or look into something, even quickly, use research instead.", obj(json!({"query": {"type": "string", "description": "What to search for"}}), &["query"]),
                  Safe, web::web_search, text("query", |q| format!("Search the web for “{q}”"))),
        Tool::new("read_web_page", "Read the text of a web page, e.g. a search result, to answer from it.",
                  obj(json!({"url": {"type": "string"}}), &["url"]), Safe, web::read_web_page,
                  text("url", |u| format!("Read {}", or(u, "a page")))),
        Tool::new("get_weather", "The weather now and for the next three days in a city or town.",
                  obj(json!({"place": {"type": "string", "description": "City or town, e.g. Toruń"}}), &["place"]),
                  Safe, weather::get_weather, text("place", |p| format!("Weather in {p}"))),
        Tool::new("search_everything", "Search all of the user's own things at once: calendar, mail, file names, \
what's inside documents, and contacts. Use it when the answer is somewhere in their stuff but you can't tell where \
(\"when's my flight?\", \"what did the landlord say about the deposit?\", \"find anything about the conference\"). \
query: the key words, not a whole sentence.",
                  obj(json!({"query": {"type": "string", "description": "e.g. flight Lisbon, deposit, conference"}}), &["query"]),
                  Safe, everything::search_everything, text("query", |q| format!("Search your things for “{q}”"))),
        Tool::new("search_mail", "Search the user's mail (every account in Online Accounts, whatever mail app they \
use) for words: a sender, a subject, a topic. Returns the newest matches with sender, subject, date and the start of \
the text. Empty query: the newest mail.",
                  obj(json!({"query": {"type": "string", "description": "e.g. Ola lease, faktura, flight"}}), &[]),
                  Safe, mail::search_mail, text("query", |q| format!("Search mail for “{q}”"))),
        Tool::new("read_mail", "Read a whole message that search_mail found, by its id in brackets.",
                  obj(json!({"id": {"type": "string"}}), &["id"]), Safe, mail::read_mail, |_| "Read a message".into()),
        Tool::new("research", "Research a topic in depth on the web, like a researcher: plans a report, searches and \
reads up to 14 pages, checks what's still missing and searches for that too (up to 20 sources), and writes a report \
with sections and cited sources, shown to the user as it is. Takes a few minutes; for \"quick research\" or a short look it reads 5 pages and answers in under a minute. Use \
it when the user asks to research, look into, review or compare something. save_as pdf, md or txt saves the report in Documents/Research. For one quick \
fact, web_search is enough.",
                  obj(json!({"question": {"type": "string", "description": "The full question with any details the user gave, in the user's own language (the report is written in it)"},
                             "save_as": {"type": "string", "enum": ["pdf", "md", "txt"], "description": "Only when the user asks for a file"}}), &["question"]),
                  Safe, research::research, text("question", |q| format!("Research “{q}”"))).with_progress(),
        Tool::new("copy_to_clipboard", "Copy text to the clipboard.", obj(json!({"text": {"type": "string"}}), &["text"]),
                  Safe, settings::copy_to_clipboard, |_| "Copy to clipboard".into()),
        Tool::new("undo", "Undo the last change you made (a setting, the volume, brightness, Wi-Fi, a reminder, \
something remembered, a file written, moved, edited or made) when the user says undo, revert, change it back or \
cofnij. whole_task true undoes everything the last task changed (\"undo all that\", \"cofnij wszystko\").",
                  obj(json!({"whole_task": {"type": "boolean"}}), &[]), Safe,
                  brains::undo, |_| "Undo the last change".into()),
        Tool::new("set_reminder", "Remind the user of something later, or set a timer (leave text empty). Give \
in_minutes, or at as HH:MM or YYYY-MM-DD HH:MM in local time (today's date is in the prompt).",
                  obj(json!({"text": {"type": "string", "description": "What to remind them of"},
                             "in_minutes": {"type": "number"}, "at": {"type": "string"}}), &[]),
                  Safe, crate::reminders::set_reminder, text("text", |t| format!("Remind you: {}", or(t, "timer")))),
        Tool::new("list_reminders", "List the reminders and timers that are set.", obj(json!({}), &[]), Safe,
                  crate::reminders::list_reminders, |_| "List reminders".into()),
        Tool::new("cancel_reminder", "Cancel a reminder or timer by what it's about or its time.",
                  obj(json!({"which": {"type": "string"}}), &[]), Safe, crate::reminders::cancel_reminder,
                  text("which", |w| format!("Cancel the reminder {}", or(w, "")))),
        Tool::new("rewrite_selection", "Rewrite the text the user has selected (highlighted) in any app: proofread, \
shorten, make it formal, translate… The result goes to the clipboard to paste. Use it for \"this\", \"the selected \
text\", \"popraw to\". You can't see the selection; this tool reads it, so call it instead of asking for the text.", obj(json!({"instruction": {"type": "string", "description": "What to do with it, e.g. proofread"},
                             "text": {"type": "string", "description": "Only when the user gave the text in the message"}}), &["instruction"]),
                  Safe, settings::rewrite_selection, text("instruction", |i| format!("Rewrite the selection: {}", or(i, "proofread")))),
        Tool::new("clipboard_history", "What the user copied recently, newest first (text only). Use it for \"what I \
copied\", \"the link from before\", \"paste that address\". query keeps only clips with those words.",
                  obj(json!({"query": {"type": "string"}}), &[]), Safe, settings::clipboard_history,
                  |_| "Look at what you copied".into()),
        Tool::new("set_dark_mode", "Turn the dark style on or off.", enabled(), Safe, settings::set_dark_mode,
                  onoff("enabled", "dark mode")),
        Tool::new("set_do_not_disturb", "Turn Do Not Disturb (hide notification banners) on or off.", enabled(), Safe,
                  settings::set_do_not_disturb, onoff("enabled", "Do Not Disturb")),
        Tool::new("set_volume", "Set the output volume in percent (0-100): also for louder or quieter (\"głośniej\", \
\"ciszej\", \"ścisz\", \"podgłośnij\"), a step of about 15.",
                  obj(json!({"percent": {"type": "integer", "minimum": 0, "maximum": 100}}), &["percent"]), Safe,
                  settings::set_volume, text("percent", |p| format!("Set volume to {p}%"))),
        Tool::new("set_mute", "Mute or unmute the sound output.", obj(json!({"muted": {"type": "boolean"}}), &["muted"]),
                  Safe, settings::set_mute, onoff("muted", "mute")),
        Tool::new("set_brightness", "Set the screen brightness in percent (1-100): also when the screen is too bright or \
too dark (\"przyciemnij\", \"rozjaśnij\"), a step of about 20.",
                  obj(json!({"percent": {"type": "integer", "minimum": 1, "maximum": 100}}), &["percent"]), Safe,
                  settings::set_brightness, text("percent", |p| format!("Set brightness to {p}%"))),
        Tool::new("set_power_mode", "Set the power mode: power-saver, balanced or performance.",
                  obj(json!({"mode": {"type": "string", "enum": ["power-saver", "balanced", "performance"]}}), &["mode"]),
                  Safe, settings::set_power_mode, text("mode", |m| {
                      let label = settings::POWER_MODES.iter().find(|(k, _)| *k == m).map(|(_, l)| l.to_string());
                      format!("Set power mode to {}", label.unwrap_or(m))
                  })),
        Tool::new("set_night_light", "Turn Night Light (warmer screen colors) on or off.", enabled(), Safe,
                  settings::set_night_light, onoff("enabled", "Night Light")),
        Tool::new("find_settings", "Find GNOME settings by words, e.g. 'text scaling' or 'touchpad tap'. \
Returns schema and key names to use with get_setting and change_setting.",
                  obj(json!({"query": {"type": "string"}}), &["query"]), Safe, settings::find_settings,
                  text("query", |q| format!("Look up settings for “{q}”"))),
        Tool::new("get_setting", "Read the current value of a GNOME setting.",
                  obj(json!({"schema": {"type": "string"}, "key": {"type": "string"}}), &["schema", "key"]), Safe,
                  settings::get_setting, text("key", |k| format!("Read {}", or(k, "a setting")))),
        Tool::new("change_setting", "Change a GNOME desktop setting. value is in GVariant text form, \
e.g. true, 1.25, 'prefer-dark', 300.",
                  obj(json!({"schema": {"type": "string"}, "key": {"type": "string"}, "value": {"type": "string"}}),
                      &["schema", "key", "value"]), Safe, settings::change_setting,
                  |a| format!("Set {} to {}", or(arg_str(a, "key"), "a setting"), arg_str(a, "value")))
            .with_detail(|a| format!("{} {} = {}", arg_str(a, "schema"), arg_str(a, "key"), arg_str(a, "value"))),
        Tool::new("open_settings", "Open GNOME Settings at a page, e.g. display, power, keyboard, sound.",
                  obj(json!({"panel": {"type": "string"}}), &["panel"]), Safe, settings::open_settings,
                  text("panel", |p| format!("Open Settings: {p}"))),
        Tool::new("list_devices", "List the user's other devices on Tailscale (for Taildrop).", obj(json!({}), &[]), Safe,
                  connect::list_devices, |_| "List Tailscale devices".into()),
        Tool::new("send_file", "Send a file to one of the user's devices with Taildrop (Tailscale): a document, photo or \
screenshot, e.g. one find_files found. Pass the file's path; the file itself is sent.",
                  obj(json!({"path": {"type": "string"}, "device": {"type": "string", "description": "Device name"}}),
                      &["path", "device"]), Safe, connect::send_file, // Taildrop only reaches your own devices
                  |a| format!("Send {} to {}", base_name(&or(arg_str(a, "path"), "a file")), or(arg_str(a, "device"), "a device"))),
        Tool::new("send_text", "Send a note of text to one of the user's devices with Taildrop: an answer, a list, an \
address, a link. For the last answer, pass its text. Not for files: to send a file, use send_file with its path.",
                  obj(json!({"text": {"type": "string"}, "device": {"type": "string", "description": "Device name"},
                             "title": {"type": "string", "description": "A short name for the note"}}), &["text", "device"]),
                  Safe, connect::send_text, |a| format!("Send a note to {}", or(arg_str(a, "device"), "a device"))),
        Tool::new("set_wifi", "Turn Wi-Fi on or off. Turning it off disconnects the network.", enabled(), Safe,
                  settings::set_wifi, onoff("enabled", "Wi-Fi")),
        Tool::new("set_bluetooth", "Turn Bluetooth on or off. Turning it off disconnects Bluetooth devices.", enabled(),
                  Safe, settings::set_bluetooth, onoff("enabled", "Bluetooth")),
        Tool::new("list_networks", "List saved networks (Wi-Fi, wired, VPN) and which are connected or in range.",
                  obj(json!({}), &[]), Safe, connect::list_networks, |_| "List saved networks".into()),
        Tool::new("connect_network", "Connect to a saved Wi-Fi network, wired connection or VPN by name.",
                  obj(json!({"name": {"type": "string"}}), &["name"]), Safe, connect::connect_network,
                  text("name", |n| format!("Connect to {}", or(n, "a network")))),
        Tool::new("disconnect_network", "Disconnect a connected network or VPN by name.",
                  obj(json!({"name": {"type": "string"}}), &["name"]), Safe, connect::disconnect_network,
                  text("name", |n| format!("Disconnect {}", or(n, "a network")))),
        Tool::new("list_bluetooth", "List paired Bluetooth devices (headphones, mice…) and which are connected. \
To connect one, call connect_bluetooth right away (it finds the device by everyday name).",
                  obj(json!({}), &[]), Safe, connect::list_bluetooth, |_| "List Bluetooth devices".into()),
        Tool::new("connect_bluetooth", "Connect a paired Bluetooth device by name, e.g. 'AirPods'. \
Turns Bluetooth on if needed.", obj(json!({"device": {"type": "string"}}), &["device"]), Safe,
                  connect::connect_bluetooth, text("device", |d| format!("Connect {}", or(d, "a device")))),
        Tool::new("disconnect_bluetooth", "Disconnect a connected Bluetooth device by name.",
                  obj(json!({"device": {"type": "string"}}), &["device"]), Safe, connect::disconnect_bluetooth,
                  text("device", |d| format!("Disconnect {}", or(d, "a device")))),
        Tool::new("save_screenshot", "Take a screenshot and save it as a file, to send, share or keep. Returns its path.",
                  obj(json!({}), &[]), Safe, system_actions::save_screenshot, |_| "Take a screenshot".into()),
        Tool::new("look_at_screen", "Take a screenshot to see what's on the user's screen right now. The picture is \
only for you and isn't kept; use save_screenshot for one to send or keep.", obj(json!({}), &[]),
                  Safe, system_actions::look_at_screen, |_| "Look at the screen".into()),
        Tool::new("look_at_image", "Look at an image file (photo, screenshot, picture).", path(), Safe,
                  system_actions::look_at_image, text("path", |p| format!("Look at {}", or(p, "an image")))),
        Tool::new("edit_image", "Change pictures: make them smaller (resize: 50% or the longest side in pixels, \
like 1920), convert (format: jpg, png, webp, tif), compress (quality 1-100), turn them (rotate: 90, 180, 270) or take \
out the location and camera details (remove_location true). files: one picture, or several with * and ? in the name \
(Pictures/Trip/*.jpg). The results go to Pictures/Edited; the originals stay as they are.",
                  obj(json!({"files": {"type": "string"}, "resize": {"type": "string"}, "format": {"type": "string"},
                             "quality": {"type": "integer"}, "rotate": {"type": "integer"},
                             "remove_location": {"type": "boolean"}}), &["files"]), Safe,
                  images::edit_image, text("files", |p| format!("Edit {}", or(p, "pictures")))),
        Tool::new("images_to_pdf", "Put pictures into one PDF, a page each, in name order. files: one picture or \
several with * and ? in the name (Pictures/Scans/*.jpg); name: the PDF's name. It goes to Pictures/Edited.",
                  obj(json!({"files": {"type": "string"}, "name": {"type": "string"}}), &["files"]), Safe,
                  images::images_to_pdf, text("files", |p| format!("Make a PDF of {}", or(p, "the pictures")))),
        Tool::new("read_window", "Read the text of the focused window (web page, document, email…) and what's selected \
in it. Use it when the user says this, here, or asks about what they're looking at.",
                  obj(json!({"app": {"type": "string", "description": "Only to pick a window other than the focused one"}}), &[]),
                  Safe, screen::read_window, text("app", |a| format!("Read {}", or(a, "the window")))),
        Tool::new("search_file_contents", "Search inside the user's files (documents, PDFs, notes) for words, \
using GNOME's file index. Returns files and matching snippets.", obj(json!({"query": {"type": "string"}}), &["query"]),
                  Safe, localsearch::search_file_contents, text("query", |q| format!("Search files for “{q}”"))),
        Tool::new("read_document", "Read the text of a document (PDF, Word, Excel, PowerPoint, LibreOffice, EPUB…), \
the start of it for long ones.", path(),
                  Safe, localsearch::read_document, text("path", |p| format!("Read {}", base_name(&or(p, "a document"))))),
        Tool::new("get_events", "Get calendar events. when: today, tomorrow, this week, next week, a weekday or a \
date like 2026-11-20.", obj(json!({"when": {"type": "string"}}), &[]), Safe, personal::get_events,
                  text("when", |w| format!("Check the calendar ({})", or(w, "today")))),
        Tool::new("ask_document", "Answer a question about what a document says, or summarize it (PDF, office \
document, text), however long: it reads the parts that matter. path from find_files.",
                  obj(json!({"path": {"type": "string"}, "question": {"type": "string", "description": "The user's question, or \"summarize\""}}),
                      &["path", "question"]),
                  Safe, docs::ask_document, text("path", |p| format!("Read {}", base_name(&or(p, "a document"))))),
        Tool::new("create_event", "Add an event to the user's calendar. start: HH:MM, a weekday and time (\"friday \
13:00\"), or YYYY-MM-DD HH:MM, local time; minutes (default 60) or end.",
                  obj(json!({"title": {"type": "string"}, "start": {"type": "string"}, "minutes": {"type": "number"},
                             "end": {"type": "string"}, "place": {"type": "string"}}), &["title", "start"]),
                  Safe, personal::create_event, text("title", |t| format!("Add “{}” to the calendar", or(t, "an event")))),
        Tool::new("draft_email", "Open a ready email draft in the user's mail app; they check it and press Send. to: an \
address, or a contact's name (looked up in their contacts).",
                  obj(json!({"to": {"type": "string"}, "subject": {"type": "string"}, "body": {"type": "string"}}), &["body"]),
                  Safe, personal::draft_email, text("to", |t| format!("Draft an email to {}", or(t, "someone")))),
        Tool::new("find_contact", "Find a contact's email addresses and phone numbers by name.",
                  obj(json!({"name": {"type": "string"}}), &["name"]), Safe, personal::find_contact,
                  text("name", |n| format!("Look up {}", or(n, "a contact")))),
        Tool::new("media_control", "Control music and video players: play, pause, play_pause, next_track (skip this \
song), previous_track (go back a song), stop, or status (only to say what's playing: \"what song is this\", \"co teraz leci\").", obj(json!({"action": {"type": "string", "enum": ["play", "pause", "play_pause", "next_track",
                  "previous_track", "stop", "status"]}, "player": {"type": "string", "description": "Optional, e.g. Spotify"}}), &["action"]),
                  Safe, connect::media_control, text("action", |a| format!("Media: {}", a.replace('_', "/")))),
        Tool::new("run_command", "Run a shell command inside the user's 'zed' toolbox container (Fedora with dev \
tools; the home folder is shared). For building, compiling, git, scripts. Output is returned.",
                  obj(json!({"command": {"type": "string"}, "folder": {"type": "string", "description": "Working folder"}}), &["command"]),
                  Confirm, system_actions::run_command, |_| "Run a command in the zed toolbox".into())
            .with_detail(system_actions::command_detail),
        Tool::new("run_app_action", "Run one of an app's actions, e.g. app firefox, action New Private Window.",
                  obj(json!({"app": {"type": "string"}, "action": {"type": "string"}}), &["app", "action"]), Safe,
                  apps::run_app_action, |a| format!("{}: {}", or(arg_str(a, "app"), "App"), arg_str(a, "action"))),
    ]
}

/// File name of a path argument, for summaries.
pub fn base_name(raw: &str) -> String {
    Path::new(raw)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_by_everyday_name() {
        let names: Vec<String> = ["Wiertless's AirPods Pro", "MX Master 3", "JBL Flip 5"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            pick("my airpods", &names, "device").unwrap(),
            "Wiertless's AirPods Pro"
        );
        assert_eq!(pick("mx master", &names, "device").unwrap(), "MX Master 3");
        assert_eq!(pick("flip", &names, "device").unwrap(), "JBL Flip 5");
        assert!(pick("headphones", &names, "device").is_err());
        for t in all() {
            if let Some(hint) = next_step(&t.name) {
                assert!(
                    all()
                        .iter()
                        .any(|o| hint.contains(&o.name) || hint.starts_with("(Answer")),
                    "{}: {hint}",
                    t.name
                );
            }
        }
        let keys: Vec<String> = ["text-scaling-factor", "font-name", "cursor-size"]
            .map(String::from)
            .to_vec();
        assert_eq!(close_to("text scale", &keys, 3), ["text-scaling-factor"]);
        assert!(close_to("wallpaper", &keys, 3).is_empty());
        let peers: Vec<String> = ["jakubs-iphone", "framework", "ipad-mini"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            pick("iphone (jakub)", &peers, "device").unwrap(),
            "jakubs-iphone"
        );
        assert_eq!(
            pick("Jakub's iPhone", &peers, "device").unwrap(),
            "jakubs-iphone"
        );
        let peers: Vec<String> = ["iphone-15", "framework"].map(String::from).to_vec();
        assert_eq!(
            pick("iphone (jakub)", &peers, "device").unwrap(),
            "iphone-15"
        );
        let nets: Vec<String> = ["Home 5G", "Home 2.4"].map(String::from).to_vec();
        assert!(
            pick("home", &nets, "network")
                .unwrap_err()
                .0
                .contains("more than one")
        );
    }

    #[test]
    fn same_tools_and_risks_as_the_python_app() {
        let names: Vec<String> = all().into_iter().map(|t| t.name).collect();
        assert_eq!(names.len(), 80, "{names:?}");
        let risky: Vec<String> = all()
            .into_iter()
            .filter(|t| t.risk == Risk::Confirm)
            .map(|t| t.name)
            .collect();
        assert_eq!(
            risky,
            [
                "write_file",
                "edit_file",
                "move_file",
                "trash_file",
                "run_command"
            ]
        );
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
