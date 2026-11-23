//! The chat window, laid out after the GNOME HIG: chats beside the
//! conversation (two pages on narrow windows, with libadwaita's back swipe),
//! the model as the window subtitle, a floating composer.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};

use assistant_core::flm::{Model, pick_vision_model};
use assistant_core::store;
use assistant_core::tools::system_actions::image_data_url;

use crate::config;
use crate::core::{Core, Turn};
use crate::reply::Reply;

pub const APP_ID: &str = "io.github.wiertless.PandaAssistant";
pub const APP_NAME: &str = "Assistant";
use assistant_core::flm::DEFAULT_MODEL;
const STALE: Duration = Duration::from_secs(30 * 60);
const MAX_ATTACHMENTS: usize = 4;
const IMAGE_TYPES: [&str; 6] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
    "image/tiff",
];
const SUGGESTIONS: [(&str, &str); 6] = [
    ("How much memory is free?", "How much memory is free?"),
    ("Turn on Night Light", "Turn on Night Light"),
    ("What's on my screen?", "What's on my screen?"),
    ("Fedora 45 news", "Search the web for Fedora 45 news"),
    ("Open Downloads", "Open my Downloads folder"),
    ("Bigger text", "Make the text a bit bigger"),
];

pub fn greeting() -> &'static str {
    let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Hello, night owl",
    }
}

pub fn relative_time(stamp: f64) -> String {
    let (Ok(then), Ok(now)) = (
        glib::DateTime::from_unix_local(stamp as i64),
        glib::DateTime::now_local(),
    ) else {
        return String::new();
    };
    let days =
        (now.ymd() != then.ymd()) as i64 * ((now.to_unix() - then.to_unix()) / 86400 + 1).max(1);
    let f = |s: &str| then.format(s).map(|x| x.to_string()).unwrap_or_default();
    match days {
        0 => f("%H:%M"),
        1 => "Yesterday".into(),
        2..=6 => f("%A"),
        _ if then.year() == now.year() => f("%-d %b"),
        _ => f("%-d %b %Y"),
    }
}

/// How a saved user message reads in the chat: skill runs as /name.
/// Ends the screen context a pop-up question starts with (see `Window::popup`).
pub use assistant_core::agent::CONTEXT_END;

pub fn shown_user_text(content: &str) -> String {
    let content = match content.split_once(CONTEXT_END) {
        Some((head, rest)) if head.starts_with(assistant_core::agent::CONTEXT_MARK) => rest,
        _ => content,
    };
    let head = "Follow my saved skill “";
    if let Some(rest) = content.strip_prefix(head) {
        let title = rest.split('”').next().unwrap_or("");
        let details = content
            .rsplit_once("\n\nDetails: ")
            .map(|(_, d)| d)
            .unwrap_or("");
        return format!(
            "/{}{}",
            store::slugify(title),
            if details.is_empty() {
                String::new()
            } else {
                format!(" {details}")
            }
        );
    }
    content.to_string()
}

pub struct State {
    pub model: String,
    pub models: Vec<Model>,
    pub busy: bool,
    pub chat_id: String,
    pub last_turn: Option<Instant>,
    pub attachments: Vec<(gdk::Texture, String)>,
    pub state: String,
    pub reply: Option<Rc<Reply>>,
    pub text_scale: f64,
    pub smarter_choice: String,
    pub ctx_len: i64,
    pub unload_after: i64,
    pub web_enabled: bool,
    pub memory_enabled: bool,
    pub messages: Vec<Value>,
    stepped_aside: bool,
    /// Messages typed while an answer was coming, sent one by one after it.
    queue: Vec<Queued>,
}

/// A message waiting for the current answer to finish.
struct Queued {
    text: String,
    prompt: String,
    images: Vec<String>,
    thumbs: Vec<gdk::Texture>,
    /// Its bubble, shown dimmed with a "Queued" line until it's sent.
    bubble: gtk::Box,
    status: gtk::Box,
}

pub struct Window {
    pub win: adw::ApplicationWindow,
    pub app: adw::Application,
    pub core: Rc<Core>,
    pub st: RefCell<State>,
    title: adw::WindowTitle,
    banner: adw::Banner,
    stack: gtk::Stack,
    pub messages: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    send_btn: gtk::Button,
    think_btn: gtk::ToggleButton,
    attach_box: gtk::Box,
    attach_revealer: gtk::Revealer,
    pub split: adw::NavigationSplitView,
    search: gtk::SearchEntry,
    chat_list: gtk::ListBox,
    chat_stack: gtk::Stack,
    no_chats: adw::StatusPage,
    greeting: gtk::Label,
    toasts: adw::ToastOverlay,
    model_menu: gio::Menu,
    model_action: gio::SimpleAction,
    unload_action: gio::SimpleAction,
    zoom_css: gtk::CssProvider,
    loading: Rc<Cell<bool>>,
}

