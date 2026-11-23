//! Assistant: a local AI assistant for GNOME on your NPU. This is the window;
//! the model, tools and memory live in the assistantd service.

// Callback tables (actions, service hooks) read best spelled out.
#![allow(clippy::type_complexity)]

mod config;
mod core;
mod markup;
mod popup;
mod prefs;
mod reply;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use window::{APP_ID, APP_NAME, Window};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO_URL: &str = "https://github.com/Wiertless/panda-assistant";
const CSS: &str = r#"
.greeting { font-size: 24pt; font-weight: 800; }
.suggestion { border-radius: 999px; padding: 6px 16px; font-weight: normal; }
.composer {
  background: @card_bg_color;
  border-radius: 26px;
  padding: 5px 5px 5px 18px;
  box-shadow: 0 0 0 1px alpha(currentColor, 0.07), 0 2px 6px alpha(black, 0.08), 0 8px 24px alpha(black, 0.06);
}
window.popup { background: none; box-shadow: none; outline: none; border: none; }
.popup-pill {
  border-radius: 24px;
  padding: 4px 4px 4px 16px;
  transition: padding 420ms cubic-bezier(0.34, 1.56, 0.64, 1), border-radius 420ms cubic-bezier(0.34, 1.56, 0.64, 1);
  background: @popover_bg_color;
  color: @popover_fg_color;
  box-shadow: 0 4px 14px alpha(black, 0.14), 0 10px 30px alpha(black, 0.10);
}
.info-card { padding: 12px 16px; }
.popup-pill.open { border-radius: 28px; padding: 9px 9px 9px 20px; }
.composer entry, .composer entry:focus-within { background: none; box-shadow: none; outline: none; min-height: 36px; }
.bubble-user {
  background: @accent_bg_color;
  color: @accent_fg_color;
  border-radius: 20px 20px 6px 20px;
  padding: 9px 15px;
}
.bubble-user selection { background: alpha(white, 0.35); color: @accent_fg_color; }
.assistant-name { font-weight: 700; }
.assistant-text { padding: 2px 0; }
.pill { border-radius: 999px; padding: 4px 12px 4px 9px; font-size: 0.92em; background: alpha(@accent_color, 0.10); }
.pill.failed { background: alpha(@warning_color, 0.16); }
.pill.quiet { background: alpha(currentColor, 0.06); }
.pill image { color: @accent_color; }
.pill.failed image { color: @warning_color; }
.confirm-card { padding: 6px; }
.think-toggle:checked { color: @accent_color; background: alpha(@accent_color, 0.14); }
expander-widget.thinking > box > title { opacity: 0.7; font-size: 0.92em; padding: 0; min-height: 0; }
expander-widget.thinking.live > box > title { opacity: 0.9; }
.thinking-text { font-size: 0.92em; opacity: 0.72; padding: 4px 0 2px 12px; margin-left: 6px; border-left: 2px solid alpha(currentColor, 0.15); }
expander-widget.step > box > title { padding: 0; min-height: 0; }
expander-widget.step > box > title > arrow { min-width: 12px; min-height: 12px; margin-right: 4px; opacity: 0.6; }
.step-time { opacity: 0.55; font-size: 0.85em; }
.step-detail { font-family: monospace; font-size: 0.86em; padding: 8px 12px; margin-top: 6px; border-radius: 8px; background: alpha(currentColor, 0.05); }
.confirm-detail { padding: 8px 12px; border-radius: 8px; background: alpha(currentColor, 0.06); }
.attachment { border-radius: 10px; }
.chat-row-time { font-size: 0.85em; }
"#;

type Slot = Rc<RefCell<Option<Rc<Window>>>>;

/// The window, created on first need.
fn window(app: &adw::Application, slot: &Slot) -> Rc<Window> {
    if let Some(w) = slot.borrow().as_ref() {
        return w.clone();
    }
    let w = Window::new(app);
    add_window_actions(&w);
    let s = slot.clone();
    w.win.connect_destroy(move |_| {
        s.borrow_mut().take();
    });
    *slot.borrow_mut() = Some(w.clone());
    w
}

fn present(app: &adw::Application, slot: &Slot) -> Rc<Window> {
    let w = window(app, slot);
    w.win.present();
    app.withdraw_notification("panda");
    w
}

fn add_window_actions(w: &Rc<Window>) {
    let delete = gio::SimpleAction::new("delete-chat", Some(glib::VariantTy::STRING));
    let weak = w.weak();
    delete.connect_activate(move |_, v| {
        if let (Some(w), Some(id)) = (weak.upgrade(), v.and_then(|v| v.get::<String>())) {
            w.delete_chat(&id);
        }
    });
    w.win.add_action(&delete);
    let remove = gio::SimpleAction::new("remove-attachment", Some(glib::VariantTy::UINT32));
    let weak = w.weak();
    remove.connect_activate(move |_, v| {
        if let (Some(w), Some(i)) = (weak.upgrade(), v.and_then(|v| v.get::<u32>())) {
            w.remove_attachment(i as usize);
        }
    });
    w.win.add_action(&remove);
}

fn show_about(app: &adw::Application, slot: &Slot) {
    let about = adw::AboutDialog::builder()
        .application_name(APP_NAME)
        .application_icon(APP_ID)
        .version(VERSION)
        .developer_name("Wiertless")
        .website(REPO_URL)
        .issue_url(format!("{REPO_URL}/issues"))
        .license_type(gtk::License::MitX11)
        .comments(
            "A local assistant that runs on your NPU with FastFlowLM and can act on your desktop.",
        )
        .build();
    let parent = slot.borrow().as_ref().map(|w| w.win.clone());
    about.present(parent.as_ref());
    let _ = app;
}

