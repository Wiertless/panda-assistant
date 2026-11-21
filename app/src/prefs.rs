//! Preferences (General, Memory, Skills, Connections, Activity) and Keyboard Shortcuts.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};

use assistant_core::flm::pick_smarter_model;
use assistant_core::{activity, mcp, store};

use crate::config;
use crate::window::Window;

const CTX_CHOICES: [(i64, &str); 4] = [
    (4096, "4K tokens"),
    (8192, "8K tokens"),
    (16384, "16K tokens"),
    (32768, "32K tokens"),
];
const IDLE_CHOICES: [(i64, &str); 5] = [
    (1, "After 1 Minute"),
    (5, "After 5 Minutes"),
    (15, "After 15 Minutes"),
    (30, "After 30 Minutes"),
    (0, "Never"),
];

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn flat_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .valign(gtk::Align::Center)
        .build();
    b.add_css_class("flat");
    b
}

/// A group whose rows are rebuilt by `fill`.
struct Rows {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
}

impl Rows {
    fn new(group: adw::PreferencesGroup) -> Rc<Self> {
        Rc::new(Self {
            group,
            rows: RefCell::default(),
        })
    }
    fn clear(&self) {
        for r in self.rows.borrow_mut().drain(..) {
            self.group.remove(&r);
        }
    }
    fn add(&self, row: &impl IsA<gtk::Widget>) {
        self.group.add(row);
        self.rows.borrow_mut().push(row.clone().upcast());
    }
}

pub fn show_preferences(win: &Rc<Window>) {
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&general_page(win));
    dialog.add(&memory_page(win));
    dialog.add(&skills_page(win));
    dialog.add(&connections_page(win));
    dialog.add(&activity_page());
    dialog.present(Some(&win.win));
}

