//! The compact tool set (Fable's review): about 28 broad tools instead of
//! 64 narrow ones, so every request can carry all of them and routing goes
//! away. Each broad tool forwards to the narrow one underneath with
//! translated arguments, so behaviour, pills, undo and the eval's stand-ins
//! stay the same; only what the model sees changes.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::tool::{Args, Risk, Tool, ToolError, arg_str, obj};

/// How a broad call becomes a narrow one: (narrow tool name, its arguments).
type Route = fn(&Args) -> Result<(String, Args), ToolError>;

fn args(v: Value) -> Args {
    match v {
        Value::Object(m) => m,
        _ => Args::new(),
    }
}

fn s(a: &Args, k: &str) -> Value {
    a.get(k).cloned().unwrap_or(Value::Null)
}

fn on(a: &Args) -> Value {
    match a.get("on") {
        Some(Value::Bool(b)) => Value::Bool(*b),
        Some(v) => crate::repair::fit(
            &json!({"properties": {"on": {"type": "boolean"}}}),
            args(json!({"on": v})),
        )
        .0
        .get("on")
        .cloned()
        .unwrap_or(Value::Bool(true)),
        None => Value::Bool(true),
    }
}

fn open_route(a: &Args) -> Result<(String, Args), ToolError> {
    let t = arg_str(a, "target");
    let t = t.trim();
    let looks_url = t.starts_with("http")
        || t.starts_with("www.")
        || (t.contains('.')
            && !t.contains(' ')
            && !t.contains('/')
            && t.rsplit('.').next().is_some_and(|tld| {
                (2..=6).contains(&tld.len())
                    && tld.chars().all(|c| c.is_ascii_alphabetic())
                    && ![
                        "pdf", "txt", "doc", "docx", "odt", "png", "jpg", "jpeg", "md", "csv",
                    ]
                    .contains(&tld)
            }));
    Ok(if looks_url {
        ("open_url".into(), args(json!({"url": t})))
    } else if t.starts_with('~')
        || t.starts_with('/')
        || t.contains('/')
        || t.rsplit_once('.')
            .is_some_and(|(_, e)| e.len() <= 5 && !e.contains(' '))
    {
        ("open_file".into(), args(json!({"path": t})))
    } else {
        ("open_app".into(), args(json!({"name": t})))
    })
}

fn set_route(a: &Args) -> Result<(String, Args), ToolError> {
    let setting = arg_str(a, "setting")
        .to_lowercase()
        .replace([' ', '-'], "_");
    let v = s(a, "value");
    let pick = |tool: &str, key: &str| Ok((tool.to_string(), args(json!({ key: v.clone() }))));
    match setting.as_str() {
        "dark_mode" | "dark" => pick("set_dark_mode", "enabled"),
        "night_light" => pick("set_night_light", "enabled"),
        "do_not_disturb" | "dnd" => pick("set_do_not_disturb", "enabled"),
        "wifi" | "wi_fi" => pick("set_wifi", "enabled"),
        "bluetooth" => pick("set_bluetooth", "enabled"),
        "mute" | "muted" => pick("set_mute", "muted"),
        "volume" => pick("set_volume", "percent"),
        "brightness" => pick("set_brightness", "percent"),
        "power_mode" | "power" => pick("set_power_mode", "mode"),
        _ => Err(format!(
            "unknown setting '{setting}'; one of: {}. For anything else use change_setting.",
            SETTINGS.join(", ")
        )
        .into()),
    }
}

const SETTINGS: [&str; 9] = [
    "dark_mode",
    "night_light",
    "do_not_disturb",
    "volume",
    "mute",
    "brightness",
    "power_mode",
    "wifi",
    "bluetooth",
];

fn send_route(a: &Args) -> Result<(String, Args), ToolError> {
    let to = s(a, "to");
    if !arg_str(a, "path").trim().is_empty() {
        Ok((
            "send_file".into(),
            args(json!({"path": s(a, "path"), "device": to})),
        ))
    } else {
        Ok((
            "send_text".into(),
            args(json!({"text": s(a, "text"), "device": to, "title": s(a, "title")})),
        ))
    }
}

