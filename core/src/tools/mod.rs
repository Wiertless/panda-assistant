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
pub mod files;
pub mod groups;
pub mod intents;
pub mod localsearch;
pub mod personal;
pub mod run;
pub mod screen;
pub mod settings;
pub mod system;
pub mod system_actions;
pub mod web;

/// Spawn a program and forget it (opening a window, a Settings page…).
pub fn spawn_detached(cmd: &[&str]) -> Result<(), ToolError> {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(cmd[0]).args(&cmd[1..])
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn().map(|_| ()).map_err(|e| ToolError(format!("couldn't start {}: {e}", cmd[0])))
}

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
    // Last: any real word ("iphone (jakub)" -> the one device with iphone in its name).
    let words: Vec<&str> = q.split(' ').filter(|w| w.chars().count() >= 3).collect();
    let tests: [&dyn Fn(&str) -> bool; 5] = [
        &|n| n == q, &|n| n.starts_with(&q), &|n| n.contains(&q),
        &|n| q.split(' ').all(|w| n.contains(w)),
        &|n| words.iter().any(|w| n.split(' ').any(|nw| nw.contains(w) || (nw.len() >= 3 && w.contains(nw)))),
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
        Tool::new("open_app", "Open an app by its name ('Zed') or by what it is: browser, email, files, calendar, \
music, video, photos, text editor, pdf, terminal or settings opens the user's default app for it.",
                  obj(json!({"name": {"type": "string", "description": "App name or kind of app"}}), &["name"]),
                  Safe, apps::open_app, text("name", |n| format!("Open {}", or(n, "an app")))),
        Tool::new("open_url", "Open a web address in the default browser.", obj(json!({"url": {"type": "string"}}), &["url"]),
                  Safe, apps::open_url, text("url", |u| format!("Open {}", or(u, "a link")))),
        Tool::new("open_file", "Open a file or folder with its default app. Paths can be relative to the home \
folder, start with ~, or start with a folder name like Downloads or Documents.", path(), Safe, files::open_file,
                  text("path", |p| format!("Open {}", or(p, "a file")))),
        Tool::new("show_in_files", "Show a file or folder in the file manager (opens its folder with it selected).",
                  path(), Safe, intents::show_in_files, text("path", |p| format!("Show {} in Files", or(p, "it")))),
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
        Tool::new("get_status", "Get dark mode, Do Not Disturb, Wi-Fi, volume and brightness state.", obj(json!({}), &[]),
                  Safe, settings::get_status, |_| "Read settings".into()),
        Tool::new("web_search", "Search the web for current information, facts, news, prices or how-tos. \
Returns titles, links and short snippets.", obj(json!({"query": {"type": "string", "description": "What to search for"}}), &["query"]),
                  Safe, web::web_search, text("query", |q| format!("Search the web for “{q}”"))),
        Tool::new("read_web_page", "Read the text of a web page, e.g. a search result, to answer from it.",
                  obj(json!({"url": {"type": "string"}}), &["url"]), Safe, web::read_web_page,
                  text("url", |u| format!("Read {}", or(u, "a page")))),
        Tool::new("copy_to_clipboard", "Copy text to the clipboard.", obj(json!({"text": {"type": "string"}}), &["text"]),
                  Safe, settings::copy_to_clipboard, |_| "Copy to clipboard".into()),
        Tool::new("set_dark_mode", "Turn the dark style on or off.", enabled(), Safe, settings::set_dark_mode,
                  onoff("enabled", "dark mode")),
        Tool::new("set_do_not_disturb", "Turn Do Not Disturb (hide notification banners) on or off.", enabled(), Safe,
                  settings::set_do_not_disturb, onoff("enabled", "Do Not Disturb")),
        Tool::new("set_volume", "Set the output volume in percent (0-100).",
                  obj(json!({"percent": {"type": "integer", "minimum": 0, "maximum": 100}}), &["percent"]), Safe,
                  settings::set_volume, text("percent", |p| format!("Set volume to {p}%"))),
        Tool::new("set_mute", "Mute or unmute the sound output.", obj(json!({"muted": {"type": "boolean"}}), &["muted"]),
                  Safe, settings::set_mute, onoff("muted", "mute")),
        Tool::new("set_brightness", "Set the screen brightness in percent (1-100).",
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
        Tool::new("send_file", "Send a file to one of the user's devices with Taildrop (Tailscale).",
                  obj(json!({"path": {"type": "string"}, "device": {"type": "string", "description": "Device name"}}),
                      &["path", "device"]), Safe, connect::send_file, // Taildrop only reaches your own devices
                  |a| format!("Send {} to {}", base_name(&or(arg_str(a, "path"), "a file")), or(arg_str(a, "device"), "a device"))),
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
        Tool::new("list_bluetooth", "List paired Bluetooth devices (headphones, mice…) and which are connected.",
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
        Tool::new("read_window", "Read the text of the focused window (web page, document, email…) and what's selected \
in it. Use it when the user says this, here, or asks about what they're looking at.",
                  obj(json!({"app": {"type": "string", "description": "Only to pick a window other than the focused one"}}), &[]),
                  Safe, screen::read_window, text("app", |a| format!("Read {}", or(a, "the window")))),
        Tool::new("search_file_contents", "Search inside the user's files (documents, PDFs, notes) for words, \
using GNOME's file index. Returns files and matching snippets.", obj(json!({"query": {"type": "string"}}), &["query"]),
                  Safe, localsearch::search_file_contents, text("query", |q| format!("Search files for “{q}”"))),
        Tool::new("read_document", "Read the text of a PDF, office document or other file that read_file can't.", path(),
                  Safe, localsearch::read_document, text("path", |p| format!("Read {}", base_name(&or(p, "a document"))))),
        Tool::new("get_events", "Get calendar events. when: today, tomorrow, this week, next week, a weekday or a \
date like 2026-11-20.", obj(json!({"when": {"type": "string"}}), &[]), Safe, personal::get_events,
                  text("when", |w| format!("Check the calendar ({})", or(w, "today")))),
        Tool::new("find_contact", "Find a contact's email addresses and phone numbers by name.",
                  obj(json!({"name": {"type": "string"}}), &["name"]), Safe, personal::find_contact,
                  text("name", |n| format!("Look up {}", or(n, "a contact")))),
        Tool::new("media_control", "Control music and video players: play, pause, play_pause, next, previous, stop, \
or status (what's playing).", obj(json!({"action": {"type": "string", "enum": ["play", "pause", "play_pause", "next",
                  "previous", "stop", "status"]}, "player": {"type": "string", "description": "Optional, e.g. Spotify"}}), &["action"]),
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
        let peers: Vec<String> = ["jakubs-iphone", "framework", "ipad-mini"].map(String::from).to_vec();
        assert_eq!(pick("iphone (jakub)", &peers, "device").unwrap(), "jakubs-iphone");
        assert_eq!(pick("Jakub's iPhone", &peers, "device").unwrap(), "jakubs-iphone");
        let peers: Vec<String> = ["iphone-15", "framework"].map(String::from).to_vec();
        assert_eq!(pick("iphone (jakub)", &peers, "device").unwrap(), "iphone-15");
        let nets: Vec<String> = ["Home 5G", "Home 2.4"].map(String::from).to_vec();
        assert!(pick("home", &nets, "network").unwrap_err().0.contains("more than one"));
    }

    #[test]
    fn same_tools_and_risks_as_the_python_app() {
        let names: Vec<String> = all().into_iter().map(|t| t.name).collect();
        assert_eq!(names.len(), 54, "{names:?}");
        let risky: Vec<String> = all().into_iter().filter(|t| t.risk == Risk::Confirm).map(|t| t.name).collect();
        assert_eq!(risky, ["run_command"]);
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