fn general_page(win: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();

    let abilities = adw::PreferencesGroup::builder().title("Abilities").build();
    let web = adw::SwitchRow::builder()
        .title("Search the Web")
        .active(win.st.borrow().web_enabled)
        .subtitle("Look things up with DuckDuckGo and read web pages")
        .build();
    let w = win.weak();
    web.connect_active_notify(move |row| {
        if let Some(w) = w.upgrade() {
            w.st.borrow_mut().web_enabled = row.is_active();
            config::update(&[("web_search", json!(row.is_active()))]);
            w.core.reload();
        }
    });
    abilities.add(&web);
    page.add(&abilities);

    let details = adw::PreferencesGroup::builder()
        .title("Answer Details")
        .description("A line under each answer")
        .build();
    let cfg = config::load();
    for (key, title, subtitle, default) in [
        (
            "stats_thinking",
            "Thinking Time",
            "How long the model thought before answering",
            true,
        ),
        (
            "stats_tokens",
            "Tokens",
            "How many tokens the answer took",
            false,
        ),
        (
            "stats_speed",
            "Speed",
            "How long the answer took, and tokens per second",
            true,
        ),
    ] {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(config::bool_or(&cfg, key, default))
            .build();
        row.connect_active_notify(move |row| config::update(&[(key, json!(row.is_active()))]));
        details.add(&row);
    }
    page.add(&details);

    let allowed = Rows::new(
        adw::PreferencesGroup::builder()
            .title("Allowed Without Asking")
            .description("Actions you chose “Always Allow” for")
            .build(),
    );
    page.add(&allowed.group);
    fn fill_allowed(allowed: &Rc<Rows>, win: &Rc<Window>) {
        allowed.clear();
        // The service writes Always Allow choices to the shared settings file.
        let names: Vec<String> = config::load()["always_allow"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        for name in &names {
            let title = match name.split_once("__") {
                Some((server, tool)) => format!("{server}: {tool}"),
                None => {
                    let t = name.replace('_', " ");
                    t[..1].to_uppercase() + &t[1..]
                }
            };
            let row = adw::ActionRow::builder().title(esc(&title)).build();
            let ask = gtk::Button::builder()
                .label("Ask Again")
                .valign(gtk::Align::Center)
                .build();
            let (a, w, n) = (allowed.clone(), win.weak(), name.clone());
            ask.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    let rest: Vec<Value> = config::load()["always_allow"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|v| v.as_str() != Some(&n))
                        .cloned()
                        .collect();
                    config::update(&[("always_allow", Value::Array(rest))]);
                    w.core.reload();
                    fill_allowed(&a, &w);
                }
            });
            row.add_suffix(&ask);
            allowed.add(&row);
        }
        allowed.group.set_visible(!names.is_empty());
    }
    fill_allowed(&allowed, win);

    let smart = adw::PreferencesGroup::builder().title("Smarter Model")
        .description("When a request is too hard, the model can hand it to a bigger one. On the NPU only one model is \
loaded at a time, so the hand-off takes a moment. With Lemonade, a model marked “graphics chip” runs next to the default \
one instead, at the cost of more power and memory, so Automatic never picks one. A minute after it's done, the bigger \
model is unloaded.")
        .build();
    let (models, current, choice) = {
        let st = win.st.borrow();
        (
            st.models.clone(),
            st.model.clone(),
            st.smarter_choice.clone(),
        )
    };
    let candidates: Vec<&assistant_core::flm::Model> = models
        .iter()
        .filter(|m| m.installed && m.can_use_tools() && !m.single_turn() && m.name != current)
        .collect();
    let auto = pick_smarter_model(&models, &current, None);
    let mut values = vec!["auto".to_string(), "off".to_string()];
    values.extend(candidates.iter().map(|m| m.name.clone()));
    let mut labels = vec![
        auto.map(|a| format!("Automatic ({a})"))
            .unwrap_or("Automatic (none available)".into()),
        "Off".into(),
    ];
    labels.extend(candidates.iter().map(|m| m.display_name()));
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let smart_row = adw::ComboRow::builder()
        .title("Hand Hard Requests To")
        .model(&gtk::StringList::new(&label_refs))
        .build();
    smart_row.set_selected(values.iter().position(|v| *v == choice).unwrap_or(0) as u32);
    let w = win.weak();
    smart_row.connect_selected_notify(move |row| {
        if let Some(w) = w.upgrade() {
            let v = values[row.selected() as usize].clone();
            w.st.borrow_mut().smarter_choice = v.clone();
            config::update(&[("smarter_model", json!(v))]);
            w.core.reload();
        }
    });
    smart.add(&smart_row);
    page.add(&smart);

    let memory = adw::PreferencesGroup::builder().title("Memory")
        .description("The default model stays loaded so answers start quickly; set a timer to unload it when idle. Bigger models never stay.")
        .build();
    let ctx_labels: Vec<&str> = CTX_CHOICES.iter().map(|(_, l)| *l).collect();
    let ctx_row = adw::ComboRow::builder()
        .title("Context Length")
        .subtitle("16K and more: every tool goes with every question, so answers start sooner")
        .model(&gtk::StringList::new(&ctx_labels))
        .build();
    let ctx = win.st.borrow().ctx_len;
    ctx_row.set_selected(CTX_CHOICES.iter().position(|(v, _)| *v == ctx).unwrap_or(2) as u32);
    let w = win.weak();
    ctx_row.connect_selected_notify(move |row| {
        if let Some(w) = w.upgrade() {
            let v = CTX_CHOICES[row.selected() as usize].0;
            w.st.borrow_mut().ctx_len = v;
            config::update(&[("ctx_len", json!(v))]);
            w.core.reload(); // the service restarts FastFlowLM with the new length on next use
        }
    });
    let idle_labels: Vec<&str> = IDLE_CHOICES.iter().map(|(_, l)| *l).collect();
    let idle_row = adw::ComboRow::builder()
        .title("Unload Model")
        .subtitle("Frees memory when the assistant isn't used")
        .model(&gtk::StringList::new(&idle_labels))
        .build();
    let idle = win.st.borrow().unload_after;
    idle_row.set_selected(
        IDLE_CHOICES
            .iter()
            .position(|(v, _)| *v == idle)
            .unwrap_or(IDLE_CHOICES.len() - 1) as u32,
    );
    let w = win.weak();
    idle_row.connect_selected_notify(move |row| {
        if let Some(w) = w.upgrade() {
            let v = IDLE_CHOICES[row.selected() as usize].0;
            w.st.borrow_mut().unload_after = v;
            config::update(&[("unload_after_minutes", json!(v))]);
            w.core.reload();
        }
    });
    memory.add(&ctx_row);
    memory.add(&idle_row);
    page.add(&memory);
    page
}

