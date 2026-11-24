//! One assistant turn in the window: its name and model, a status
//! pill while it works, the answer as it streams, action pills, Allow/Cancel
//! cards, and a copy button at the end.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use adw::prelude::*;
use gtk::gio;

use crate::core::Turn;
use crate::markup::to_pango;
use crate::window::{APP_NAME, Window};

fn tool_icon(name: &str) -> &'static str {
    match name {
        "get_weather" => "weather-overcast-symbolic",
        "list_reminders" => "alarm-symbolic",
        "web_search" | "find_files" | "search_file_contents" => "system-search-symbolic",
        "read_web_page" | "open_url" => "web-browser-symbolic",
        "open_app" | "run_app_action" => "application-x-executable-symbolic",
        "open_file" => "folder-open-symbolic",
        "list_folder" => "folder-symbolic",
        "read_file" => "text-x-generic-symbolic",
        "read_document" => "x-office-document-symbolic",
        "get_battery" => "battery-good-symbolic",
        "get_system_info" => "computer-symbolic",
        "get_datetime" => "preferences-system-time-symbolic",
        "set_dark_mode" => "weather-clear-night-symbolic",
        "set_night_light" => "night-light-symbolic",
        "set_power_mode" => "power-profile-balanced-symbolic",
        "set_volume" => "audio-volume-high-symbolic",
        "set_mute" => "audio-volume-muted-symbolic",
        "set_brightness" => "display-brightness-symbolic",
        "set_wifi" | "list_networks" | "connect_network" => "network-wireless-symbolic",
        "disconnect_network" => "network-offline-symbolic",
        "set_bluetooth" | "list_bluetooth" | "connect_bluetooth" => "bluetooth-active-symbolic",
        "disconnect_bluetooth" => "bluetooth-disabled-symbolic",
        "set_do_not_disturb" => "notifications-disabled-symbolic",
        "copy_to_clipboard" => "edit-copy-symbolic",
        "remember" => "starred-symbolic",
        "forget" => "non-starred-symbolic",
        "use_skill" => "view-list-bullet-symbolic",
        "save_skill" => "document-save-symbolic",
        "look_at_screen" => "camera-photo-symbolic",
        "look_at_image" | "edit_image" => "image-x-generic-symbolic",
        "images_to_pdf" => "x-office-document-symbolic",
        "calculate" => "accessories-calculator-symbolic",
        "undo" => "edit-undo-symbolic",
        "run_command" => "utilities-terminal-symbolic",
        "send_file" => "send-to-symbolic",
        "list_devices" => "network-workgroup-symbolic",
        "read_window" => "view-reveal-symbolic",
        "get_events" => "x-office-calendar-symbolic",
        "find_contact" => "avatar-default-symbolic",
        "list_app_actions" => "view-more-symbolic",
        "media_control" => "media-playback-start-symbolic",
        n if n.contains("__") => "network-server-symbolic", // another app's tool (MCP)
        _ => "emblem-system-symbolic",
    }
}

/// What a step touched, for its menu: a web link, or a file or folder.
enum Target {
    Link(String),
    File(std::path::PathBuf),
}