fn main() -> glib::ExitCode {
    if std::env::args().any(|a| a == "--version") {
        println!("{APP_NAME} {VERSION}");
        return glib::ExitCode::SUCCESS;
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "ask",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Ask a question",
        Some("TEXT"),
    );
    app.add_main_option(
        "open-chat",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Open a saved chat",
        Some("ID"),
    );
    app.add_main_option(
        "new-chat",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start a new chat",
        None,
    );
    app.add_main_option(
        "popup",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open ready to ask, with what's on screen (for a keyboard shortcut)",
        None,
    );
    app.add_main_option(
        "unload",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Unload the model",
        None,
    );
    app.add_main_option(
        "version",
        b'\0'.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Show the version",
        None,
    );
    let slot: Slot = Rc::default();

    let s = slot.clone();
    app.connect_startup(move |app| {
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        // Running from a checkout: find the icon next to the sources.
        let dev_icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
        if dev_icons.join(format!("{APP_ID}.svg")).exists() {
            gtk::IconTheme::for_display(&gdk::Display::default().unwrap())
                .add_search_path(dev_icons);
        }
        let simple: [(&str, Box<dyn Fn()>); 6] = {
            let (a1, s1) = (app.clone(), s.clone());
            let (a2, s2) = (app.clone(), s.clone());
            let (a3, s3) = (app.clone(), s.clone());
            let (a4, s4) = (app.clone(), s.clone());
            let (a5, s5) = (app.clone(), s.clone());
            let s6 = s.clone();
            [
                ("quit", Box::new(move || a1.quit())),
                ("about", Box::new(move || show_about(&a2, &s1))),
                (
                    "activate",
                    Box::new(move || {
                        present(&a3, &s2);
                    }),
                ),
                // Right-click actions of the app icon (see the desktop file).
                ("new-chat", Box::new(move || present(&a4, &s3).new_chat())),
                (
                    "look-at-screen",
                    Box::new(move || present(&a5, &s4).ask("What's on my screen?")),
                ),
                (
                    "unload-model",
                    Box::new(move || {
                        if let Some(w) = s6.borrow().as_ref() {
                            w.unload(true)
                        } else {
                            // No window: ask the service directly.
                            let _ = s5;
                            crate::core::Core::new().unload();
                        }
                    }),
                ),
            ]
        };
        for (name, f) in simple {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| f());
            app.add_action(&a);
        }
        // From GNOME search (via assistantd) and from notifications.
        let (a, s2) = (app.clone(), s.clone());
        let ask = gio::SimpleAction::new("ask", Some(glib::VariantTy::STRING));
        ask.connect_activate(move |_, v| {
            if let Some(text) = v.and_then(|v| v.get::<String>()) {
                present(&a, &s2).ask_from_shell(&text);
            }
        });
        app.add_action(&ask);
        let (a, s2) = (app.clone(), s.clone());
        // A reminder notification's buttons: handled without opening a window.
        for (name, method) in [
            ("snooze-reminder", "SnoozeReminder"),
            ("done-reminder", "DismissReminder"),
        ] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let a = app.clone();
            action.connect_activate(move |_, v| {
                if let Some(id) = v.and_then(|v| v.get::<String>()) {
                    let guard = a.hold(); // stay up until the service answered
                    crate::core::Core::reminder(method, &id, move || drop(guard));
                }
            });
            app.add_action(&action);
        }
        let open = gio::SimpleAction::new("open-chat", Some(glib::VariantTy::STRING));
        open.connect_activate(move |_, v| {
            if let Some(id) = v.and_then(|v| v.get::<String>()) {
                present(&a, &s2).open_chat(&id);
            }
        });
        app.add_action(&open);
        for (action, accels) in [
            ("app.quit", &["<Ctrl>q"][..]),
            ("window.close", &["<Ctrl>w"]),
            ("win.new-chat", &["<Ctrl>n"]),
            ("win.focus-entry", &["<Ctrl>l"]),
            ("win.stop", &["Escape"]),
            ("win.show-help-overlay", &["<Ctrl>question"]),
            ("win.preferences", &["<Ctrl>comma"]),
            ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal"]),
            ("win.zoom-out", &["<Ctrl>minus"]),
            ("win.zoom-reset", &["<Ctrl>0"]),
            ("win.search-chats", &["<Ctrl>f"]),
            ("win.attach", &["<Ctrl><Shift>a"]),
        ] {
            app.set_accels_for_action(action, accels);
        }
    });

    let s = slot.clone();
    app.connect_activate(move |app| {
        present(app, &s);
    });

    let s = slot.clone();
    app.connect_command_line(move |app, cmd| {
        let opts = cmd.options_dict();
        if opts.contains("unload") {
            app.activate_action("unload-model", None);
            return glib::ExitCode::SUCCESS;
        }
        if opts.contains("popup") {
            let (a, s2) = (app.clone(), s.clone());
            popup::show(
                app,
                Rc::new(move |chat: &str, follow_up: &str| {
                    let w = present(&a, &s2);
                    w.open_chat(chat);
                    if !follow_up.is_empty() {
                        w.ask(follow_up);
                    }
                }),
            );
            return glib::ExitCode::SUCCESS;
        }
        let w = present(app, &s);
        if let Some(text) = opts.lookup::<String>("ask").ok().flatten() {
            w.ask(&text);
        } else if let Some(id) = opts.lookup::<String>("open-chat").ok().flatten() {
            w.open_chat(&id);
        } else if opts.contains("new-chat") {
            w.new_chat();
        }
        glib::ExitCode::SUCCESS
    });

    // Stop a turn this window started; the service keeps or frees the model per its settings.
    let s = slot.clone();
    app.connect_shutdown(move |_| {
        if let Some(w) = s.borrow().as_ref()
            && w.st.borrow().busy
        {
            w.stop()
        }
    });
    app.run()
}
