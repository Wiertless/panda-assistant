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
pub const GROUPS: [(&str, &str, &[&str]); 12] = [
    (
        "reminders",
        "listing and cancelling reminders and timers",
        &[
            "remind",
            "reminder",
            "timer",
            "alarm",
            "przypomn",
            "przypomin",
            "minutnik",
            "budzik",
        ],
    ),
    (
        "clipboard",
        "what the user copied recently, and rewriting the selected text (proofread, shorten, translate)",
        &[
            "copied",
            "copy",
            "clipboard",
            "paste",
            "pasted",
            "skopiow",
            "schowek",
            "schowk",
            "wklej",
            "selected",
            "selection",
            "highlighted",
            "proofread",
            "rewrite",
            "rephrase",
            "shorten",
            "shorter",
            "translate",
            "formal",
            "spelling",
            "grammar",
            "zaznacz",
            "popraw",
            "przepisz",
            "skróć",
            "przetłumacz",
            "przetlumacz",
            "ortograf",
            "literówk",
        ],
    ),
    (
        "settings",
        "dark mode, Night Light, volume, brightness, power mode, Do Not Disturb, Wi-Fi/Bluetooth on or off, any GNOME setting",
        &[
            "dark",
            "light",
            "night",
            "volume",
            "sound",
            "mute",
            "loud",
            "quiet",
            "bright",
            "dim",
            "power",
            "battery saver",
            "performance",
            "disturb",
            "notification",
            "setting",
            "font",
            "text size",
            "bigger",
            "smaller",
            "scal",
            "wallpaper",
            "background",
            "theme",
            "wifi",
            "wi-fi",
            "bluetooth",
            "keyboard",
            "mouse",
            "touchpad",
            "display",
            "screen",
            "ciemn",
            "jasn",
            "głoś",
            "glos",
            "cisz",
            "wycisz",
            "ścisz",
            "pogłoś",
            "podgłoś",
            "rozjaśn",
            "przyciem",
            "ściemn",
            "przyciszy",
            "zgłoś",
            "louder",
            "quieter",
            "turn down",
            "turn up",
            "jasność",
            "jasnosc",
            "ustawien",
            "tryb",
            "czcionk",
            "tapet",
            "ekran",
            "dźwięk",
            "dzwiek",
        ],
    ),
    (
        "files",
        "list folders, find files by name or by what's inside, read text files, PDFs and documents, look at images",
        &[
            "file",
            "folder",
            "director",
            "document",
            "pdf",
            "download",
            "picture",
            "photo",
            "image",
            "read ",
            "find",
            "where is",
            "lease",
            "contract",
            "invoice",
            "receipt",
            "resume",
            "cv",
            "letter",
            "report",
            "statement",
            "ticket",
            "manual",
            "umow",
            "umów",
            "faktur",
            "rachun",
            "paragon",
            "raport",
            "bilet",
            "instrukcj",
            "plik",
            "folder",
            "katalog",
            "dokument",
            "pobran",
            "zdję",
            "zdje",
            "obraz",
            "notat",
            "przeczytaj",
            "znajdź",
            "znajdz",
            "gdzie",
        ],
    ),
    (
        "system",
        "battery level, memory, disk space, uptime, OS version",
        &[
            "battery", "memory", "ram", "disk", "storage", "space", "uptime", "kernel", "version",
            "cpu", "bateri", "pamię", "pamie", "dysk", "miejsc", "wersj", "procesor",
        ],
    ),
    (
        "devices",
        "saved Wi-Fi networks and VPNs, paired Bluetooth devices (headphones…), sending files and notes to other devices with Taildrop",
        &[
            "connect",
            "network",
            "vpn",
            "wifi",
            "wi-fi",
            "bluetooth",
            "headphone",
            "airpod",
            "earbud",
            "speaker",
            "mouse",
            "pair",
            "send",
            "taildrop",
            "tailscale",
            "phone",
            "device",
            "połącz",
            "polacz",
            "rozłącz",
            "rozlacz",
            "sieć",
            "siec",
            "słuchawk",
            "sluchawk",
            "głośnik",
            "glosnik",
            "wyślij",
            "wyslij",
            "telefon",
            "urządz",
            "urzadz",
        ],
    ),
    (
        "personal",
        "calendar events (reading and adding them), contacts, and email drafts",
        &[
            "calendar",
            "event",
            "meeting",
            "appointment",
            "schedule",
            "today",
            "tomorrow",
            "week",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "contact",
            "email",
            "e-mail",
            "mail",
            "write to",
            "tell ",
            "add ",
            "lunch",
            "dinner",
            "dentist",
            "doctor",
            "napisz",
            "dodaj",
            "spotkanie",
            "obiad",
            "kolacj",
            "lekarz",
            "email address",
            "phone number",
            "birthday",
            "number",
            "email",
            "address",
            "sister",
            "brother",
            "mom",
            "mum",
            "dad",
            "wife",
            "husband",
            "girlfriend",
            "boyfriend",
            "friend",
            "colleague",
            "boss",
            "kalendarz",
            "spotkani",
            "wydarzen",
            "dziś",
            "dzis",
            "dzisiaj",
            "jutro",
            "tydzień",
            "tydzien",
            "poniedział",
            "wtor",
            "środ",
            "czwart",
            "piąt",
            "sobot",
            "niedziel",
            "kontakt",
            "numer",
            "urodzin",
            "siostr",
            "brat",
            "mam",
            "tat",
            "żon",
            "mąż",
            "męża",
            "dziewczyn",
            "chłopak",
            "przyjaci",
            "szef",
            "adres",
            "mail",
        ],
    ),
    (
        "apps",
        "playing songs from the user's library, searching inside apps (sums, emoji, weather, contacts…), app actions, opening a terminal, music and video playback",
        &[
            "app",
            "terminal",
            "konsol",
            "emoji",
            "symbol",
            "convert",
            "przelicz",
            "song",
            "album",
            "artist",
            "listen",
            "gapless",
            "put on",
            "play some",
            "puść coś",
            "pusc cos",
            "zagraj coś",
            "zagraj cos",
            "rhythmbox",
            "utwór",
            "piosenk",
            "zagraj",
            "puść",
            "pusc",
            "posłucha",
            "płyt",
            "action",
            "private window",
            "new window",
            "compose",
            "music",
            "song",
            "play",
            "pause",
            "skip",
            "next",
            "previous",
            "track",
            "spotify",
            "video",
            "calculat",
            "weather",
            "search in",
            "emoji",
            "character",
            "aplikacj",
            "muzyk",
            "piosenk",
            "utwór",
            "utwor",
            "odtwórz",
            "odtworz",
            "zatrzymaj",
            "następn",
            "nastepn",
            "pogod",
            "kalkulator",
            "policz",
        ],
    ),
    (
        "web",
        "reading web pages, researching a question in depth",
        &[
            "http",
            "www.",
            ".com",
            "website",
            "page",
            "link",
            "article",
            "strona",
            "stron",
            "artykuł",
            "artykul",
            "research",
            "look into",
            "find out",
            "compare",
            "comparison",
            "in depth",
            "sources",
            "review",
            "zbadaj",
            "zbada",
            "dowiedz",
            "porówn",
            "porown",
            "poszukaj informacji",
            "recenzj",
            "źródł",
            "zrodl",
        ],
    ),
    (
        "memory",
        "forgetting remembered facts, saving workflows as skills",
        &[
            "forget",
            "skill",
            "workflow",
            "routine",
            "save that",
            "remember",
            "note that",
            "call me",
            "from now on",
            "my name is",
            "zapomnij",
            "umiejętn",
            "umiejetn",
            "zapisz",
            "zapamiętaj",
            "pamiętaj",
            "mów do mnie",
            "od teraz",
            "nazywam się",
        ],
    ),
    (
        "commands",
        "running shell commands in the developer toolbox (build, compile, git, scripts)",
        &[
            "command",
            "terminal",
            "shell",
            "build",
            "compile",
            "make",
            "cargo",
            "git ",
            "npm",
            "script",
            "run ",
            "install",
            "polecen",
            "terminal",
            "skompiluj",
            "zbuduj",
            "uruchom",
            "skrypt",
        ],
    ),
    (
        "connected",
        "tools from other apps and services connected over MCP",
        &[],
    ),
];

