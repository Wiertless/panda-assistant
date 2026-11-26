//! The quick bar on Super+Space: one line to type in, the answer under it, Esc
//! closes it. What was on screen (app, window title, selected text) goes along
//! with the question. Anything longer, or an action that asks first, moves to
//! the full window with the same chat.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use assistant_core::store;

use crate::core::{Core, Turn};
use crate::window::CONTEXT_END;

/// Opens the chat in the full window, then asks the follow-up there (if any).
pub type OpenChat = Rc<dyn Fn(&str, &str)>;

thread_local! {
    static OPEN: RefCell<Option<adw::Window>> = const { RefCell::new(None) };
}

/// "Perfume X – Helium" → "Perfume X": browsers end titles with their name, and the small
/// model took it for part of the page ("Riccardo is Helium perfume").
fn without_app<'a>(title: &'a str, app: &str) -> &'a str {
    let lower = title.to_lowercase();
    for dash in [" – ", " — ", " - "] {
        let end = format!("{dash}{}", app.to_lowercase());
        if !app.is_empty() && lower.ends_with(&end) {
            return title[..title.len() - end.len()].trim_end();
        }
    }
    title
}

/// "[On screen: Firefox — “Title”. Selected text: «…»]" ahead of the question.
fn screen_note(core: &Core) -> String {
    let ctx = core.context().unwrap_or_default();
    let s = |k: &str| ctx[k].as_str().unwrap_or("").trim().to_string();
    let (app, title, selection) = (s("app"), s("title"), s("selection"));
    let title = without_app(&title, &app);
    let mut note = vec![];
    if !app.is_empty() || !title.is_empty() {
        note.push(format!("{app} — “{title}”"));
    }
    if !selection.is_empty() {
        note.push(format!(
            "Selected text: «{}»",
            selection.chars().take(4000).collect::<String>()
        ));
    }
    if note.is_empty() {
        return String::new();
    }
    format!(
        "{}{}{CONTEXT_END}",
        assistant_core::agent::CONTEXT_MARK,
        note.join(". ")
    )
}

struct Bar {
    win: adw::Window,
    /// Clips the answer area; a spring animates its height open (with a bounce).
    unfold: gtk::ScrolledWindow,
    below: gtk::Box,
    pill: gtk::Box,
    opened: std::cell::Cell<bool>,
    springing: Rc<std::cell::Cell<bool>>,
    entry: gtk::Entry,
    answer: gtk::Label,
    status: gtk::Label,
    scroll: gtk::ScrolledWindow,
    text: RefCell<String>,
    chat: String,
    open_full: OpenChat,
}

impl Bar {
    /// Spring the answer area from its height now to what its content needs.
    fn unfold(&self) {
        self.opened.set(true);
        self.pill.add_css_class("open");
        if self.springing.get() {
            return;
        }
        let width = self.unfold.width().max(self.pill.width() - 30).max(1);
        let from = self.unfold.height() as f64;
        let target = self
            .below
            .measure(gtk::Orientation::Vertical, width)
            .1
            .min(470) as f64;
        if target - from < 8.0 {
            return;
        }
        self.springing.set(true);
        let clip = self.unfold.clone();
        let set = adw::CallbackAnimationTarget::new(move |v| {
            let h = v.max(0.0) as i32;
            clip.set_min_content_height(h);
            clip.set_max_content_height(h);
        });
        let spring = adw::SpringAnimation::new(
            &self.unfold,
            from,
            target,
            adw::SpringParams::new(0.425, 1.0, 320.0),
            set,
        );
        let (clip, busy) = (self.unfold.clone(), self.springing.clone());
        // Then follow the answer as it streams in.
        spring.connect_done(move |_| {
            busy.set(false);
            clip.set_min_content_height(-1);
            clip.set_max_content_height(-1);
        });
        spring.play();
    }
    fn show_answer(&self) {
        self.unfold();
        self.scroll.set_visible(true);
        self.answer
            .set_markup(&crate::markup::to_pango(self.text.borrow().trim()));
    }
    fn to_full(&self, follow_up: &str) {
        (self.open_full)(&self.chat, follow_up);
        self.win.close();
    }
}

impl Turn for Bar {
    fn on_text(&self, text: &str) {
        self.text.borrow_mut().push_str(text);
        self.status.set_visible(false);
        self.show_answer();
    }
    fn on_reasoning(&self, _: &str) {
        self.status.set_label("Thinking…");
    }
    fn on_tool_started(&self, _: &str, summary: &str) {
        self.status.set_visible(true);
        self.status.set_label(&format!("{summary}…"));
    }
    fn on_tool_finished(&self, _: &str, _: &str, _: bool) {}
    fn on_notice(&self, _: &str) {}
    fn on_progress(&self, text: &str) {
        self.status.set_label(text);
    }
    fn on_handoff(&self, model: &str) {
        self.status.set_label(&format!("Asking {model}…"));
    }
    fn on_compacting(&self) {}
    fn on_retract(&self) {
        self.text.borrow_mut().clear();
    }
    // Allow/Cancel needs the full window's card.
    fn on_confirm(&self, _: u32, _: &str, _: &str, _: &str, _: bool) {
        self.to_full("");
    }
    fn on_finished(&self, error: &str) {
        self.status.set_visible(!error.is_empty());
        self.status.set_label(error);
        self.show_answer();
        self.entry.set_text("");
        self.entry
            .set_placeholder_text(Some("Follow up in the full window…"));
        self.entry.set_sensitive(true);
        self.entry.grab_focus();
    }
}

