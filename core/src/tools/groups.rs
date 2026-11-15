//! Tool groups, so a request only carries the tools it may need.
//!
//! Every tool's schema costs context tokens on every message (all of them
//! together take about 3.5K of an 8K context). So a small core set always
//! goes along, and the other groups join when the message mentions them (in
//! English or Polish) or when the model asks for them with `more_tools`. A
//! group that joined stays for the rest of the chat.

use serde_json::{Value, json};

pub const CORE: &str = "core";
pub const MORE_TOOLS: &str = "more_tools";

/// (group, what it's for, word stems that bring it in).
pub const GROUPS: [(&str, &str, &[&str]); 11] = [
    ("clipboard", "what the user copied recently",
     &["copied", "copy", "clipboard", "paste", "pasted", "skopiow", "schowek", "schowk", "wklej"]),
    ("settings", "dark mode, Night Light, volume, brightness, power mode, Do Not Disturb, Wi-Fi/Bluetooth on or off, any GNOME setting",
     &["dark", "light", "night", "volume", "sound", "mute", "loud", "quiet", "bright", "dim", "power", "battery saver", "performance",
       "disturb", "notification", "setting", "font", "text size", "bigger", "smaller", "scal", "wallpaper", "background", "theme",
       "wifi", "wi-fi", "bluetooth", "keyboard", "mouse", "touchpad", "display", "screen",
       "ciemn", "jasn", "głoś", "glos", "cisz", "wycisz", "jasność", "jasnosc", "ustawien", "tryb", "czcionk", "tapet", "ekran", "dźwięk", "dzwiek"]),
    ("files", "list folders, find files by name or by what's inside, read text files, PDFs and documents, look at images",
     &["file", "folder", "director", "document", "pdf", "download", "picture", "photo", "image", "note", "read ", "find", "where is",
       "plik", "folder", "katalog", "dokument", "pobran", "zdję", "zdje", "obraz", "notat", "przeczytaj", "znajdź", "znajdz", "gdzie"]),
    ("system", "battery level, memory, disk space, uptime, OS version",
     &["battery", "memory", "ram", "disk", "storage", "space", "uptime", "kernel", "version", "cpu",
       "bateri", "pamię", "pamie", "dysk", "miejsc", "wersj", "procesor"]),
    ("devices", "saved Wi-Fi networks and VPNs, paired Bluetooth devices (headphones…), sending files and notes to other devices with Taildrop",
     &["connect", "network", "vpn", "wifi", "wi-fi", "bluetooth", "headphone", "airpod", "earbud", "speaker", "mouse", "pair",
       "send", "taildrop", "tailscale", "phone", "device",
       "połącz", "polacz", "rozłącz", "rozlacz", "sieć", "siec", "słuchawk", "sluchawk", "głośnik", "glosnik", "wyślij", "wyslij", "telefon", "urządz", "urzadz"]),
    ("personal", "calendar events and contacts from the user's accounts",
     &["calendar", "event", "meeting", "appointment", "schedule", "today", "tomorrow", "week", "monday", "tuesday", "wednesday",
       "thursday", "friday", "saturday", "sunday", "contact", "email address", "phone number", "birthday",
       "kalendarz", "spotkani", "wydarzen", "dziś", "dzis", "dzisiaj", "jutro", "tydzień", "tydzien", "poniedział", "wtor", "środ",
       "czwart", "piąt", "sobot", "niedziel", "kontakt", "numer", "urodzin"]),
    ("apps", "searching inside apps (sums, emoji, weather, contacts…), app actions, opening a terminal, music and video playback",
     &["app", "terminal", "konsol", "emoji", "symbol", "convert", "przelicz", "action", "private window", "new window", "compose", "music", "song", "play", "pause", "skip", "next", "previous",
       "track", "spotify", "video", "calculat", "weather", "search in", "emoji", "character",
       "aplikacj", "muzyk", "piosenk", "utwór", "utwor", "odtwórz", "odtworz", "zatrzymaj", "następn", "nastepn", "pogod", "kalkulator", "policz"]),
    ("web", "reading web pages", &["http", "www.", ".com", "website", "page", "link", "article", "strona", "stron", "artykuł", "artykul"]),
    ("memory", "forgetting remembered facts, saving workflows as skills",
     &["forget", "skill", "workflow", "routine", "save that", "zapomnij", "umiejętn", "umiejetn", "zapisz"]),
    ("commands", "running shell commands in the developer toolbox (build, compile, git, scripts)",
     &["command", "terminal", "shell", "build", "compile", "make", "cargo", "git ", "npm", "script", "run ", "install",
       "polecen", "terminal", "skompiluj", "zbuduj", "uruchom", "skrypt"]),
    ("connected", "tools from other apps and services connected over MCP", &[]),
];