/// One worked example per group, shown while the group is active: small models
/// copy an example far more reliably than they follow a rule.
pub const EXAMPLES: [(&str, &str); 12] = [
    (
        "reminders",
        "\"cancel the laundry reminder\" → cancel_reminder(which \"laundry\").",
    ),
    (
        CORE,
        "\"grab the screen and put it on my tablet\" → save_screenshot, then send_file with the path it returned \
and the tablet's name (from what you remember, or list_devices).",
    ),
    (
        "settings",
        "\"turn off the animations\" → find_settings(query \"animations\"), then change_setting with the schema \
and key it found and the new value.",
    ),
    (
        "files",
        "\"what's the total on my last invoice?\" → find_files(name \"invoice\"), then ask_document with the newest \
path and the question.",
    ),
    (
        "system",
        "\"is my disk getting full?\" → get_system_info, then answer with the numbers.",
    ),
    (
        "devices",
        "\"connect the speaker\" → connect_bluetooth(device \"speaker\"); if the error lists the paired devices, \
call it again with the one that is a speaker.",
    ),
    (
        "personal",
        "\"anything on Friday afternoon?\" → get_events(when \"friday\"), then answer from the events.",
    ),
    (
        "apps",
        "\"open thunderbrd\" → open_app(name \"thunderbrd\"); the error lists close names, so call it again with \
\"Thunderbird\".",
    ),
    (
        "web",
        "\"who won yesterday's match?\" → web_search, then read_web_page on the best result if the snippets don't say. \
\"research which e-reader is best for PDFs\" → research(question \"which e-reader is best for reading PDFs?\").",
    ),
    (
        "memory",
        "\"forget my old address\" → forget(fact \"address\").",
    ),
    (
        "commands",
        "\"build my project\" → run_command with the build command, in the project's folder.",
    ),
    (
        "clipboard",
        "\"make the selected text more polite\" → rewrite_selection(instruction \"make it more polite\").",
    ),
];