fn memory_page(win: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Memory")
        .icon_name("starred-symbolic")
        .build();
    let top = adw::PreferencesGroup::builder()
        .description("The assistant remembers what you ask it to, in every new chat. It's kept on this computer only.").build();
    let switch = adw::SwitchRow::builder()
        .title("Remember Things")
        .active(win.st.borrow().memory_enabled)
        .build();
    let w = win.weak();
    switch.connect_active_notify(move |row| {
        if let Some(w) = w.upgrade() {
            w.st.borrow_mut().memory_enabled = row.is_active();
            config::update(&[("memory", json!(row.is_active()))]);
            w.core.reload();
        }
    });
    top.add(&switch);
    page.add(&top);
    let facts = Rows::new(adw::PreferencesGroup::builder().title("Remembered").build());
    page.add(&facts.group);
    fn fill(facts: &Rc<Rows>, win: &Rc<Window>) {
        facts.clear();
        let add = adw::EntryRow::builder()
            .title("Add Something to Remember")
            .show_apply_button(true)
            .build();
        let (f, w) = (facts.clone(), win.weak());
        add.connect_apply(move |row| {
            if let Some(w) = w.upgrade() {
                if let Err(e) = store::remember(&row.text()) {
                    w.toast(&(e[..1].to_uppercase() + &e[1..]));
                }
                w.core.reload();
                fill(&f, &w);
            }
        });
        facts.add(&add);
        for fact in store::facts() {
            let row = adw::ActionRow::builder()
                .title(esc(&fact))
                .title_lines(0)
                .build();
            let delete = flat_button("user-trash-symbolic", "Forget");
            let (f, w) = (facts.clone(), win.weak());
            delete.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    store::forget(&fact);
                    w.core.reload();
                    fill(&f, &w);
                }
            });
            row.add_suffix(&delete);
            facts.add(&row);
        }
    }
    fill(&facts, win);
    page
}

/// Everything the assistant did, newest first.
fn activity_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Activity")
        .icon_name("document-open-recent-symbolic")
        .build();
    let rows = Rows::new(
        adw::PreferencesGroup::builder()
            .title("What the Assistant Did")
            .description("Every action, newest first. Kept on this computer only (the last 500).")
            .build(),
    );
    let clear = gtk::Button::builder()
        .label("Clear")
        .valign(gtk::Align::Center)
        .build();
    clear.add_css_class("flat");
    rows.group.set_header_suffix(Some(&clear));
    page.add(&rows.group);
    fn fill(rows: &Rc<Rows>) {
        rows.clear();
        let entries = activity::recent(200);
        if entries.is_empty() {
            rows.add(
                &adw::ActionRow::builder()
                    .title("Nothing yet")
                    .subtitle("Actions show up here as they happen")
                    .build(),
            );
        }
        for e in entries {
            let when = glib::DateTime::from_unix_local(e["time"].as_i64().unwrap_or(0))
                .ok()
                .and_then(|t| t.format("%e %b, %H:%M").ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let mut sub = vec![when];
            match e["decision"].as_str() {
                Some("declined") => sub.push("you cancelled it".into()),
                Some("allowed") => sub.push("you allowed it".into()),
                Some("always") => sub.push("you chose Always Allow".into()),
                _ => {}
            }
            if e["from"] == "search" {
                sub.push("from search".into());
            }
            let result = e["result"].as_str().unwrap_or("");
            let row = adw::ActionRow::builder()
                .title(esc(e["summary"].as_str().unwrap_or("")))
                .subtitle(esc(&sub.join(" · ")))
                .tooltip_text(result)
                .build();
            let ok = e["ok"].as_bool().unwrap_or(false);
            let icon = gtk::Image::from_icon_name(if ok {
                "object-select-symbolic"
            } else {
                "dialog-warning-symbolic"
            });
            if !ok {
                icon.add_css_class("warning");
            }
            row.add_prefix(&icon);
            rows.add(&row);
        }
    }
    fill(&rows);
    let r = rows.clone();
    clear.connect_clicked(move |_| {
        activity::clear();
        fill(&r);
    });
    page
}