pub fn group_of(tool: &str) -> &'static str {
    match tool {
        "set_dark_mode" | "set_do_not_disturb" | "set_volume" | "set_mute" | "set_brightness" | "set_power_mode"
        | "set_night_light" | "find_settings" | "get_setting" | "change_setting" | "open_settings" | "set_wifi"
        | "set_bluetooth" => "settings",
        "clipboard_history" => "clipboard",
        "list_folder" | "find_files" | "read_file" | "read_document" | "search_file_contents" | "look_at_image" => "files",
        "get_battery" | "get_system_info" => "system",
        "list_networks" | "connect_network" | "disconnect_network" | "list_bluetooth" | "connect_bluetooth"
        | "disconnect_bluetooth" | "list_devices" | "send_file" | "send_text" => "devices",
        "get_events" | "find_contact" => "personal",
        "list_app_actions" | "run_app_action" | "media_control" | "search_apps" | "open_search_result" | "open_terminal" => "apps",
        "read_web_page" => "web",
        "forget" | "save_skill" => "memory",
        "run_command" => "commands",
        t if t.contains("__") => "connected",
        _ => CORE, // open apps/links/files, show in Files, time, status, remember, use a skill, web search, the screen
    }
}

/// Groups the words of a message point at.
pub fn groups_for(text: &str) -> Vec<&'static str> {
    let t = format!(" {} ", text.to_lowercase());
    GROUPS.iter().filter(|(_, _, words)| words.iter().any(|w| t.contains(w))).map(|(g, _, _)| *g).collect()
}

/// The meta tool listing the groups not yet included.
pub fn more_tools_schema(missing: &[&str]) -> Value {
    let list: Vec<String> = GROUPS.iter().filter(|(g, _, _)| missing.contains(g)).map(|(g, d, _)| format!("{g}: {d}")).collect();
    json!({"type": "function", "function": {
        "name": MORE_TOOLS,
        "description": format!("Get more tools when none of yours fits the request. Groups: {}", list.join("; ")),
        "parameters": {"type": "object", "properties": {
            "groups": {"type": "array", "items": {"type": "string", "enum": missing}, "description": "Which groups"}},
            "required": ["groups"]}}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_by_words() {
        assert_eq!(groups_for("turn on dark mode"), ["settings"]);
        assert_eq!(groups_for("connect my airpods"), ["devices"]);
        assert_eq!(groups_for("co mam jutro w kalendarzu?"), ["personal"]);
        assert_eq!(groups_for("włącz tryb ciemny"), ["settings"]);
        assert!(groups_for("hello there").is_empty());
        assert!(groups_for("find the PDF about the lease").contains(&"files"));
        assert!(groups_for("open a terminal in Projects").contains(&"apps"));
        assert!(groups_for("what's the weather in Kraków").contains(&"apps"));
    }

    #[test]
    fn every_tool_has_a_known_group() {
        let names: Vec<&str> = GROUPS.iter().map(|(g, _, _)| *g).collect();
        for t in crate::tools::all() {
            let g = group_of(&t.name);
            assert!(g == CORE || names.contains(&g), "{} -> {g}", t.name);
        }
        let core = crate::tools::all().into_iter().filter(|t| group_of(&t.name) == CORE).count();
        assert!(core <= 14, "core grew to {core} tools");
    }
}

#[cfg(test)]
mod cost {
    #[test]
    fn report_token_cost() {
        let all = crate::tools::all();
        let tokens = |ts: Vec<serde_json::Value>| crate::agent::estimate_tokens(&serde_json::Value::Array(ts));
        let full = tokens(all.iter().map(|t| t.schema()).collect());
        let mut core: Vec<serde_json::Value> = all.iter().filter(|t| super::group_of(&t.name) == super::CORE).map(|t| t.schema()).collect();
        let missing: Vec<&str> = super::GROUPS.iter().map(|(g, _, _)| *g).collect();
        core.push(super::more_tools_schema(&missing));
        let routed = tokens(core);
        eprintln!("tool schemas: all {full} tokens, core + more_tools {routed} tokens");
        assert!(routed * 2 < full);
    }
}