fn find_route(a: &Args) -> Result<(String, Args), ToolError> {
    let q = s(a, "query");
    Ok(match arg_str(a, "kind").to_lowercase().as_str() {
        "inside_files" | "text" | "contents" => {
            ("search_file_contents".into(), args(json!({"query": q})))
        }
        "settings" | "setting" => ("find_settings".into(), args(json!({"query": q}))),
        "contacts" | "contact" | "people" => ("find_contact".into(), args(json!({"name": q}))),
        "devices" | "device" => ("list_devices".into(), Args::new()),
        "networks" | "network" | "wifi" => ("list_networks".into(), Args::new()),
        "bluetooth" => ("list_bluetooth".into(), Args::new()),
        "folder" => ("list_folder".into(), args(json!({"path": q}))),
        "reminders" => ("list_reminders".into(), Args::new()),
        _ => ("find_files".into(), args(json!({"name": q}))),
    })
}

fn read_route(a: &Args) -> Result<(String, Args), ToolError> {
    let q = arg_str(a, "question");
    Ok(if q.trim().is_empty() {
        ("read_document".into(), args(json!({"path": s(a, "path")})))
    } else {
        (
            "ask_document".into(),
            args(json!({"path": s(a, "path"), "question": q})),
        )
    })
}

fn screenshot_route(a: &Args) -> Result<(String, Args), ToolError> {
    let keep = matches!(a.get("keep"), Some(Value::Bool(true))) || arg_str(a, "keep") == "true";
    Ok((
        if keep {
            "save_screenshot"
        } else {
            "look_at_screen"
        }
        .into(),
        Args::new(),
    ))
}

fn web_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(if !arg_str(a, "url").trim().is_empty() {
        ("read_web_page".into(), args(json!({"url": s(a, "url")})))
    } else {
        ("web_search".into(), args(json!({"query": s(a, "query")})))
    })
}

fn memory_route(a: &Args) -> Result<(String, Args), ToolError> {
    let forget = arg_str(a, "action").to_lowercase().starts_with("forget");
    Ok((
        if forget { "forget" } else { "remember" }.into(),
        args(json!({"fact": s(a, "fact")})),
    ))
}

fn skill_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(if arg_str(a, "action").to_lowercase().starts_with("save") {
        (
            "save_skill".into(),
            args(
                json!({"name": s(a, "name"), "steps": s(a, "steps"), "description": s(a, "description")}),
            ),
        )
    } else {
        ("use_skill".into(), args(json!({"name": s(a, "name")})))
    })
}

fn reminder_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(match arg_str(a, "action").to_lowercase().as_str() {
        "list" => ("list_reminders".into(), Args::new()),
        "cancel" | "delete" | "remove" => (
            "cancel_reminder".into(),
            args(json!({"which": s(a, "which")})),
        ),
        _ => (
            "set_reminder".into(),
            args(json!({"text": s(a, "text"), "in_minutes": s(a, "in_minutes"), "at": s(a, "at")})),
        ),
    })
}

fn calendar_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(
        if arg_str(a, "action").to_lowercase().starts_with("add")
            || !arg_str(a, "title").trim().is_empty()
        {
            (
                "create_event".into(),
                args(
                    json!({"title": s(a, "title"), "start": s(a, "start"), "minutes": s(a, "minutes"),
                                          "end": s(a, "end"), "place": s(a, "place")}),
                ),
            )
        } else {
            ("get_events".into(), args(json!({"when": s(a, "when")})))
        },
    )
}

fn clipboard_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(
        if arg_str(a, "action").to_lowercase().starts_with("copy") && !arg_str(a, "text").is_empty()
        {
            (
                "copy_to_clipboard".into(),
                args(json!({"text": s(a, "text")})),
            )
        } else {
            (
                "clipboard_history".into(),
                args(json!({"query": s(a, "query")})),
            )
        },
    )
}