fn skills_page(win: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Skills")
        .icon_name("view-list-bullet-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder().title("Skills")
        .description("Saved workflows. Ask “save that as a skill”, or add Markdown files to the skills folder. Run one by typing / and its name.")
        .build();
    let folder = flat_button("folder-open-symbolic", "Open Skills Folder");
    let w = win.weak();
    folder.connect_clicked(move |_| {
        if let Some(w) = w.upgrade() {
            let dir = store::skills_dir();
            let _ = std::fs::create_dir_all(&dir);
            gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(
                Some(&w.win),
                None::<&gio::Cancellable>,
                |_| {},
            );
        }
    });
    group.set_header_suffix(Some(&folder));
    page.add(&group);
    let rows = Rows::new(group);
    fn fill(rows: &Rc<Rows>, win: &Rc<Window>) {
        rows.clear();
        let skills = store::skills();
        if skills.is_empty() {
            rows.add(
                &adw::ActionRow::builder()
                    .title("No skills yet")
                    .subtitle(
                        "Try: “Save a skill called morning: turn on dark mode, set volume to 30%”",
                    )
                    .build(),
            );
        }
        for skill in skills {
            let sub = if skill.description.is_empty() {
                format!("/{}", skill.slug)
            } else {
                format!("/{} · {}", skill.slug, skill.description)
            };
            let row = adw::ActionRow::builder()
                .title(esc(&skill.title))
                .subtitle(esc(&sub))
                .build();
            let delete = flat_button("user-trash-symbolic", "Delete Skill");
            let (r, w, slug) = (rows.clone(), win.weak(), skill.slug.clone());
            delete.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    store::delete_skill(&slug);
                    w.core.reload();
                    fill(&r, &w);
                }
            });
            row.add_suffix(&delete);
            rows.add(&row);
        }
    }
    fill(&rows, win);
    page
}

fn connections_page(win: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Connections")
        .icon_name("network-server-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder().title("Apps and Services")
        .description(format!("The assistant can use tools that other apps and services offer through the Model Context \
Protocol (MCP). Tools that change things ask first. Each one uses some of the model's memory, so at most {}.", mcp::MAX_TOOLS))
        .build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let edit = flat_button("document-edit-symbolic", "Edit Configuration File");
    let add = flat_button("list-add-symbolic", "Add Server");
    buttons.append(&edit);
    buttons.append(&add);
    group.set_header_suffix(Some(&buttons));
    page.add(&group);
    let rows = Rows::new(group);

    fn status(s: &Value) -> String {
        if s["disabled"] == true {
            return "Off".into();
        }
        if let Some(e) = s["error"].as_str() {
            return format!("Couldn't connect: {e}");
        }
        if s["running"] == true {
            let n = s["tools"].as_u64().unwrap_or(0);
            return format!(
                "{n} tool{}{}",
                if n == 1 { "" } else { "s" },
                if s["trust"] == true {
                    " · trusted"
                } else {
                    ""
                }
            );
        }
        "Connects when you send a message".into()
    }
    fn changed(rows: &Rc<Rows>, win: &Rc<Window>) {
        if let Err(e) = win.core.reload_now() {
            win.toast(&e);
        }
        fill(rows, win);
    }
    fn fill(rows: &Rc<Rows>, win: &Rc<Window>) {
        rows.clear();
        let info = win
            .core
            .connections()
            .unwrap_or_else(|e| json!({"servers": [], "config_error": e}));
        if let Some(e) = info["config_error"].as_str() {
            rows.add(
                &adw::ActionRow::builder()
                    .title("The configuration file has a mistake")
                    .subtitle(esc(e))
                    .build(),
            );
        } else if info["servers"].as_array().is_none_or(|a| a.is_empty()) {
            rows.add(
                &adw::ActionRow::builder()
                    .title("No servers yet")
                    .subtitle("Add one with the + button")
                    .build(),
            );
        }
        for s in info["servers"].as_array().into_iter().flatten() {
            let name = s["name"].as_str().unwrap_or("").to_string();
            let row = adw::ActionRow::builder()
                .title(esc(&name))
                .subtitle(esc(&format!(
                    "{}\n{}",
                    status(s),
                    s["target"].as_str().unwrap_or("")
                )))
                .subtitle_lines(3)
                .build();
            let switch = gtk::Switch::builder()
                .active(s["disabled"] != true)
                .valign(gtk::Align::Center)
                .build();
            let (r, w, n) = (rows.clone(), win.weak(), name.clone());
            switch.connect_active_notify(move |sw| {
                if let Some(w) = w.upgrade() {
                    let mut configs = mcp::load_config().unwrap_or_default();
                    for c in configs.iter_mut().filter(|c| c.name == n) {
                        c.disabled = !sw.is_active();
                    }
                    let _ = mcp::save_config(&configs);
                    changed(&r, &w);
                }
            });
            let delete = flat_button("user-trash-symbolic", "Remove Server");
            let (r, w, n) = (rows.clone(), win.weak(), name);
            delete.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    let configs: Vec<_> = mcp::load_config()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|c| c.name != n)
                        .collect();
                    let _ = mcp::save_config(&configs);
                    changed(&r, &w);
                }
            });
            row.add_suffix(&switch);
            row.add_suffix(&delete);
            rows.add(&row);
        }
    }
    let w = win.weak();
    edit.connect_clicked(move |_| {
        if let Some(w) = w.upgrade() {
            let path = mcp::config_path();
            if !path.exists() {
                let _ = mcp::save_config(&[]);
            }
            gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
                Some(&w.win),
                None::<&gio::Cancellable>,
                |_| {},
            );
        }
    });
    let (r, w, p) = (rows.clone(), win.weak(), page.clone());
    add.connect_clicked(move |_| {
        let Some(win) = w.upgrade() else { return };
        let name = adw::EntryRow::builder().title("Name").build();
        let target = adw::EntryRow::builder().title("Command or URL").build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list.add_css_class("boxed-list");
        list.append(&name);
        list.append(&target);
        let dialog = adw::AlertDialog::builder().heading("Add Server")
            .body("The command that starts the server, e.g. “npx -y @modelcontextprotocol/server-memory”, or its web address.")
            .extra_child(&list).build();
        dialog.add_responses(&[("cancel", "_Cancel"), ("add", "_Add")]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("add"));
        let (r, w) = (r.clone(), win.weak());
        dialog.connect_response(None, move |_, response| {
            let Some(win) = w.upgrade() else { return };
            if response != "add" {
                return;
            }
            match mcp::parse_server(&name.text(), &target.text()) {
                Ok(cfg) => {
                    let mut configs: Vec<_> = mcp::load_config().unwrap_or_default().into_iter().filter(|c| c.name != cfg.name).collect();
                    configs.push(cfg);
                    if let Err(e) = mcp::save_config(&configs) {
                        win.toast(&e);
                    }
                    changed(&r, &win);
                }
                Err(e) => win.toast(&(e[..1].to_uppercase() + &e[1..])),
            }
        });
        dialog.present(p.root().as_ref());
    });
    fill(&rows, win);
    page
}