fn button(icon: &str, tip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .valign(gtk::Align::Center)
        .build()
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let cfg = config::load();
        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title(APP_NAME)
            .default_width(config::int_or(&cfg, "width", 600) as i32)
            .default_height(config::int_or(&cfg, "height", 760) as i32)
            .width_request(360)
            .height_request(294)
            .build();
        if config::bool_or(&cfg, "maximized", false) {
            win.maximize();
        }
        let model = config::str_or(&cfg, "model", DEFAULT_MODEL);

        // Header: new chat at the start, primary menu at the end, model as subtitle.
        let title = adw::WindowTitle::new(APP_NAME, &model);
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let new_btn = button("list-add-symbolic", "New Chat");
        new_btn.set_action_name(Some("win.new-chat"));
        header.pack_start(&new_btn);
        let model_menu = gio::Menu::new();
        let menu = gio::Menu::new();
        menu.append_submenu(Some("_Model"), &model_menu);
        let mem = gio::Menu::new();
        mem.append(Some("_Unload Model"), Some("win.unload"));
        mem.append(Some("_Restart Assistant"), Some("win.restart"));
        menu.append_section(None, &mem);
        let tail = gio::Menu::new();
        tail.append(Some("_Preferences"), Some("win.preferences"));
        tail.append(Some("_Keyboard Shortcuts"), Some("win.show-help-overlay"));
        tail.append(Some(&format!("_About {APP_NAME}")), Some("app.about"));
        menu.append_section(None, &tail);
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .tooltip_text("Main Menu")
                .primary(true)
                .build(),
        );

        // Conversation.
        let messages = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();
        messages.add_css_class("conversation");
        let clamp = adw::Clamp::builder()
            .maximum_size(720)
            .tightening_threshold(560)
            .child(&messages)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let (start_page, greeting_label, chips) = build_start_page();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&start_page, Some("empty"));
        stack.add_named(&scroller, Some("chat"));

        // Composer: a rounded floating bar, with attached pictures above it.
        let entry = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("Message, or type / for skills…")
            .build();
        let attach_btn = button("mail-attachment-symbolic", "Attach Image");
        attach_btn.add_css_class("flat");
        attach_btn.add_css_class("circular");
        attach_btn.set_action_name(Some("win.attach"));
        let send_btn = button("go-up-symbolic", "Send");
        send_btn.add_css_class("suggested-action");
        send_btn.add_css_class("circular");
        // Thinking on: the models reason before every answer and step (shown above
        // the answer), slower; off: they answer straight away.
        let think_btn = gtk::ToggleButton::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text("Think First (slower, shows its reasoning)")
            .valign(gtk::Align::Center)
            .active(config::bool_or(&cfg, "thinking", false))
            .build();
        think_btn.add_css_class("flat");
        think_btn.add_css_class("circular");
        think_btn.add_css_class("think-toggle");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&think_btn);
        row.append(&attach_btn);
        row.append(&send_btn);
        let attach_box = gtk::Box::builder()
            .spacing(8)
            .margin_top(6)
            .margin_bottom(2)
            .build();
        let attach_revealer = gtk::Revealer::builder()
            .child(&attach_box)
            .reveal_child(false)
            .build();
        let composer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        composer.add_css_class("composer");
        composer.append(&attach_revealer);
        composer.append(&row);
        let composer_clamp = adw::Clamp::builder()
            .maximum_size(720)
            .tightening_threshold(560)
            .child(&composer)
            .margin_top(6)
            .margin_bottom(16)
            .margin_start(12)
            .margin_end(12)
            .build();

        let banner = adw::Banner::builder().revealed(false).build();
        banner.set_action_name(Some("win.restart"));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&banner);
        content.append(&stack);
        let view = adw::ToolbarView::builder().content(&content).build();
        view.add_top_bar(&header);
        view.add_bottom_bar(&composer_clamp);
        view.set_bottom_bar_style(adw::ToolbarStyle::Flat);

        // Chats beside the conversation; on narrow windows they're two pages, and
        // libadwaita's back swipe (touchpad or touchscreen) returns to the list.
        let (sidebar, search, chat_list, chat_stack, no_chats) = build_sidebar();
        let split = adw::NavigationSplitView::builder()
            .min_sidebar_width(240.0)
            .max_sidebar_width(320.0)
            .show_content(true)
            .build();
        split.set_content(Some(&adw::NavigationPage::with_tag(
            &view, APP_NAME, "chat",
        )));
        split.set_sidebar(Some(&adw::NavigationPage::with_tag(
            &sidebar, "Chats", "chats",
        )));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        win.set_content(Some(&toasts));
        let narrow =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 680sp").unwrap());
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        win.add_breakpoint(narrow);

        let zoom_css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &zoom_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let model_action = gio::SimpleAction::new_stateful(
            "model",
            Some(glib::VariantTy::STRING),
            &model.to_variant(),
        );
        let unload_action = gio::SimpleAction::new("unload", None);
        unload_action.set_enabled(false);

        let this = Rc::new(Self {
            st: RefCell::new(State {
                model,
                models: vec![],
                busy: false,
                chat_id: store::new_chat_id(),
                last_turn: None,
                attachments: vec![],
                state: "Not loaded".into(),
                reply: None,
                text_scale: cfg["text_scale"].as_f64().unwrap_or(1.0),
                smarter_choice: config::str_or(&cfg, "smarter_model", "auto"),
                ctx_len: config::int_or(
                    &cfg,
                    "ctx_len",
                    assistant_core::flm::DEFAULT_CHAT_CTX_LEN as i64,
                ),
                unload_after: config::int_or(
                    &cfg,
                    "unload_after_minutes",
                    assistant_core::service::DEFAULT_UNLOAD_MINUTES as i64,
                ),
                web_enabled: config::bool_or(&cfg, "web_search", true),
                memory_enabled: config::bool_or(&cfg, "memory", true),
                messages: vec![],
                stepped_aside: false,
                queue: vec![],
            }),
            win,
            app: app.clone(),
            core: Core::new(),
            title,
            banner,
            stack,
            messages,
            scroller,
            entry,
            send_btn,
            think_btn,
            attach_box,
            attach_revealer,
            split,
            search,
            chat_list,
            chat_stack,
            no_chats,
            greeting: greeting_label,
            toasts,
            model_menu,
            model_action,
            unload_action,
            zoom_css,
            loading: Rc::new(Cell::new(false)),
        });
        this.connect(&chips);
        let scale = this.st.borrow().text_scale;
        this.set_text_scale(scale, false);
        this.refresh_chats();
        this.entry.grab_focus();
        let w = Rc::downgrade(&this);
        glib::idle_add_local_once(move || {
            if let Some(w) = w.upgrade() {
                w.load_models()
            }
        });
        this
    }

    fn connect(self: &Rc<Self>, chips: &[gtk::Button]) {
        let w = Rc::downgrade(self);
        let weak = move || w.upgrade();
        macro_rules! on {
            ($body:expr) => {{
                let weak = weak.clone();
                move || {
                    if let Some(s) = weak() {
                        $body(&s)
                    }
                }
            }};
        }
        for (chip, (_, prompt)) in chips.iter().zip(SUGGESTIONS) {
            let f = on!(|s: &Rc<Self>| s.ask(prompt));
            chip.connect_clicked(move |_| f());
        }
        let f = on!(|s: &Rc<Self>| s.send());
        self.entry.connect_activate(move |_| f());
        // Start loading the model while the question is typed.
        let f = on!(|s: &Rc<Self>| s.preload());
        self.entry.connect_changed(move |e| {
            if !e.text().trim().is_empty() {
                f()
            }
        });
        // Pasting a picture attaches it (the signal lives on the inner GtkText).
        if let Some(text) = self.entry.delegate().and_downcast::<gtk::Text>() {
            let w = weak.clone();
            text.connect_paste_clipboard(move |t| {
                if let Some(s) = w() {
                    s.on_paste(t)
                }
            });
        }
        let f = on!(|s: &Rc<Self>| if s.st.borrow().busy {
            s.stop()
        } else {
            s.send()
        });
        self.send_btn.connect_clicked(move |_| f());
        let w = weak.clone();
        self.think_btn.connect_toggled(move |b| {
            config::update(&[("thinking", json!(b.is_active()))]);
            if let Some(s) = w() {
                s.core.reload();
                s.toast(if b.is_active() {
                    "Thinking first: slower, and you can see its reasoning"
                } else {
                    "Answering straight away"
                });
            }
        });
        let w = weak.clone();
        self.search.connect_search_changed(move |_| {
            if let Some(s) = w() {
                s.refresh_chats()
            }
        });
        self.search.connect_stop_search(|e| e.set_text(""));
        let w = weak.clone();
        self.chat_list.connect_row_activated(move |_, row| {
            if let Some(s) = w() {
                let id: String = row.widget_name().into();
                s.open_chat(&id);
            }
        });
        let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = weak.clone();
        drop_target.connect_drop(move |_, value, _, _| {
            let Some(s) = w() else { return false };
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            if !s.can_attach() {
                return false;
            }
            list.files()
                .iter()
                .map(|f| s.attach_file(f))
                .fold(false, |a, b| a | b)
        });
        self.win.add_controller(drop_target);
        self.setup_zoom();

        // Window actions.
        let actions: [(&str, Box<dyn Fn()>); 11] = [
            ("restart", Box::new(on!(|s: &Rc<Self>| s.restart()))),
            ("new-chat", Box::new(on!(|s: &Rc<Self>| s.new_chat()))),
            (
                "focus-entry",
                Box::new(on!(|s: &Rc<Self>| {
                    s.entry.grab_focus();
                })),
            ),
            ("stop", Box::new(on!(|s: &Rc<Self>| s.stop()))),
            (
                "show-help-overlay",
                Box::new(on!(|s: &Rc<Self>| crate::prefs::show_shortcuts(s))),
            ),
            (
                "preferences",
                Box::new(on!(|s: &Rc<Self>| crate::prefs::show_preferences(s))),
            ),
            ("attach", Box::new(on!(|s: &Rc<Self>| s.choose_image()))),
            (
                "search-chats",
                Box::new(on!(|s: &Rc<Self>| {
                    s.split.set_show_content(false);
                    s.search.grab_focus();
                })),
            ),
            (
                "zoom-in",
                Box::new(on!(|s: &Rc<Self>| {
                    let v = s.st.borrow().text_scale + 0.1;
                    s.set_text_scale(v, true)
                })),
            ),
            (
                "zoom-out",
                Box::new(on!(|s: &Rc<Self>| {
                    let v = s.st.borrow().text_scale - 0.1;
                    s.set_text_scale(v, true)
                })),
            ),
            (
                "zoom-reset",
                Box::new(on!(|s: &Rc<Self>| s.set_text_scale(1.0, true))),
            ),
        ];
        for (name, f) in actions {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| f());
            self.win.add_action(&a);
        }
        let f = on!(|s: &Rc<Self>| s.unload(true));
        self.unload_action.connect_activate(move |_, _| f());
        self.win.add_action(&self.unload_action);
        let w = weak.clone();
        self.model_action.connect_activate(move |a, v| {
            if let (Some(s), Some(v)) = (w(), v) {
                s.on_model(a, v)
            }
        });
        self.win.add_action(&self.model_action);

        // What the service asks of the window.
        let mut hooks = self.core.hooks.borrow_mut();
        let w = weak.clone();
        hooks.state = Some(Box::new(move |_, state| {
            if let Some(s) = w() {
                s.set_state(state)
            }
        }));
        hooks.clipboard = Some(Box::new(|text| {
            if let Some(d) = gdk::Display::default() {
                d.clipboard().set_text(text)
            }
        }));
        let w = weak.clone();
        hooks.step_aside = Some(Box::new(move || {
            if let Some(s) = w() {
                // Minimize so the window behind becomes the focused one (screenshots, reading a window).
                let aside = s.in_view();
                s.st.borrow_mut().stepped_aside = aside;
                if aside {
                    s.win.minimize()
                }
            }
        }));
        let w = weak.clone();
        hooks.step_back = Some(Box::new(move || {
            if let Some(s) = w()
                && std::mem::take(&mut s.st.borrow_mut().stepped_aside)
            {
                s.win.present()
            }
        }));
        drop(hooks);
        let w = weak.clone();
        self.win.connect_close_request(move |win| {
            if let Some(s) = w() {
                let maximized = win.is_maximized();
                let mut changes = vec![("maximized", json!(maximized))];
                if !maximized {
                    let (wd, ht) = win.default_size();
                    changes.push(("width", json!(wd)));
                    changes.push(("height", json!(ht)));
                }
                config::update(&changes);
                s.stop();
            }
            glib::Propagation::Proceed
        });
    }

    pub fn in_view(&self) -> bool {
        self.win.is_visible() && self.win.is_active()
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(
            adw::Toast::builder()
                .title(glib::markup_escape_text(text))
                .timeout(3)
                .build(),
        );
    }

    pub fn scroll_down(&self) {
        let adj = self.scroller.vadjustment();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            adj.set_value(adj.upper() - adj.page_size())
        });
    }

    /// Add a row to the conversation with a short slide-in.
    pub fn append_animated(&self, widget: &impl IsA<gtk::Widget>) {
        let rev = gtk::Revealer::builder()
            .child(widget)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(220)
            .build();
        self.messages.append(&rev);
        let r = rev.clone();
        glib::idle_add_local_once(move || r.set_reveal_child(true));
        self.scroll_down();
    }

    // -- text size: pinch, Ctrl+scroll, Ctrl+plus/minus -------------------------
    fn setup_zoom(self: &Rc<Self>) {
        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let (w, s0) = (Rc::downgrade(self), start.clone());
        pinch.connect_begin(move |_, _| {
            if let Some(s) = w.upgrade() {
                s0.set(s.st.borrow().text_scale)
            }
        });
        let (w, s0) = (Rc::downgrade(self), start);
        pinch.connect_scale_changed(move |_, scale| {
            if let Some(s) = w.upgrade() {
                s.set_text_scale(s0.get() * scale, false)
            }
        });
        let w = Rc::downgrade(self);
        pinch.connect_end(move |_, _| {
            if let Some(s) = w.upgrade() {
                let v = s.st.borrow().text_scale;
                s.set_text_scale(v, true)
            }
        });
        self.scroller.add_controller(pinch);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |ctrl, _dx, dy| {
            let Some(s) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !ctrl
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let v = s.st.borrow().text_scale - dy * 0.05;
            s.set_text_scale(v, true);
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);
    }

    pub fn set_text_scale(&self, scale: f64, save: bool) {
        let scale = (scale.clamp(0.7, 2.0) * 100.0).round() / 100.0;
        self.st.borrow_mut().text_scale = scale;
        self.zoom_css.load_from_string(&format!(
            ".conversation {{ font-size: {:.0}%; }}",
            scale * 100.0
        ));
        if save {
            config::update(&[("text_scale", json!(scale))]);
        }
    }

    // -- chats sidebar ------------------------------------------------------------
    pub fn refresh_chats(&self) {
        let query = self.search.text().trim().to_string();
        while let Some(row) = self.chat_list.row_at_index(0) {
            self.chat_list.remove(&row);
        }
        let chats = store::list_chats(&query);
        let current = self.st.borrow().chat_id.clone();
        for chat in &chats {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&chat.id);
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let texts = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .hexpand(true)
                .valign(gtk::Align::Center)
                .build();
            texts.append(
                &gtk::Label::builder()
                    .label(&chat.title)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            let when = gtk::Label::builder()
                .label(relative_time(chat.updated))
                .xalign(0.0)
                .build();
            when.add_css_class("dim-label");
            when.add_css_class("chat-row-time");
            texts.append(&when);
            let delete = button("user-trash-symbolic", "Delete Chat");
            delete.add_css_class("flat");
            delete.add_css_class("circular");
            let id = chat.id.clone();
            let win = self.win.clone();
            delete.connect_clicked(move |_| {
                let _ = WidgetExt::activate_action(&win, "win.delete-chat", Some(&id.to_variant()));
            });
            bx.append(&texts);
            bx.append(&delete);
            row.set_child(Some(&bx));
            self.chat_list.append(&row);
            if chat.id == current {
                self.chat_list.select_row(Some(&row));
            }
        }
        if chats.is_empty() {
            self.no_chats.set_title(if query.is_empty() {
                "No Chats"
            } else {
                "No Results"
            });
            self.no_chats.set_description(if query.is_empty() {
                Some("Your conversations are saved here")
            } else {
                None
            });
            self.chat_stack.set_visible_child_name("empty");
        } else {
            self.chat_stack.set_visible_child_name("list");
        }
    }

    pub fn delete_chat(self: &Rc<Self>, id: &str) {
        store::delete_chat(id);
        if id == self.st.borrow().chat_id {
            self.new_chat();
        }
        self.refresh_chats();
        self.toast("Chat deleted");
    }

    pub fn open_chat(self: &Rc<Self>, id: &str) {
        if self.st.borrow().busy || id == self.st.borrow().chat_id {
            self.split.set_show_content(true);
            return;
        }
        // A chat started from search is saved when its answer ends; until then
        // it only exists in the service, so open it empty and follow the turn.
        let running = || {
            self.core
                .status()
                .is_ok_and(|s| s["busy"] == true && s["chat_id"] == id)
        };
        let Some(data) =
            store::load_chat(id).or_else(|| running().then(|| serde_json::json!({"messages": []})))
        else {
            self.toast("Couldn't open that chat");
            self.refresh_chats();
            return;
        };
        self.new_chat();
        let messages = data["messages"].as_array().cloned().unwrap_or_default();
        self.st.borrow_mut().chat_id = id.to_string();
        let model = data["model"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| self.st.borrow().model.clone());
        self.render_history(&messages, &model);
        self.st.borrow_mut().messages = messages;
        self.stack.set_visible_child_name("chat");
        self.split.set_show_content(true);
        self.refresh_chats();
        self.follow_running_turn();
    }

    /// If the service is still answering in this chat (asked from search), show
    /// the rest of the answer here, and any Allow/Cancel it waits on.
    fn follow_running_turn(self: &Rc<Self>) {
        let chat_id = self.st.borrow().chat_id.clone();
        let Ok(status) = self.core.status() else {
            return;
        };
        if status["busy"] != true || status["chat_id"] != chat_id.as_str() {
            return;
        }
        let pending = self.core.pending(&chat_id).unwrap_or(Value::Null);
        self.set_busy(true);
        self.stack.set_visible_child_name("chat");
        if let Some(q) = status["question"].as_str().filter(|q| !q.is_empty()) {
            self.add_user(&shown_user_text(q), &[], true);
        }
        let reply = Reply::new(self, true, None);
        self.st.borrow_mut().reply = Some(reply.clone());
        self.core.watch(&chat_id, reply.clone());
        for p in pending.as_array().into_iter().flatten() {
            reply.on_confirm(
                p["request"].as_u64().unwrap_or(0) as u32,
                p["name"].as_str().unwrap_or(""),
                p["summary"].as_str().unwrap_or(""),
                p["detail"].as_str().unwrap_or(""),
                p["can_always"].as_bool().unwrap_or(false),
            );
        }
    }

    /// Draw a saved chat: user bubbles, answers and finished action pills.
    fn render_history(self: &Rc<Self>, messages: &[Value], model: &str) {
        let results: std::collections::HashMap<String, String> = messages
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| {
                (
                    m["tool_call_id"].as_str().unwrap_or("").to_string(),
                    m["content"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let mut reply: Option<Rc<Reply>> = None;
        for m in messages {
            let content = store::plain_text(&m["content"]);
            match m["role"].as_str() {
                Some("user") => {
                    if content == "(Image from the tool.)" {
                        continue;
                    }
                    if let Some(r) = reply.take() {
                        r.finish(false);
                    }
                    self.add_user(&shown_user_text(&content), &[], false);
                }
                Some("assistant") => {
                    let r = reply.get_or_insert_with(|| Reply::new(self, false, Some(model)));
                    for call in m["tool_calls"].as_array().into_iter().flatten() {
                        let f = &call["function"];
                        let name = f["name"].as_str().unwrap_or("");
                        let args: serde_json::Map<String, Value> =
                            serde_json::from_str(f["arguments"].as_str().unwrap_or("{}"))
                                .unwrap_or_default();
                        let summary = assistant_core::tools::all()
                            .into_iter()
                            .find(|t| t.name == name)
                            .map(|t| (t.summary)(&args))
                            .unwrap_or_else(|| name.to_string());
                        let result = results
                            .get(call["id"].as_str().unwrap_or(""))
                            .cloned()
                            .unwrap_or_default();
                        let ok = !(result.starts_with("Error")
                            || result == "The user declined this action.");
                        r.set_summary(name, &summary);
                        r.on_tool_finished(
                            name,
                            if ok {
                                &result
                            } else {
                                result.trim_start_matches("Error: ")
                            },
                            ok,
                        );
                    }
                    if !content.is_empty() {
                        r.add_text_block(&content);
                    }
                }
                _ => {}
            }
        }
        if let Some(r) = reply {
            r.finish(false);
        }
        self.scroll_down();
    }

    // -- pictures -------------------------------------------------------------------
    fn model_sees(&self, name: &str) -> bool {
        self.st
            .borrow()
            .models
            .iter()
            .any(|m| m.name == name && m.can_see())
    }

    fn can_attach(&self) -> bool {
        let st = self.st.borrow();
        if self.model_sees(&st.model) || pick_vision_model(&st.models, &st.model, None).is_some() {
            return true;
        }
        drop(st);
        self.toast("No installed model can look at images");
        false
    }

    fn choose_image(self: &Rc<Self>) {
        if !self.can_attach() {
            return;
        }
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        for t in IMAGE_TYPES {
            filter.add_mime_type(t);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Attach Image")
            .filters(&filters)
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        dialog.open_multiple(Some(&self.win), None::<&gio::Cancellable>, move |res| {
            let (Some(s), Ok(files)) = (w.upgrade(), res) else {
                return;
            };
            for i in 0..files.n_items() {
                if let Some(f) = files.item(i).and_downcast::<gio::File>() {
                    s.attach_file(&f);
                }
            }
        });
    }

    fn attach_file(&self, file: &gio::File) -> bool {
        let Some(path) = file.path() else {
            return false;
        };
        let ctype = file
            .query_info(
                "standard::content-type",
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            )
            .ok()
            .and_then(|i| i.content_type())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if !IMAGE_TYPES.contains(&ctype.as_str()) {
            self.toast(&format!(
                "{} isn't an image",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return false;
        }
        match (image_data_url(&path, 1024), gdk::Texture::from_file(file)) {
            (Ok(url), Ok(thumb)) => {
                self.add_attachment(thumb, url);
                true
            }
            (Err(e), _) => {
                self.toast(&e.0);
                false
            }
            (_, Err(e)) => {
                self.toast(e.message());
                false
            }
        }
    }

    fn add_attachment(&self, thumb: gdk::Texture, url: String) {
        if self.st.borrow().attachments.len() >= MAX_ATTACHMENTS {
            self.toast(&format!("Up to {MAX_ATTACHMENTS} images per message"));
            return;
        }
        self.st.borrow_mut().attachments.push((thumb, url));
        self.show_attachments();
        self.preload();
    }

    fn show_attachments(&self) {
        while let Some(c) = self.attach_box.first_child() {
            self.attach_box.remove(&c);
        }
        for (i, (thumb, _)) in self.st.borrow().attachments.iter().enumerate() {
            let pic = gtk::Picture::builder()
                .paintable(thumb)
                .content_fit(gtk::ContentFit::Cover)
                .width_request(56)
                .height_request(56)
                .overflow(gtk::Overflow::Hidden)
                .build();
            pic.add_css_class("attachment");
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text("Remove")
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .build();
            remove.add_css_class("circular");
            remove.add_css_class("osd");
            let win = self.win.clone();
            remove.connect_clicked(move |_| {
                let _ = WidgetExt::activate_action(
                    &win,
                    "win.remove-attachment",
                    Some(&(i as u32).to_variant()),
                );
            });
            let overlay = gtk::Overlay::builder().child(&pic).build();
            overlay.add_overlay(&remove);
            self.attach_box.append(&overlay);
        }
        self.attach_revealer
            .set_reveal_child(!self.st.borrow().attachments.is_empty());
    }

    pub fn remove_attachment(&self, index: usize) {
        {
            let mut st = self.st.borrow_mut();
            if index < st.attachments.len() {
                st.attachments.remove(index);
            }
        }
        self.show_attachments();
    }

    fn on_paste(self: &Rc<Self>, text: &gtk::Text) {
        let clipboard = text.clipboard();
        if !clipboard
            .formats()
            .contains_type(gdk::Texture::static_type())
        {
            return;
        }
        text.stop_signal_emission_by_name("paste-clipboard");
        if !self.can_attach() {
            return;
        }
        let w = Rc::downgrade(self);
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |res| {
            let (Some(s), Ok(Some(texture))) = (w.upgrade(), res) else {
                return;
            };
            let path: PathBuf = glib::user_cache_dir()
                .join("panda-assistant")
                .join("pasted.png");
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            if texture.save_to_png(&path).is_ok() {
                match image_data_url(&path, 1024) {
                    Ok(url) => s.add_attachment(texture, url),
                    Err(e) => s.toast(&e.0),
                }
            }
            let _ = std::fs::remove_file(path);
        });
    }

    // -- models and memory -------------------------------------------------------------
    fn load_models(self: &Rc<Self>) {
        match self.core.status() {
            Ok(st) => {
                // Until the user picks one, show the model the service chose for what's installed.
                if config::load()["model"].as_str().is_none()
                    && let Some(m) = st["model"].as_str().filter(|m| !m.is_empty())
                {
                    self.st.borrow_mut().model = m.to_string();
                }
                self.set_state(st["state"].as_str().unwrap_or("Not loaded"))
            }
            Err(e) => {
                self.banner.set_title(&e);
                self.banner.set_button_label(Some("Restart"));
                self.banner.set_revealed(true);
                return;
            }
        }
        let w = Rc::downgrade(self);
        self.core.models(move |res| {
            let Some(s) = w.upgrade() else { return };
            let list = match res {
                Ok(v) => v,
                Err(e) => {
                    s.toast(&format!("Couldn't list models: {e}"));
                    Value::Null
                }
            };
            let models: Vec<Model> = list
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| Model {
                    name: m["name"].as_str().unwrap_or("").into(),
                    installed: true,
                    labels: m["labels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l.as_str().map(String::from))
                        .collect(),
                })
                .collect();
            s.fill_models(models);
        });
    }

    fn fill_models(&self, mut models: Vec<Model>) {
        if models.is_empty() {
            self.banner
                .set_title("No models found. Is FastFlowLM installed?");
            self.banner.set_button_label(Some("Restart"));
            self.banner.set_revealed(true);
        }
        let current = self.st.borrow().model.clone();
        if !models.iter().any(|m| m.name == current) {
            models.insert(
                0,
                Model {
                    name: current,
                    installed: true,
                    labels: vec![],
                },
            );
        }
        self.model_menu.remove_all();
        for m in &models {
            let label = if m.can_use_tools() || m.labels.is_empty() {
                m.display_name()
            } else {
                format!("{} (chat only)", m.display_name())
            };
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(Some("win.model"), Some(&m.name.to_variant()));
            self.model_menu.append_item(&item);
        }
        self.st.borrow_mut().models = models;
        self.apply_model();
    }

    fn apply_model(&self) {
        self.show_state();
        let st = self.st.borrow();
        self.banner.set_revealed(false);
        self.banner.set_button_label(None); // only errors offer Restart
        if let Some(m) = st
            .models
            .iter()
            .find(|m| m.name == st.model)
            .filter(|m| !m.labels.is_empty())
        {
            if m.single_turn() {
                self.banner.set_title(
                    "This model doesn't remember earlier messages and can't take actions",
                );
                self.banner.set_revealed(true);
            } else if !m.can_use_tools() {
                self.banner
                    .set_title("This model can chat but can't take actions");
                self.banner.set_revealed(true);
            }
        }
    }

    fn on_model(self: &Rc<Self>, action: &gio::SimpleAction, value: &glib::Variant) {
        let name = value.get::<String>().unwrap_or_default();
        if self.st.borrow().busy || name == self.st.borrow().model {
            return;
        }
        action.set_state(value);
        self.st.borrow_mut().model = name.clone();
        config::update(&[("model", json!(name))]);
        self.core.reload();
        self.new_chat();
        self.apply_model();
        self.toast(&format!("Switched to {name}"));
    }

    pub fn set_state(&self, state: &str) {
        self.st.borrow_mut().state = state.to_string();
        self.show_state();
        self.unload_action
            .set_enabled(state == "Loaded" && !self.st.borrow().busy);
    }

    fn show_state(&self) {
        let st = self.st.borrow();
        self.title
            .set_subtitle(&format!("{} · {}", st.model, st.state));
    }

    /// Ask the service to load the model while the question is typed.
    fn preload(&self) {
        let state = self.st.borrow().state.clone();
        if state == "Loaded" || state == "Loading…" || self.loading.get() {
            return;
        }
        self.loading.set(true);
        self.core.load();
        let flag = self.loading.clone();
        glib::timeout_add_seconds_local_once(5, move || flag.set(false));
    }

    /// Restart the service (model server and connected apps too), then pick
    /// the chat back up from disk with fresh settings.
    pub fn restart(self: &Rc<Self>) {
        if self.st.borrow().busy {
            self.stop();
        }
        let before = self.core.owner();
        self.banner.set_revealed(false);
        self.set_state("Restarting…");
        self.core.restart();
        let w = Rc::downgrade(self);
        let started = std::time::Instant::now();
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            let Some(s) = w.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut gone = before.is_none() || s.core.owner() != before;
            if !gone && started.elapsed() < std::time::Duration::from_secs(20) {
                return glib::ControlFlow::Continue;
            }
            if !gone {
                // It hung: kill it and every flm serve, and start over.
                s.core.kill_service();
                gone = true;
            }
            s.core.finish_all("The assistant was restarted");
            let id = s.st.borrow().chat_id.clone();
            s.st.borrow_mut().chat_id = String::new(); // so open_chat reloads it
            if store::load_chat(&id).is_some() {
                s.open_chat(&id);
            } else {
                s.st.borrow_mut().chat_id = id;
            }
            s.load_models(); // also starts the new service
            s.toast(if gone {
                "Assistant restarted"
            } else {
                "The service didn't restart; try again"
            });
            glib::ControlFlow::Break
        });
    }

    pub fn unload(&self, manual: bool) {
        if self.st.borrow().busy {
            return;
        }
        self.core.unload();
        if manual {
            self.toast("Model unloaded");
        }
    }

    // -- chat ------------------------------------------------------------------------
    pub fn new_chat(self: &Rc<Self>) {
        if self.st.borrow().busy {
            self.stop();
        }
        {
            let mut st = self.st.borrow_mut();
            st.chat_id = store::new_chat_id();
            st.attachments.clear();
            st.messages.clear();
        }
        self.show_attachments();
        self.chat_list.unselect_all();
        while let Some(c) = self.messages.first_child() {
            self.messages.remove(&c);
        }
        self.greeting.set_label(greeting());
        self.split.set_show_content(true);
        self.stack.set_visible_child_name("empty");
        self.entry.grab_focus();
    }

    pub fn ask(self: &Rc<Self>, text: &str) {
        self.entry.set_text(text);
        if !self.st.borrow().busy {
            self.send();
        }
    }

    /// A question from GNOME search while the window is open: a long pause starts a new chat.
    pub fn ask_from_shell(self: &Rc<Self>, text: &str) {
        let stale = self
            .st
            .borrow()
            .last_turn
            .is_some_and(|t| t.elapsed() > STALE);
        if stale && !self.st.borrow().busy && !self.st.borrow().messages.is_empty() {
            self.new_chat();
        }
        self.win.present();
        self.ask(text);
    }

    fn send(self: &Rc<Self>) {
        let text = self.entry.text().trim().to_string();
        if text.is_empty() && self.st.borrow().attachments.is_empty() {
            return;
        }
        let mut prompt = if text.is_empty() {
            "What's in this picture?".to_string()
        } else {
            text.clone()
        };
        if let Some(rest) = text.strip_prefix('/') {
            let (name, details) = rest.split_once(' ').unwrap_or((rest, ""));
            match store::find_skill(name) {
                Some(skill) => prompt = store::skill_prompt(&skill, details),
                None => {
                    let names: Vec<String> = store::skills()
                        .iter()
                        .map(|s| format!("/{}", s.slug))
                        .collect();
                    self.toast(&format!(
                        "No skill called /{name}{}",
                        if names.is_empty() {
                            String::new()
                        } else {
                            format!(". Skills: {}", names.join(", "))
                        }
                    ));
                    return;
                }
            }
        }
        let (images, thumbs): (Vec<String>, Vec<gdk::Texture>) = {
            let mut st = self.st.borrow_mut();
            let a = std::mem::take(&mut st.attachments);
            (
                a.iter().map(|(_, u)| u.clone()).collect(),
                a.into_iter().map(|(t, _)| t).collect(),
            )
        };
        self.entry.set_text("");
        self.show_attachments();
        self.stack.set_visible_child_name("chat");
        let bubble = self.add_user(&text, &thumbs, true);
        if self.st.borrow().busy {
            self.queue_message(text, prompt, images, thumbs, bubble);
            return;
        }
        self.dispatch(&prompt, &images);
    }

    /// Show a message typed during an answer as queued: dimmed, with a way to take it back.
    fn queue_message(
        self: &Rc<Self>,
        text: String,
        prompt: String,
        images: Vec<String>,
        thumbs: Vec<gdk::Texture>,
        bubble: gtk::Box,
    ) {
        bubble.add_css_class("dim-label");
        let status = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::End)
            .build();
        let label = gtk::Label::new(Some("Queued"));
        label.add_css_class("caption");
        let undo = gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text("Take Back")
            .valign(gtk::Align::Center)
            .build();
        undo.add_css_class("flat");
        undo.add_css_class("circular");
        status.append(&label);
        status.append(&undo);
        bubble.append(&status);
        let (w, b) = (self.weak(), bubble.clone());
        undo.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.take_back(&b);
            }
        });
        self.st.borrow_mut().queue.push(Queued {
            text,
            prompt,
            images,
            thumbs,
            bubble,
            status,
        });
    }

    /// Put a queued message back in the text box and remove its bubble.
    fn take_back(&self, bubble: &gtk::Box) {
        let q = {
            let mut st = self.st.borrow_mut();
            let Some(i) = st.queue.iter().position(|q| q.bubble == *bubble) else {
                return;
            };
            st.queue.remove(i)
        };
        self.restore(vec![q]);
    }

    /// Queued messages go back into the text box (after a stop or an error), so
    /// nothing is lost and nothing is sent unasked.
    fn restore(&self, queued: Vec<Queued>) {
        if queued.is_empty() {
            return;
        }
        let mut texts: Vec<String> = queued.iter().map(|q| q.text.clone()).collect();
        let typed = self.entry.text().trim().to_string();
        if !typed.is_empty() {
            texts.push(typed);
        }
        for q in queued {
            let gone = q
                .bubble
                .parent()
                .and_then(|p| p.downcast::<gtk::Revealer>().ok());
            match gone {
                Some(rev) => self.messages.remove(&rev),
                None => self.messages.remove(&q.bubble),
            }
            let mut st = self.st.borrow_mut();
            st.attachments.extend(q.thumbs.into_iter().zip(q.images));
        }
        self.show_attachments();
        self.entry.set_text(&texts.join(" "));
        self.entry.set_position(-1);
    }

    /// Send the next queued message, if any.
    fn send_queued(self: &Rc<Self>) {
        if self.st.borrow().busy || self.st.borrow().queue.is_empty() {
            return;
        }
        let q = self.st.borrow_mut().queue.remove(0);
        q.bubble.remove(&q.status);
        q.bubble.remove_css_class("dim-label");
        self.dispatch(&q.prompt, &q.images);
    }

    fn dispatch(self: &Rc<Self>, prompt: &str, images: &[String]) {
        self.set_busy(true);
        let reply = Reply::new(self, true, None);
        self.st.borrow_mut().reply = Some(reply.clone());
        let chat_id = self.st.borrow().chat_id.clone();
        if let Err(e) = self.core.ask(&chat_id, prompt, images, reply.clone()) {
            reply.on_finished(&e);
        }
    }

    pub fn stop(&self) {
        if self.st.borrow().busy {
            self.core.stop();
        }
        // Stopping means "not now": what was queued goes back to the text box.
        let queued = std::mem::take(&mut self.st.borrow_mut().queue);
        self.restore(queued);
    }

    /// The service finished a turn (it has saved the chat).
    pub fn turn_finished(self: &Rc<Self>, error: &str) {
        let chat_id = self.st.borrow().chat_id.clone();
        if let Some(data) = store::load_chat(&chat_id) {
            self.st.borrow_mut().messages =
                data["messages"].as_array().cloned().unwrap_or_default();
        }
        {
            let mut st = self.st.borrow_mut();
            st.last_turn = Some(Instant::now());
            st.reply = None;
        }
        self.refresh_chats();
        self.set_busy(false);
        if !error.is_empty() {
            let queued = std::mem::take(&mut self.st.borrow_mut().queue);
            self.restore(queued);
        } else if !self.st.borrow().queue.is_empty() {
            // Not from inside the finished turn's own callback.
            let w = self.weak();
            glib::idle_add_local_once(move || {
                if let Some(w) = w.upgrade() {
                    w.send_queued();
                }
            });
        }
    }

    pub fn set_busy(&self, busy: bool) {
        self.st.borrow_mut().busy = busy;
        self.send_btn.set_icon_name(if busy {
            "media-playback-stop-symbolic"
        } else {
            "go-up-symbolic"
        });
        self.send_btn
            .set_tooltip_text(Some(if busy { "Stop" } else { "Send" }));
        self.send_btn.remove_css_class("suggested-action");
        self.send_btn.remove_css_class("destructive-action");
        self.send_btn.add_css_class(if busy {
            "destructive-action"
        } else {
            "suggested-action"
        });
        self.model_action.set_enabled(!busy);
        self.unload_action
            .set_enabled(self.st.borrow().state == "Loaded" && !busy);
        if !busy {
            self.entry.grab_focus();
        }
    }

    pub fn add_user(&self, text: &str, thumbs: &[gdk::Texture], animate: bool) -> gtk::Box {
        let bx = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .halign(gtk::Align::End)
            .margin_start(48)
            .build();
        if !thumbs.is_empty() {
            let pics = gtk::Box::builder()
                .spacing(6)
                .halign(gtk::Align::End)
                .build();
            for t in thumbs {
                let pic = gtk::Picture::builder()
                    .paintable(t)
                    .content_fit(gtk::ContentFit::Cover)
                    .width_request(120)
                    .height_request(90)
                    .overflow(gtk::Overflow::Hidden)
                    .build();
                pic.add_css_class("attachment");
                pics.append(&pic);
            }
            bx.append(&pics);
        }
        let shown = text.replace(store::IMAGE_MARKER, "").trim().to_string();
        if !shown.is_empty() || thumbs.is_empty() {
            let label = gtk::Label::builder()
                .label(if shown.is_empty() { text } else { &shown })
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .selectable(true)
                .halign(gtk::Align::End)
                .max_width_chars(52)
                .build();
            label.add_css_class("bubble-user");
            bx.append(&label);
        }
        if text.contains(store::IMAGE_MARKER) && thumbs.is_empty() {
            let note = gtk::Label::builder()
                .label("Image not saved")
                .halign(gtk::Align::End)
                .build();
            note.add_css_class("dim-label");
            note.add_css_class("caption");
            bx.append(&note);
        }
        if animate {
            self.append_animated(&bx)
        } else {
            self.messages.append(&bx)
        }
        bx
    }

    pub fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }
}