fn app_action_route(a: &Args) -> Result<(String, Args), ToolError> {
    Ok(if arg_str(a, "action").trim().is_empty() {
        ("list_app_actions".into(), args(json!({"app": s(a, "app")})))
    } else {
        (
            "run_app_action".into(),
            args(json!({"app": s(a, "app"), "action": s(a, "action")})),
        )
    })
}

fn connect_route(a: &Args) -> Result<(String, Args), ToolError> {
    let target = arg_str(a, "target");
    let connect = on(a) == Value::Bool(true);
    let t = target.to_lowercase();
    let network = [
        "wifi", "wi-fi", "network", "vpn", "sieć", "siec", "hotspot", "ethernet",
    ]
    .iter()
    .any(|w| t.contains(w));
    Ok(match (network, connect) {
        (true, true) => ("connect_network".into(), args(json!({"name": target}))),
        (true, false) => ("disconnect_network".into(), args(json!({"name": target}))),
        (false, true) => ("connect_bluetooth".into(), args(json!({"device": target}))),
        (false, false) => (
            "disconnect_bluetooth".into(),
            args(json!({"device": target})),
        ),
    })
}

/// Tools kept as they are.
const KEPT: [&str; 15] = [
    "research",
    "play_music",
    "get_status",
    "get_datetime",
    "undo",
    "change_setting",
    "open_settings",
    "look_at_image",
    "read_window",
    "draft_email",
    "rewrite_selection",
    "search_apps",
    "open_search_result",
    "open_terminal",
    "run_command",
];