pub fn example_for(group: &str) -> Option<&'static str> {
    EXAMPLES.iter().find(|(g, _)| *g == group).map(|(_, e)| *e)
}

/// Routing by meaning: the message's embedding against each group's, for
/// wordings the keyword lists miss. Off (keywords only) when the server has
/// no embedding model.
pub struct Embedder {
    client: std::sync::Arc<crate::flm::FlmClient>,
    model: String,
    /// The groups' vectors once asked for (None when embeddings weren't there,
    /// asked again after a while: the server may restart with them).
    groups: std::sync::Mutex<Option<(std::time::Instant, Option<Vectors>)>>,
}

type Vectors = Vec<Vec<f32>>;

const RETRY_EMBEDDINGS: std::time::Duration = std::time::Duration::from_secs(600);

/// Closest group must be at least this similar, and others this close to it.
const MIN_SIMILARITY: f32 = 0.3;
const NEAR_BEST: f32 = 0.04;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    dot / (norm(a) * norm(b))
}

/// The groups worth adding for these similarities (in GROUPS order): the best,
/// and any others nearly as close, at most two.
pub fn nearest(sims: &[f32]) -> Vec<&'static str> {
    let best = sims.iter().cloned().fold(f32::MIN, f32::max);
    if best < MIN_SIMILARITY {
        return vec![];
    }
    let mut picked: Vec<(f32, &'static str)> = sims
        .iter()
        .zip(GROUPS.iter())
        .filter(|(s, _)| **s >= best - NEAR_BEST)
        .map(|(s, (g, _, _))| (*s, *g))
        .collect();
    picked.sort_by(|a, b| b.0.total_cmp(&a.0));
    picked.into_iter().take(2).map(|(_, g)| g).collect()
}

impl Embedder {
    pub fn new(client: std::sync::Arc<crate::flm::FlmClient>, model: &str) -> Self {
        Self {
            client,
            model: model.into(),
            groups: std::sync::Mutex::new(None),
        }
    }

    pub fn groups_for(&self, text: &str) -> Vec<&'static str> {
        let mut cache = self.groups.lock().unwrap();
        let stale = match cache.as_ref() {
            None => true,
            Some((when, None)) => when.elapsed() > RETRY_EMBEDDINGS,
            Some(_) => false,
        };
        if stale {
            let docs: Vec<String> = GROUPS
                .iter()
                .map(|(g, d, _)| format!("{d}. For example: {}", example_for(g).unwrap_or("")))
                .collect();
            *cache = Some((
                std::time::Instant::now(),
                self.client.embed(&self.model, &docs, "document").ok(),
            ));
        }
        let Some((_, Some(groups))) = cache.as_ref() else {
            return vec![];
        };
        let Ok(q) = self.client.embed(&self.model, &[text.to_string()], "query") else {
            return vec![];
        };
        nearest(&groups.iter().map(|g| cosine(&q[0], g)).collect::<Vec<_>>())
    }
}

pub fn group_of(tool: &str) -> &'static str {
    match tool {
        "set_dark_mode" | "set_do_not_disturb" | "set_volume" | "set_mute" | "set_brightness"
        | "set_power_mode" | "set_night_light" | "get_setting" | "change_setting"
        | "open_settings" | "set_wifi" | "set_bluetooth" => "settings",
        "clipboard_history" | "rewrite_selection" => "clipboard",
        "list_reminders" | "cancel_reminder" => "reminders",
        "list_folder"
        | "read_file"
        | "read_document"
        | "ask_document"
        | "search_file_contents"
        | "look_at_image" => "files",
        "get_battery" | "get_system_info" => "system",
        "list_networks"
        | "connect_network"
        | "disconnect_network"
        | "list_bluetooth"
        | "connect_bluetooth"
        | "disconnect_bluetooth"
        | "list_devices"
        | "send_file"
        | "send_text" => "devices",
        "get_events" | "find_contact" | "create_event" | "draft_email" => "personal",
        "list_app_actions" | "run_app_action" | "media_control" | "play_music" | "search_apps"
        | "open_search_result" | "open_terminal" => "apps",
        "read_web_page" | "research" => "web",
        "forget" | "save_skill" => "memory",
        "run_command" => "commands",
        t if t.contains("__") => "connected",
        // Core: open apps/links/files, show in Files, time, status, remember, use a skill, web search, the
        // screen, and the two lookups no keyword list can cover ("my lease", "tap to click").
        _ => CORE,
    }
}