/// A step's summary in plain words, and what it touched. "Read https://…" reads "Read
/// a web page" and "Edit ~/Obrazy/Zrzuty ekranu/a.png" reads "Edit a.png"; the link or
/// path goes in the details and the step's menu.
fn plain_step(name: &str, summary: &str) -> (String, Vec<Target>) {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let words: Vec<&str> = summary.split(' ').collect();
    let clean = |w: &str| {
        w.trim_matches(|c: char| matches!(c, '“' | '”' | '"' | '\'' | ',' | ')' | '('))
            .to_string()
    };
    let resolve = |w: &str| -> std::path::PathBuf {
        match w.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if w.starts_with('/') => w.into(),
            None => home.join(w),
        }
    };
    let mut label = summary.to_string();
    let mut targets = vec![];
    let mut i = 0;
    while i < words.len() {
        let w = clean(words[i]);
        if w.starts_with("http://") || w.starts_with("https://") {
            let url = w.trim_end_matches(['.', ',']).to_string();
            label = match name {
                "read_web_page" => "Read a web page".into(),
                "open_url" => "Open a link".into(),
                _ => label.replace(words[i], "a link"),
            };
            targets.push(Target::Link(url));
            i += 1;
            continue;
        }
        if !w.contains('/') || w.len() < 3 {
            i += 1;
            continue;
        }
        // The longest run of words from here that is a real path (folders with spaces).
        let mut found = None;
        for end in (i + 1..=words.len()).rev() {
            let text = clean(&words[i..end].join(" "));
            let path = resolve(&text);
            let pattern = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains(['*', '?']));
            let real = if pattern {
                path.parent().map(|p| p.to_path_buf())
            } else {
                Some(path.clone())
            };
            if let Some(real) = real.filter(|p| p.exists()) {
                found = Some((end, text, path, real, pattern));
                break;
            }
        }
        let Some((end, text, path, real, pattern)) = found else {
            i += 1;
            continue;
        };
        let name_of = |p: &std::path::Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        let shown = if pattern {
            format!("{}/{}", name_of(&real), name_of(&path))
        } else {
            name_of(&real)
        };
        label = label.replace(&text, &shown);
        targets.push(Target::File(real));
        i = end;
    }
    (label, targets)
}