/// The compact set over `base` (the narrow tools: real ones, or the eval's stand-ins).
pub fn compact(base: Vec<Tool>) -> Vec<Tool> {
    let base = Arc::new(base);
    let find = |name: &str| base.iter().find(|t| t.name == name).cloned();
    let broad = |name: &str, desc: &str, params: Value, route: Route| -> Tool {
        let (b1, b2, b3) = (base.clone(), base.clone(), base.clone());
        let run = move |ctx: &crate::tool::Context, a: &Args| {
            let (to, inner) = route(a)?;
            let tool = b1
                .iter()
                .find(|t| t.name == to)
                .ok_or_else(|| ToolError(format!("{to} isn't available")))?;
            let inner = crate::repair::fit(
                &tool.parameters,
                inner.into_iter().filter(|(_, v)| !v.is_null()).collect(),
            )
            .0;
            (tool.run)(ctx, &inner)
        };
        let summary = move |a: &Args| match route(a) {
            Ok((to, inner)) => b2
                .iter()
                .find(|t| t.name == to)
                .map(|t| (t.summary)(&inner))
                .unwrap_or(to),
            Err(_) => name_of(a),
        };
        let resolve = move |a: &Args| {
            let (to, inner) = route(a).ok()?;
            let tool = b3.iter().find(|t| t.name == to)?.clone();
            let inner = crate::repair::fit(
                &tool.parameters,
                inner.into_iter().filter(|(_, v)| !v.is_null()).collect(),
            )
            .0;
            Some((tool, inner))
        };
        let mut t = Tool::new(name, desc, params, Risk::Safe, run, summary);
        t.resolve = Some(Arc::new(resolve));
        t
    };
    fn name_of(_: &Args) -> String {
        "Do it".into()
    }
    let mut out = vec![
        broad(
            "open",
            "Open an app ('Firefox', or a kind: browser, email, files, music…), a web address, or a file path.",
            obj(json!({"target": {"type": "string"}}), &["target"]),
            open_route,
        ),
        broad(
            "set",
            "Change a common setting. setting: dark_mode, night_light, do_not_disturb, wifi, bluetooth, mute (value \
true/false); volume, brightness (value 0-100); power_mode (power-saver, balanced, performance). Other settings: change_setting.",
            obj(
                json!({"setting": {"type": "string", "enum": SETTINGS}, "value": {"type": "string"}}),
                &["setting", "value"],
            ),
            set_route,
        ),
        broad(
            "connect",
            "Connect (on true) or disconnect (on false) a paired Bluetooth device (headphones, speaker, mouse) or a \
saved network (Wi-Fi, VPN), by everyday name.",
            obj(
                json!({"target": {"type": "string"}, "on": {"type": "boolean"}}),
                &["target"],
            ),
            connect_route,
        ),
        broad(
            "send",
            "Send to one of the user's devices with Taildrop: a file (path) or a note (text).",
            obj(
                json!({"to": {"type": "string", "description": "Device name"}, "path": {"type": "string"}, "text": {"type": "string"}}),
                &["to"],
            ),
            send_route,
        ),
        broad(
            "find",
            "Look something up. kind: files (by name; default), inside_files (by what's in them), settings, contacts, \
devices (Taildrop), networks, bluetooth, folder (list one), reminders.",
            obj(
                json!({"kind": {"type": "string", "enum": ["files", "inside_files", "settings", "contacts", "devices", "networks",
                         "bluetooth", "folder", "reminders"]}, "query": {"type": "string"}}),
                &["kind"],
            ),
            find_route,
        ),
        broad(
            "read",
            "Read a file or document; with question, answer what it says or summarize it (works on long PDFs).",
            obj(
                json!({"path": {"type": "string"}, "question": {"type": "string"}}),
                &["path"],
            ),
            read_route,
        ),
        broad(
            "screenshot",
            "Take a screenshot: keep true saves it as a file (to send or keep, returns the path); keep false \
lets you look at the screen.",
            obj(json!({"keep": {"type": "boolean"}}), &[]),
            screenshot_route,
        ),
        broad(
            "web",
            "Search the web (query) or read a web page (url). Not for the user's own files, settings or devices.",
            obj(
                json!({"query": {"type": "string"}, "url": {"type": "string"}}),
                &[],
            ),
            web_route,
        ),
        broad(
            "memory",
            "Remember a lasting fact about the user (action remember), or forget one (action forget).",
            obj(
                json!({"action": {"type": "string", "enum": ["remember", "forget"]}, "fact": {"type": "string"}}),
                &["fact"],
            ),
            memory_route,
        ),
        broad(
            "skill",
            "Use one of the user's saved skills (action use), or save a workflow as one (action save, with steps).",
            obj(
                json!({"action": {"type": "string", "enum": ["use", "save"]}, "name": {"type": "string"}, "steps": {"type": "string"},
                         "description": {"type": "string"}}),
                &["name"],
            ),
            skill_route,
        ),
        broad(
            "reminder",
            "Reminders and timers. action set (text, and in_minutes or at as HH:MM / YYYY-MM-DD HH:MM; a timer \
has no text), list, or cancel (which).",
            obj(
                json!({"action": {"type": "string", "enum": ["set", "list", "cancel"]}, "text": {"type": "string"},
                         "in_minutes": {"type": "number"}, "at": {"type": "string"}, "which": {"type": "string"}}),
                &[],
            ),
            reminder_route,
        ),
        broad(
            "calendar",
            "The user's calendar: action list (when: today, tomorrow, a weekday, this week…) or add (title, start as \
HH:MM / a weekday and time / YYYY-MM-DD HH:MM, minutes, place).",
            obj(
                json!({"action": {"type": "string", "enum": ["list", "add"]}, "when": {"type": "string"}, "title": {"type": "string"},
                         "start": {"type": "string"}, "minutes": {"type": "number"}, "place": {"type": "string"}}),
                &[],
            ),
            calendar_route,
        ),
        broad(
            "clipboard",
            "Copy text to the clipboard (action copy, text), or see what the user copied recently (action history, query).",
            obj(
                json!({"action": {"type": "string", "enum": ["copy", "history"]}, "text": {"type": "string"}, "query": {"type": "string"}}),
                &[],
            ),
            clipboard_route,
        ),
        broad(
            "media",
            "Music and video players: action play, pause, play_pause, next_track, previous_track, stop, status.",
            obj(
                json!({"action": {"type": "string", "enum": ["play", "pause", "play_pause", "next_track", "previous_track", "stop", "status"]},
                         "player": {"type": "string"}}),
                &["action"],
            ),
            |a| Ok(("media_control".into(), a.clone())),
        ),
        broad(
            "app_action",
            "An app's own actions (New Private Window, Compose…): without action, lists them; with action, runs it.",
            obj(
                json!({"app": {"type": "string"}, "action": {"type": "string"}}),
                &["app"],
            ),
            app_action_route,
        ),
    ];
    out.extend(KEPT.iter().filter_map(|n| find(n)));
    // Other apps' tools (MCP) come along as they are.
    out.extend(base.iter().filter(|t| t.name.contains("__")).cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: Value) -> Args {
        args(v)
    }

    #[test]
    fn broad_calls_become_narrow_ones() {
        assert_eq!(
            set_route(&a(json!({"setting": "dark_mode", "value": "true"})))
                .unwrap()
                .0,
            "set_dark_mode"
        );
        assert_eq!(
            set_route(&a(json!({"setting": "volume", "value": "30"})))
                .unwrap()
                .1["percent"],
            "30"
        );
        assert_eq!(
            open_route(&a(json!({"target": "example.com"}))).unwrap().0,
            "open_url"
        );
        assert_eq!(
            open_route(&a(json!({"target": "~/Dokumenty/lease.pdf"})))
                .unwrap()
                .0,
            "open_file"
        );
        assert_eq!(
            open_route(&a(json!({"target": "Firefox"}))).unwrap().0,
            "open_app"
        );
        assert_eq!(
            connect_route(&a(json!({"target": "my airpods"})))
                .unwrap()
                .0,
            "connect_bluetooth"
        );
        assert_eq!(
            connect_route(&a(json!({"target": "office wifi", "on": false})))
                .unwrap()
                .0,
            "disconnect_network"
        );
        assert_eq!(
            send_route(&a(json!({"to": "phone", "path": "~/a.png"})))
                .unwrap()
                .0,
            "send_file"
        );
        assert_eq!(
            send_route(&a(json!({"to": "phone", "text": "milk"})))
                .unwrap()
                .0,
            "send_text"
        );
        assert_eq!(
            find_route(&a(json!({"kind": "settings", "query": "tap"})))
                .unwrap()
                .0,
            "find_settings"
        );
        assert_eq!(
            read_route(&a(json!({"path": "x.pdf", "question": "deposit?"})))
                .unwrap()
                .0,
            "ask_document"
        );
        assert_eq!(
            calendar_route(&a(
                json!({"action": "add", "title": "Lunch", "start": "friday 13:00"})
            ))
            .unwrap()
            .0,
            "create_event"
        );
        assert_eq!(
            reminder_route(&a(json!({"in_minutes": 5}))).unwrap().0,
            "set_reminder"
        );
    }

    #[test]
    fn the_set_is_small_and_complete() {
        let all = crate::tools::all();
        let c = compact(all.clone());
        assert!(c.len() <= 30, "{}", c.len());
        let tokens =
            crate::agent::estimate_tokens(&Value::Array(c.iter().map(|t| t.schema()).collect()));
        let full =
            crate::agent::estimate_tokens(&Value::Array(all.iter().map(|t| t.schema()).collect()));
        eprintln!(
            "compact: {} tools, {tokens} tokens (all {} tools: {full})",
            c.len(),
            all.len()
        );
        assert!(tokens * 100 < full * 55, "about half of all tools");
        // Every narrow tool a broad one names exists.
        let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        for n in KEPT {
            assert!(names.contains(&n), "{n}");
        }
    }
}
