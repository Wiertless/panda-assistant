//! One assistant turn in the window: its name and model, a status
//! pill while it works, the answer as it streams, action pills, Allow/Cancel
//! cards, and a copy button at the end.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gio;

use crate::core::Turn;
use crate::markup::to_pango;
use crate::window::{APP_NAME, Window};

fn tool_icon(name: &str) -> &'static str {
    match name {
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
        "look_at_image" => "image-x-generic-symbolic",
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

/// Notification text: markdown marks removed, cut at a word.
fn plain(markdown: &str) -> String {
    assistant_core::search::plain(markdown, 400)
}

#[cfg(feature = "adw-1-8")]
fn spinner() -> gtk::Widget {
    adw::Spinner::builder()
        .halign(gtk::Align::Start)
        .width_request(24)
        .height_request(24)
        .build()
        .upcast()
}

#[cfg(not(feature = "adw-1-8"))]
fn spinner() -> gtk::Widget {
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
}

pub struct Reply {
    win: Weak<Window>,
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
            .label(
                model
                    .map(String::from)
                    .unwrap_or_else(|| win.st.borrow().model.clone()),
            )
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
                    if answer == "deny" {
                        "cancelled"
                    } else {
                        "allowed"
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

impl Turn for Reply {
    fn on_text(&self, s: &str) {
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

    /// The model's reasoning isn't shown, only that it's thinking.
    fn on_reasoning(&self, _: &str) {
        self.spin_label.set_label("Thinking…");
    }

    fn on_tool_started(&self, name: &str, summary: &str) {
        self.set_summary(name, summary);
        self.spin_label.set_label(&format!("{summary}…"));
    }

    fn on_tool_finished(&self, name: &str, result: &str, ok: bool) {
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
            let tooltip: String = result.chars().take(600).collect();
            self.pill(
                tool_icon(name),
                if short { result } else { &summary },
                false,
                (!short).then_some(tooltip.as_str()),
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
        self.before_spinner(&pill);
        let mut i = self.inner.borrow_mut();
        i.label = None; // text after an action starts a new message
        i.answer.push('\n');
        drop(i);
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
        self.before_spinner(&self.pill(
            "go-next-symbolic",
            &format!("Handed to {model}"),
            false,
            None,
        ));
        self.model_label.set_label(model);
        self.spin_label.set_label(&format!("Loading {model}…"));
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
        // Cards need an Rc for their buttons; the window keeps this reply in one.
        let me = self.win.upgrade().and_then(|w| w.st.borrow().reply.clone());
        if let Some(me) = me {
            me.confirm_card(request, summary, detail, can_always);
        }
    }

    fn on_finished(&self, error: &str) {
        if !error.is_empty() {
            self.error(error);
        }
        self.finish(true);
        if let Some(w) = self.win.upgrade() {
            w.turn_finished(error);
        }
    }
}