/// The step's menu (as Fractal does for a message: a button on hover or focus, and
/// right-click), with what fits what it touched: Copy Link and Open in Browser, or
/// Open, Show in Files and Copy Path.
fn step_menu(pill: &gtk::Box, targets: &[Target], win: Weak<Window>) {
    if targets.is_empty() {
        return;
    }
    let actions = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    let copy = |text: String, toast: &'static str, w: Weak<Window>| {
        move |_: &gio::SimpleAction, _: Option<&glib::Variant>| {
            if let Some(d) = gtk::gdk::Display::default() {
                d.clipboard().set_text(&text);
            }
            if let Some(w) = w.upgrade() {
                w.toast(toast);
            }
        }
    };
    let open = |uri: String| {
        move |_: &gio::SimpleAction, _: Option<&glib::Variant>| {
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        }
    };
    let add =
        |id: &str, label: &str, f: Box<dyn Fn(&gio::SimpleAction, Option<&glib::Variant>)>| {
            let a = gio::SimpleAction::new(id, None);
            a.connect_activate(f);
            actions.add_action(&a);
            menu.append(Some(label), Some(&format!("step.{id}")));
        };
    match &targets[0] {
        Target::Link(url) => {
            add("open", "_Open in Browser", Box::new(open(url.clone())));
            add(
                "copy",
                "_Copy Link",
                Box::new(copy(url.clone(), "Link copied", win)),
            );
        }
        Target::File(path) => {
            let uri = gio::File::for_path(path).uri().to_string();
            add("open", "_Open", Box::new(open(uri.clone())));
            add(
                "show",
                "_Show in Files",
                Box::new(move |_, _| {
                    // The file manager selects it (org.freedesktop.FileManager1).
                    if let Ok(bus) =
                        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                    {
                        bus.call(
                            Some("org.freedesktop.FileManager1"),
                            "/org/freedesktop/FileManager1",
                            "org.freedesktop.FileManager1",
                            "ShowItems",
                            Some(&(vec![uri.as_str()], "").to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            -1,
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                }),
            );
            add(
                "copy",
                "Copy _Path",
                Box::new(copy(path.display().to_string(), "Path copied", win)),
            );
        }
    }
    pill.insert_action_group("step", Some(&actions));
    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .build();
    more.add_css_class("flat");
    more.add_css_class("circular");
    more.add_css_class("step-more");
    pill.append(&more);
    let click = gtk::GestureClick::builder().button(3).build();
    let (anchor, model) = (pill.clone(), menu.clone());
    click.connect_pressed(move |_, _, x, y| {
        let pop = gtk::PopoverMenu::from_model(Some(&model));
        pop.set_has_arrow(false);
        pop.set_parent(&anchor);
        pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| p.unparent());
        pop.popup();
    });
    pill.add_controller(click);
}

/// Notification text: markdown marks removed, cut at a word.
fn plain(markdown: &str) -> String {
    assistant_core::search::plain(markdown, 400)
}

#[cfg(feature = "adw-1-8")]
pub fn spinner() -> gtk::Widget {
    adw::Spinner::builder()
        .halign(gtk::Align::Start)
        .width_request(24)
        .height_request(24)
        .build()
        .upcast()
}

#[cfg(not(feature = "adw-1-8"))]
pub fn spinner() -> gtk::Widget {
    gtk::Spinner::builder()
        .spinning(true)
        .halign(gtk::Align::Start)
        .build()
        .upcast()
}

struct Inner {
    summaries: HashMap<String, String>,
    label: Option<gtk::Label>,
    text: String,
    answer: String,
    failure: String,
    stats: Option<serde_json::Value>,
    /// The model's reasoning as it streams: the block, its text, when it began.
    think: Option<(gtk::Expander, gtk::Label, String, Instant)>,
    /// Steps under way: the row shown while it runs, and when it started.
    running: HashMap<String, (gtk::Box, Instant)>,
    /// The task's checklist, updated in place.
    plan: Option<gtk::Box>,
}

pub struct Reply {
    win: Weak<Window>,
    /// The chat this answer belongs to (another may be open by the time it ends).
    chat_id: String,
    bx: gtk::Box,
    model_label: gtk::Label,
    spin: gtk::Box,
    spin_label: gtk::Label,
    inner: RefCell<Inner>,
}

impl Reply {
    pub fn new(win: &Rc<Window>, live: bool, model: Option<&str>) -> Rc<Self> {
        // The name and model, then the turn's content (no avatar, like a plain message).
        let bx = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .hexpand(true)
            .build();
        let row = bx.clone();
        let name = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::builder().label(APP_NAME).xalign(0.0).build();
        title.add_css_class("assistant-name");
        let model_label = gtk::Label::builder()
            .label(assistant_core::flm::pretty_model(
                &model
                    .map(String::from)
                    .unwrap_or_else(|| win.st.borrow().model.clone()),
            ))
            .xalign(0.0)
            .build();
        model_label.add_css_class("dim-label");
        model_label.add_css_class("caption");
        name.append(&title);
        name.append(&model_label);
        bx.append(&name);
        // The "working" pill: spinner + label, removed when the turn ends.
        let spin = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::Start)
            .build();
        spin.add_css_class("pill");
        spin.add_css_class("quiet");
        spin.append(&spinner());
        let spin_label = gtk::Label::new(Some("Thinking…"));
        spin.append(&spin_label);
        if live {
            bx.append(&spin);
            win.append_animated(&row);
        } else {
            win.messages.append(&row);
        }
        Rc::new(Self {
            win: win.weak(),
            chat_id: win.st.borrow().chat_id.clone(),
            bx,
            model_label,
            spin,
            spin_label,
            inner: RefCell::new(Inner {
                summaries: HashMap::new(),
                label: None,
                text: String::new(),
                answer: String::new(),
                failure: String::new(),
                stats: None,
                think: None,
                running: HashMap::new(),
                plan: None,
            }),
        })
    }

    fn scroll(&self) {
        if let Some(w) = self.win.upgrade() {
            w.scroll_down();
        }
    }

    fn before_spinner(&self, widget: &impl IsA<gtk::Widget>) {
        if self.spin.parent().is_some() {
            self.bx
                .insert_child_after(widget, self.spin.prev_sibling().as_ref());
        } else {
            self.bx.append(widget);
        }
    }

    pub fn set_summary(&self, name: &str, summary: &str) {
        self.inner
            .borrow_mut()
            .summaries
            .insert(name.into(), summary.into());
    }

    /// A finished answer paragraph (history).
    pub fn add_text_block(&self, text: &str) {
        self.on_text(text);
        let mut i = self.inner.borrow_mut();
        i.label = None;
        i.answer.push('\n');
    }

    fn pill(&self, icon: &str, text: &str, failed: bool, tooltip: Option<&str>) -> gtk::Box {
        let pill = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::Start)
            .build();
        pill.set_tooltip_text(tooltip);
        pill.add_css_class("pill");
        if failed {
            pill.add_css_class("failed");
        }
        pill.append(&gtk::Image::from_icon_name(icon));
        pill.append(
            &gtk::Label::builder()
                .label(text)
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        pill
    }

    /// The reasoning is over (an answer or a step began): its block says for how long.
    fn end_thinking(&self) {
        if let Some((block, _, _, began)) = self.inner.borrow_mut().think.take() {
            block.set_label(Some(&format!(
                "Thought for {:.1} s",
                began.elapsed().as_secs_f64()
            )));
            block.remove_css_class("live");
        }
    }

    pub fn error(&self, text: &str) {
        self.inner.borrow_mut().failure = text.into();
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .build();
        label.add_css_class("error");
        self.before_spinner(&label);
    }

    pub fn finish(&self, notify: bool) {
        self.end_thinking();
        if self.spin.parent().is_some() {
            self.bx.remove(&self.spin);
        }
        let (answer, failure) = {
            let i = self.inner.borrow();
            (i.answer.trim().to_string(), i.failure.clone())
        };
        let Some(win) = self.win.upgrade() else {
            return;
        };
        if let Some(line) = self.inner.borrow().stats.as_ref().and_then(stats_line) {
            let l = gtk::Label::builder().label(line).xalign(0.0).build();
            l.add_css_class("dim-label");
            l.add_css_class("caption");
            self.bx.append(&l);
        }
        if !answer.is_empty() {
            let copy = gtk::Button::builder()
                .icon_name("edit-copy-symbolic")
                .tooltip_text("Copy")
                .halign(gtk::Align::Start)
                .build();
            copy.add_css_class("flat");
            copy.add_css_class("circular");
            let (w, a) = (self.win.clone(), answer.clone());
            copy.connect_clicked(move |b| {
                b.clipboard().set_text(&a);
                if let Some(w) = w.upgrade() {
                    w.toast("Copied");
                }
            });
            self.bx.append(&copy);
        }
        if notify && !win.in_view() {
            let chat = win.st.borrow().chat_id.clone();
            if !answer.is_empty() {
                notify_app(&win.app, APP_NAME, &plain(&answer), &chat, "Open Chat");
            } else if !failure.is_empty() {
                notify_app(
                    &win.app,
                    &format!("{APP_NAME} couldn't answer"),
                    &failure,
                    &chat,
                    "Open Chat",
                );
            }
        }
    }

    /// Weather, events, reminders: the first line as the heading, the rest as rows.
    fn card(&self, icon: &str, text: &str) {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .halign(gtk::Align::Start)
            .build();
        card.add_css_class("card");
        card.add_css_class("info-card");
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&gtk::Image::from_icon_name(icon));
        let title = gtk::Label::builder()
            .label(lines.next().unwrap_or(""))
            .xalign(0.0)
            .wrap(true)
            .build();
        title.add_css_class("heading");
        head.append(&title);
        card.append(&head);
        for l in lines {
            let row = gtk::Label::builder()
                .label(l)
                .xalign(0.0)
                .wrap(true)
                .build();
            card.append(&row);
        }
        self.before_spinner(&card);
    }

    fn confirm_card(self: &Rc<Self>, request: u32, summary: &str, detail: &str, can_always: bool) {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        card.add_css_class("card");
        card.add_css_class("confirm-card");
        let head = gtk::Box::builder()
            .spacing(10)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        head.append(
            &gtk::Image::builder()
                .icon_name("dialog-question-symbolic")
                .pixel_size(20)
                .build(),
        );
        let title = gtk::Label::builder()
            .label(format!("{summary}?"))
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        title.add_css_class("heading");
        head.append(&title);
        let buttons = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::End)
            .margin_bottom(12)
            .margin_end(12)
            .build();
        let cancel = gtk::Button::with_mnemonic("_Cancel");
        let allow = gtk::Button::with_mnemonic("_Allow");
        allow.add_css_class("suggested-action");
        buttons.append(&cancel);
        let always = can_always.then(|| {
            let b = gtk::Button::with_mnemonic("Always A_llow");
            b.set_tooltip_text(Some("Don't ask again for this kind of action"));
            buttons.append(&b);
            b
        });
        let task = can_always.then(|| {
            let b = gtk::Button::with_mnemonic("Allow for This _Task");
            b.set_tooltip_text(Some("Don't ask again until this task is done"));
            buttons.append(&b);
            b
        });
        buttons.append(&allow);
        card.append(&head);
        if !detail.is_empty() {
            let code = gtk::Label::builder()
                .label(detail)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(true)
                .margin_start(12)
                .margin_end(12)
                .build();
            code.add_css_class("monospace");
            code.add_css_class("confirm-detail");
            card.append(&code);
        }
        card.append(&buttons);
        self.before_spinner(&card);
        self.inner.borrow_mut().label = None;
        self.spin_label.set_label("Waiting for you…");
        self.scroll();
        let pick = {
            let (me, card, buttons, title, summary) = (
                Rc::downgrade(self),
                card.clone(),
                buttons.clone(),
                title.clone(),
                summary.to_string(),
            );
            Rc::new(move |answer: &str| {
                let Some(me) = me.upgrade() else { return };
                me.spin_label.set_label("Working…");
                card.remove(&buttons);
                title.remove_css_class("heading");
                title.set_label(&format!(
                    "{summary}: {}",
                    match answer {
                        "deny" => "cancelled",
                        "task" => "allowed for this task",
                        _ => "allowed",
                    }
                ));
                if let Some(w) = me.win.upgrade() {
                    w.core.confirm(request, answer);
                }
            })
        };
        let p = pick.clone();
        allow.connect_clicked(move |_| p("allow"));
        let p = pick.clone();
        cancel.connect_clicked(move |_| p("deny"));
        if let Some(b) = task {
            let p = pick.clone();
            b.connect_clicked(move |_| p("task"));
        }
        if let Some(b) = always {
            b.connect_clicked(move |_| pick("always"));
        }
        allow.grab_focus();
        if let Some(win) = self.win.upgrade()
            && !win.in_view()
        {
            let chat = win.st.borrow().chat_id.clone();
            notify_app(
                &win.app,
                &format!("{APP_NAME} needs your OK"),
                &format!("{summary}?"),
                &chat,
                "Review",
            );
        }
    }
}