/// The group a core lookup leads into: once it ran, its follow-up tools
/// (read_document after find_files, change_setting after find_settings) come along.
pub fn leads_to(tool: &str) -> Option<&'static str> {
    match tool {
        "find_files" => Some("files"),
        "find_settings" => Some("settings"),
        _ => None,
    }
}

/// Whether a word stem starts a word in `t` ("smaller" in "a smaller one",
/// not "note" inside "denoted"); stems that start with a symbol (".com") match anywhere.
fn has_stem(t: &str, stem: &str) -> bool {
    if !stem.starts_with(char::is_alphanumeric) {
        return t.contains(stem);
    }
    t.match_indices(stem).any(|(i, _)| {
        t[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Groups the words of a message point at.
pub fn groups_for(text: &str) -> Vec<&'static str> {
    let t = text.to_lowercase();
    GROUPS
        .iter()
        .filter(|(_, _, words)| words.iter().any(|w| has_stem(&t, w.trim())))
        .map(|(g, _, _)| *g)
        .collect()
}

/// The meta tool listing the groups not yet included.
pub fn more_tools_schema(missing: &[&str]) -> Value {
    let list: Vec<String> = GROUPS
        .iter()
        .filter(|(g, _, _)| missing.contains(g))
        .map(|(g, d, _)| format!("{g}: {d}"))
        .collect();
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
        assert!(groups_for("a denoted fact").is_empty(), "stems start words");
        assert_eq!(groups_for("go to example.com"), ["web"]);
        assert!(groups_for("find the PDF about the lease").contains(&"files"));
        assert!(groups_for("how big is the deposit in my lease?").contains(&"files"));
        assert!(groups_for("mów do mnie Kuba").contains(&"memory"));
        assert!(
            groups_for("ścisz trochę").contains(&"settings"),
            "Polish verbs with prefixes"
        );
        assert!(groups_for("give me Ola's number").contains(&"personal"));
        assert!(groups_for("what's my sister's phone?").contains(&"personal"));
        assert!(groups_for("list my devices").iter().all(|g| *g != "files"));
        assert!(groups_for("open a terminal in Projects").contains(&"apps"));
        assert!(groups_for("what's the weather in Kraków").contains(&"apps"));
    }

    #[test]
    fn examples_and_nearest_groups() {
        for (g, _, _) in GROUPS.iter().filter(|(g, _, _)| *g != "connected") {
            // MCP tools vary
            assert!(example_for(g).is_some(), "{g} has no example");
        }
        assert!(example_for(CORE).is_some());
        let mut sims = vec![0.1; GROUPS.len()];
        assert!(nearest(&sims).is_empty(), "nothing close enough");
        sims[2] = 0.5;
        sims[5] = 0.48;
        sims[7] = 0.47;
        assert_eq!(nearest(&sims), [GROUPS[2].0, GROUPS[5].0]);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn every_tool_has_a_known_group() {
        let names: Vec<&str> = GROUPS.iter().map(|(g, _, _)| *g).collect();
        for t in crate::tools::all() {
            let g = group_of(&t.name);
            assert!(g == CORE || names.contains(&g), "{} -> {g}", t.name);
        }
        let core = crate::tools::all()
            .into_iter()
            .filter(|t| group_of(&t.name) == CORE)
            .count();
        assert!(core <= 18, "core grew to {core} tools");
        for t in ["find_files", "find_settings"] {
            assert_eq!(group_of(t), CORE);
            assert!(leads_to(t).is_some_and(|g| names.contains(&g)));
        }
    }
}

#[cfg(test)]
mod cost {
    #[test]
    fn report_token_cost() {
        let all = crate::tools::all();
        let tokens = |ts: Vec<serde_json::Value>| {
            crate::agent::estimate_tokens(&serde_json::Value::Array(ts))
        };
        let full = tokens(all.iter().map(|t| t.schema()).collect());
        let mut core: Vec<serde_json::Value> = all
            .iter()
            .filter(|t| super::group_of(&t.name) == super::CORE)
            .map(|t| t.schema())
            .collect();
        let missing: Vec<&str> = super::GROUPS.iter().map(|(g, _, _)| *g).collect();
        core.push(super::more_tools_schema(&missing));
        let routed = tokens(core);
        eprintln!("tool schemas: all {full} tokens, core + more_tools {routed} tokens");
        assert!(routed * 2 < full);
    }
}