pub fn show(app: &adw::Application, open_full: OpenChat) {
    // Pressed again while open: close it, like a toggle.
    if let Some(w) = OPEN.with(|o| o.borrow_mut().take()) {
        w.close();
        return;
    }
    let core = Core::new();
    let note = screen_note(&core);

    // The app's composer pill; the answer unfolds inside it.
    let entry = gtk::Entry::builder()
        .placeholder_text(if note.contains("Selected text") {
            "Ask about the selected text…"
        } else {
            "Ask Assistant…"
        })
        .hexpand(true)
        .build();
    let more = gtk::Button::builder()
        .icon_name("view-dual-symbolic")
        .tooltip_text("Continue in the Full Window")
        .visible(false)
        .build();
    more.add_css_class("flat");
    more.add_css_class("circular");
    let send = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Send")
        .build();
    send.add_css_class("suggested-action");
    send.add_css_class("circular");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&more);
    row.append(&send);

    let status = gtk::Label::builder().xalign(0.0).visible(false).build();
    status.add_css_class("dim-label");
    let answer = gtk::Label::builder()
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&answer)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .visible(false)
        .build();
    let below = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(8)
        .margin_bottom(12)
        .margin_end(13)
        .build();
    below.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    below.append(&status);
    below.append(&scroll);
    let unfold = gtk::ScrolledWindow::builder()
        .child(&below)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .min_content_height(0)
        .max_content_height(0)
        .build();

    let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pill.add_css_class("composer");
    pill.add_css_class("popup-pill");
    pill.append(&row);
    pill.append(&unfold);
    // Room for the pill's shadow inside the transparent window.
    let frame = gtk::Box::builder()
        .margin_top(16)
        .margin_bottom(24)
        .margin_start(16)
        .margin_end(16)
        .build();
    frame.append(&pill);

    let win = adw::Window::builder()
        .application(app)
        .content(&frame)
        .default_width(720)
        // libadwaita windows default to at least 200px tall: the pill would stretch to it.
        .height_request(1)
        .resizable(false)
        .title("Assistant")
        .build();
    win.add_css_class("popup");

    let bar = Rc::new(Bar {
        win: win.clone(),
        unfold: unfold.clone(),
        below: below.clone(),
        pill: pill.clone(),
        opened: Default::default(),
        springing: Default::default(),
        entry: entry.clone(),
        answer,
        status,
        scroll,
        text: RefCell::default(),
        chat: store::new_chat_id(),
        open_full,
    });

    let keys = gtk::EventControllerKey::new();
    let w = win.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);

    let b = bar.clone();
    more.connect_clicked(move |_| b.to_full(""));
    let e = entry.clone();
    send.connect_clicked(move |_| e.emit_activate());

    let (b, c, m) = (bar.clone(), core.clone(), more.clone());
    let note = RefCell::new(note);
    entry.connect_activate(move |e| {
        let q = e.text().trim().to_string();
        if q.is_empty() {
            return;
        }
        // One question per bar; follow-ups go to the full window.
        if b.opened.get() {
            b.to_full(&q);
            return;
        }
        e.set_sensitive(false);
        m.set_visible(true);
        b.status.set_visible(true);
        b.status.set_label("Thinking…");
        b.unfold();
        let prompt = format!("{}{q}", note.take());
        if let Err(err) = c.ask(&b.chat, &prompt, &[], b.clone()) {
            b.status.set_label(&err);
        }
    });

    // Keep the bar and its service link alive while it's open.
    let keep = (bar, core);
    win.connect_close_request(move |_| {
        let _ = &keep;
        OPEN.with(|o| o.borrow_mut().take());
        glib::Propagation::Proceed
    });
    OPEN.with(|o| *o.borrow_mut() = Some(win.clone()));
    win.present();
    entry.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::without_app;

    #[test]
    fn titles_lose_the_app_name() {
        assert_eq!(without_app("Riccardo – Helium", "Helium"), "Riccardo");
        assert_eq!(without_app("Notes - gedit", "gedit"), "Notes");
        assert_eq!(without_app("Helium docs", "Helium"), "Helium docs");
        assert_eq!(without_app("Title", ""), "Title");
    }
}