/// One notification at a time; clicking it opens the chat.
pub fn notify_app(app: &adw::Application, title: &str, body: &str, chat_id: &str, button: &str) {
    let note = gio::Notification::new(title);
    note.set_body(Some(body));
    note.set_default_action_and_target_value("app.open-chat", Some(&chat_id.to_variant()));
    note.add_button_with_target_value(button, "app.open-chat", Some(&chat_id.to_variant()));
    app.send_notification(Some("panda"), &note);
}

/// "Thought for 4.2 s · 312 tokens · 9.8 s, 21 t/s", as the Preferences switches say.
fn stats_line(s: &serde_json::Value) -> Option<String> {
    let cfg = crate::config::load();
    let on = |k: &str, d: bool| crate::config::bool_or(&cfg, k, d);
    let f = |k: &str| s[k].as_f64().unwrap_or(0.0);
    let mut parts = vec![];
    if on("stats_thinking", true) && f("think_seconds") >= 0.1 {
        parts.push(format!("Thought for {:.1} s", f("think_seconds")));
    }
    if on("stats_tokens", false) && f("tokens") > 0.0 {
        parts.push(format!("{} tokens", f("tokens") as u64));
    }
    if on("stats_speed", true) && f("seconds") > 0.0 {
        let rate = f("tokens_per_second");
        parts.push(if rate > 0.0 {
            format!("{:.1} s, {:.0} t/s", f("seconds"), rate)
        } else {
            format!("{:.1} s", f("seconds"))
        });
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

impl Turn for Reply {
    fn on_stats(&self, json: &str) {
        self.inner.borrow_mut().stats = serde_json::from_str(json).ok();
    }

    fn on_text(&self, s: &str) {
        self.end_thinking();
        self.spin_label.set_label("Writing…");
        let mut i = self.inner.borrow_mut();
        if i.label.is_none() {
            let label = gtk::Label::builder()
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .selectable(true)
                .build();
            label.add_css_class("assistant-text");
            self.before_spinner(&label);
            i.label = Some(label);
            i.text.clear();
        }
        i.text.push_str(s);
        i.answer.push_str(s);
        let label = i.label.clone().unwrap();
        let shown = i.text.trim_start().to_string();
        drop(i);
        // Malformed markup (shouldn't happen: it's escaped) falls back to plain text.
        if gtk::pango::parse_markup(&to_pango(&shown), '\0').is_ok() {
            label.set_markup(&to_pango(&shown));
        } else {
            label.set_text(&shown);
        }
        if self.spin.parent().is_some() {
            self.scroll();
        }
    }

    /// The text since the last action was a claim the service took back.
    fn on_retract(&self) {
        let mut i = self.inner.borrow_mut();
        if let Some(label) = i.label.take() {
            self.bx.remove(&label);
        }
        let keep = i.answer.len().saturating_sub(i.text.len());
        i.answer.truncate(keep);
        i.text.clear();
        drop(i);
        self.spin_label.set_label("Working…");
    }

    /// The reasoning streams into a block that opens on a click; it's closed by
    /// default so the answer stays the main thing.
    fn on_reasoning(&self, s: &str) {
        self.spin_label.set_label("Thinking…");
        let mut i = self.inner.borrow_mut();
        if i.think.is_none() {
            let text = gtk::Label::builder()
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .selectable(true)
                .build();
            text.add_css_class("thinking-text");
            let block = gtk::Expander::builder()
                .label("Thinking…")
                .child(&text)
                .build();
            block.add_css_class("thinking");
            block.add_css_class("live");
            self.before_spinner(&block);
            i.think = Some((block, text, String::new(), Instant::now()));
        }
        let (_, text, all, _) = i.think.as_mut().unwrap();
        all.push_str(s);
        text.set_text(all.trim());
        drop(i);
        self.scroll();
    }

    /// A step shows while it runs, with a spinner, and becomes its result when done.
    fn on_tool_started(&self, name: &str, summary: &str) {
        self.end_thinking();
        self.set_summary(name, summary);
        self.spin_label.set_label(&format!("{summary}…"));
        let row = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::Start)
            .build();
        row.add_css_class("pill");
        row.add_css_class("quiet");
        row.append(&spinner());
        row.append(&gtk::Label::new(Some(&plain_step(name, summary).0)));
        self.before_spinner(&row);
        self.inner
            .borrow_mut()
            .running
            .insert(name.into(), (row, Instant::now()));
        self.scroll();
    }

    fn on_tool_finished(&self, name: &str, result: &str, ok: bool) {
        let running = self.inner.borrow_mut().running.remove(name);
        if let Some((row, _)) = &running {
            self.bx.remove(row);
        }
        if ok && assistant_core::tools::CARDS.contains(&name) && !result.starts_with("No ") {
            self.card(tool_icon(name), result);
            return;
        }
        let short = result.chars().count() <= 90 && !result.contains('\n');
        let pill = if ok {
            // Short confirmations ("Dark mode on.") read best; long results go in the tooltip.
            let summary = self
                .inner
                .borrow()
                .summaries
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string());
            let (label, _) = plain_step(name, &summary);
            self.pill(
                tool_icon(name),
                if short { result } else { &label },
                false,
                None,
            )
        } else {
            let text: String = if short {
                result.to_string()
            } else {
                result.chars().take(200).collect()
            };
            self.pill("dialog-warning-symbolic", &text, true, None)
        };
        if ok
            && assistant_core::tools::UNDOABLE.contains(&name)
            && let Some(win) = self.win.upgrade()
        {
            // Undo right on the action: the service reverses its newest change.
            let undo = gtk::Button::builder()
                .icon_name("edit-undo-symbolic")
                .tooltip_text("Undo")
                .valign(gtk::Align::Center)
                .build();
            undo.add_css_class("flat");
            undo.add_css_class("circular");
            let w = win.weak();
            undo.connect_clicked(move |b| {
                if let Some(w) = w.upgrade() {
                    b.set_sensitive(false);
                    let msg = w.core.undo();
                    w.toast(&msg);
                }
            });
            pill.append(&undo);
        }
        if let Some((_, began)) = running {
            let took = gtk::Label::new(Some(&format!("{:.1} s", began.elapsed().as_secs_f64())));
            took.add_css_class("step-time");
            pill.append(&took);
        }
        let targets = plain_step(
            name,
            &self
                .inner
                .borrow()
                .summaries
                .get(name)
                .cloned()
                .unwrap_or_default(),
        )
        .1;
        step_menu(&pill, &targets, self.win.clone());
        let link = targets.first().map(|t| match t {
            Target::Link(u) => u.clone(),
            Target::File(p) => p.display().to_string(),
        });
        // A long result opens under its step, instead of hiding in a tooltip.
        if ok && !short {
            let shown: String = result.chars().take(4000).collect();
            let detail = gtk::Label::builder()
                .label(match &link {
                    Some(url) => format!("{url}\n\n{shown}"),
                    None => shown,
                })
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .selectable(true)
                .build();
            detail.add_css_class("step-detail");
            // One line in the title (the details have it all): a wrapping label in an
            // expander's title gets almost no width and grows tall.
            if let Some(label) = pill
                .first_child()
                .and_then(|c| c.next_sibling())
                .and_downcast::<gtk::Label>()
            {
                label.set_wrap(false);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(60);
            }
            let step = gtk::Expander::builder()
                .label_widget(&pill)
                .child(&detail)
                .halign(gtk::Align::Start)
                .build();
            step.add_css_class("step");
            self.before_spinner(&step);
        } else {
            self.before_spinner(&pill);
        }
        let mut i = self.inner.borrow_mut();
        i.label = None; // text after an action starts a new message
        i.answer.push('\n');
        drop(i);
        self.scroll();
    }

    /// A checklist card: made on the first plan, its rows redrawn as steps are ticked.
    fn on_plan(&self, json: &str) {
        let Ok(serde_json::Value::Array(steps)) = serde_json::from_str(json) else {
            return;
        };
        let card = self.inner.borrow().plan.clone().unwrap_or_else(|| {
            let card = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .halign(gtk::Align::Start)
                .build();
            card.add_css_class("card");
            card.add_css_class("plan-card");
            self.before_spinner(&card);
            self.inner.borrow_mut().plan = Some(card.clone());
            card
        });
        while let Some(c) = card.first_child() {
            card.remove(&c);
        }
        let done = steps.iter().filter(|s| s["done"] == true).count();
        let head = gtk::Label::builder()
            .label(format!("Plan · {done} of {}", steps.len()))
            .xalign(0.0)
            .build();
        head.add_css_class("heading");
        card.append(&head);
        for s in &steps {
            let ticked = s["done"] == true;
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            // The step under way spins; done ones are ticked.
            let icon: gtk::Widget = if s["now"] == true && !ticked {
                row.add_css_class("now");
                spinner()
            } else {
                gtk::Image::from_icon_name(if ticked {
                    "object-select-symbolic"
                } else {
                    "radio-symbolic"
                })
                .upcast()
            };
            let label = gtk::Label::builder()
                .label(s["step"].as_str().unwrap_or(""))
                .xalign(0.0)
                .wrap(true)
                .build();
            if ticked {
                row.add_css_class("done");
            }
            row.append(&icon);
            row.append(&label);
            card.append(&row);
        }
        self.scroll();
    }

    fn on_progress(&self, text: &str) {
        self.spin_label.set_label(text);
    }

    fn on_notice(&self, text: &str) {
        if let Some(w) = self.win.upgrade() {
            w.toast(text);
        }
    }

    fn on_handoff(&self, model: &str) {
        self.end_thinking();
        self.before_spinner(&self.pill(
            "go-next-symbolic",
            &format!("Asked {}", assistant_core::flm::pretty_model(model)),
            false,
            None,
        ));
        self.model_label
            .set_label(&assistant_core::flm::pretty_model(model));
        self.spin_label.set_label(&format!(
            "Loading {}…",
            assistant_core::flm::pretty_model(model)
        ));
        let mut i = self.inner.borrow_mut();
        i.label = None;
        i.answer.clear(); // the bigger model's reply is the answer
        drop(i);
        self.scroll();
    }

    fn on_compacting(&self) {
        self.spin_label.set_label("Summarizing older messages…");
    }

    fn on_confirm(&self, request: u32, _name: &str, summary: &str, detail: &str, can_always: bool) {
        // Asked from a chat that isn't open: say so; opening it shows the card
        // (the window picks up a running answer and its waiting cards).
        if let Some(w) = self.win.upgrade()
            && w.st.borrow().chat_id != self.chat_id
        {
            let toast = adw::Toast::builder()
                .title(glib::markup_escape_text(&format!(
                    "A task needs your OK: {summary}"
                )))
                .button_label("_Review")
                .action_name("app.open-chat")
                .action_target(&self.chat_id.to_variant())
                .priority(adw::ToastPriority::High)
                .build();
            w.toasts.add_toast(toast);
            return;
        }
        // Cards need an Rc for their buttons; the window keeps this reply in one.
        let me = self.win.upgrade().and_then(|w| w.st.borrow().reply.clone());
        if let Some(me) = me {
            me.confirm_card(request, &plain_step(_name, summary).0, detail, can_always);
        }
    }

    fn on_finished(&self, error: &str) {
        if !error.is_empty() {
            self.error(error);
        }
        self.finish(true);
        if let Some(w) = self.win.upgrade() {
            w.turn_finished(&self.chat_id, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_in_plain_words() {
        let home = std::env::var("HOME").unwrap();
        let dir = std::path::Path::new(&home).join(".cache/panda-test-plain/Zrzuty ekranu");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a b.png"), "x").unwrap();
        let (label, t) = plain_step(
            "edit_image",
            "Edit ~/.cache/panda-test-plain/Zrzuty ekranu/a b.png",
        );
        assert_eq!(label, "Edit a b.png");
        assert!(matches!(&t[..], [Target::File(p)] if p.ends_with("Zrzuty ekranu/a b.png")));
        let (label, _) = plain_step(
            "edit_image",
            "Edit ~/.cache/panda-test-plain/Zrzuty ekranu/*.png",
        );
        assert_eq!(label, "Edit Zrzuty ekranu/*.png");
        let (label, t) = plain_step("read_web_page", "Read https://example.com/a");
        assert_eq!(label, "Read a web page");
        assert!(matches!(&t[..], [Target::Link(u)] if u == "https://example.com/a"));
        assert_eq!(
            plain_step("set_volume", "Set volume to 30%").0,
            "Set volume to 30%"
        );
        std::fs::remove_dir_all(std::path::Path::new(&home).join(".cache/panda-test-plain"))
            .unwrap();
    }
}