fn build_start_page() -> (gtk::ScrolledWindow, gtk::Label, Vec<gtk::Button>) {
    let bx = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Center)
        .margin_start(24)
        .margin_end(24)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    bx.append(
        &gtk::Image::builder()
            .icon_name(APP_ID)
            .pixel_size(96)
            .margin_bottom(12)
            .build(),
    );
    let greet = gtk::Label::builder()
        .label(greeting())
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    greet.add_css_class("greeting");
    bx.append(&greet);
    let sub = gtk::Label::builder()
        .label("Ask anything. It runs privately on your NPU and can search the web, see your screen, change settings and work with your apps and files.")
        .wrap(true).justify(gtk::Justification::Center).max_width_chars(46).build();
    sub.add_css_class("dim-label");
    bx.append(&sub);
    let chips: Vec<gtk::Button> = SUGGESTIONS
        .iter()
        .map(|(label, _)| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("suggestion");
            b
        })
        .collect();
    #[cfg(feature = "adw-1-8")]
    {
        // Chips flow like words.
        let wrap = adw::WrapBox::builder()
            .child_spacing(8)
            .line_spacing(8)
            .margin_top(22)
            .halign(gtk::Align::Center)
            .justify(adw::JustifyMode::None)
            .build();
        for c in &chips {
            wrap.append(c);
        }
        bx.append(&wrap);
    }
    #[cfg(not(feature = "adw-1-8"))]
    {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(3)
            .column_spacing(8)
            .row_spacing(8)
            .margin_top(22)
            .build();
        for c in &chips {
            flow.append(c);
        }
        bx.append(&flow);
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(620)
        .child(&bx)
        .valign(gtk::Align::Center)
        .build();
    (
        gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build(),
        greet,
        chips,
    )
}

fn build_sidebar() -> (
    adw::ToolbarView,
    gtk::SearchEntry,
    gtk::ListBox,
    gtk::Stack,
    adw::StatusPage,
) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search Chats")
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("navigation-sidebar");
    let has_chat_icon =
        gtk::IconTheme::for_display(&gdk::Display::default().unwrap()).has_icon("chat-symbolic");
    let empty = adw::StatusPage::builder()
        .icon_name(if has_chat_icon {
            "chat-symbolic"
        } else {
            "mail-unread-symbolic"
        })
        .title("No Chats")
        .description("Your conversations are saved here")
        .build();
    empty.add_css_class("compact");
    let stack = gtk::Stack::new();
    stack.add_named(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
        Some("list"),
    );
    stack.add_named(&empty, Some("empty"));
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bx.append(&search);
    bx.append(&stack);
    let view = adw::ToolbarView::builder().content(&bx).build();
    view.add_top_bar(&adw::HeaderBar::new());
    (view, search, list, stack, empty)
}