const SHORTCUTS: [(&str, &str); 13] = [
    ("New Chat", "<Ctrl>n"),
    ("Focus Message Field", "<Ctrl>l"),
    ("Search Chats", "<Ctrl>f"),
    ("Attach Image", "<Ctrl><Shift>a"),
    ("Bigger Text", "<Ctrl>plus"),
    ("Smaller Text", "<Ctrl>minus"),
    ("Normal Text Size", "<Ctrl>0"),
    ("Stop Answer", "Escape"),
    ("Preferences", "<Ctrl>comma"),
    ("Keyboard Shortcuts", "<Ctrl>question"),
    ("Close Window", "<Ctrl>w"),
    ("Quit", "<Ctrl>q"),
    ("", ""),
];

#[cfg(feature = "adw-1-8")]
pub fn show_shortcuts(win: &Rc<Window>) {
    let dialog = adw::ShortcutsDialog::new();
    let section = adw::ShortcutsSection::new(Some("General"));
    for (label, accel) in SHORTCUTS.iter().filter(|(l, _)| !l.is_empty()) {
        section.add(adw::ShortcutsItem::new(label, accel));
    }
    dialog.add(section);
    dialog.present(Some(&win.win));
}

#[cfg(not(feature = "adw-1-8"))]
#[allow(deprecated)]
pub fn show_shortcuts(win: &Rc<Window>) {
    let group = gtk::ShortcutsGroup::builder().title("General").build();
    for (label, accel) in SHORTCUTS.iter().filter(|(l, _)| !l.is_empty()) {
        group.add_shortcut(
            &gtk::ShortcutsShortcut::builder()
                .title(*label)
                .accelerator(*accel)
                .build(),
        );
    }
    let section = gtk::ShortcutsSection::builder().visible(true).build();
    section.add_group(&group);
    let shortcuts = gtk::ShortcutsWindow::builder()
        .transient_for(&win.win)
        .modal(true)
        .build();
    shortcuts.add_section(&section);
    shortcuts.present();
}
