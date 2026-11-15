//! GNOME Shell search, served by assistantd itself.
//!
//! The overview shows "Ask: <query>", plus saved skills and earlier chats
//! that match. Results come from plain files, so nothing waits on the model
//! while you type. Picking "Ask" hands the question to the window if it's
//! running; otherwise the service answers by itself and posts the answer as a
//! notification under the app's name, so no window process starts at all.

use std::sync::Arc;

use gio::prelude::*;
use glib::Variant;

use crate::service::Service;
use crate::store;

pub const BUS_NAME: &str = "io.github.wiertless.PandaAssistant.SearchProvider";
pub const OBJECT_PATH: &str = "/io/github/wiertless/PandaAssistant/SearchProvider";
pub const APP_ID: &str = "io.github.wiertless.PandaAssistant";
const APP_PATH: &str = "/io/github/wiertless/PandaAssistant";
const MIN_CHARS: usize = 3;
const MAX_SKILLS: usize = 2;
const MAX_CHATS: usize = 3;
// Result ids: a plain query means "Ask"; these prefixes mark the others.
pub const SKILL: &str = "panda-skill:";
pub const CHAT: &str = "panda-chat:";

const INTROSPECTION: &str = r#"
<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet"><arg type="as" name="terms" direction="in"/><arg type="as" name="results" direction="out"/></method>
    <method name="GetSubsearchResultSet"><arg type="as" name="previous_results" direction="in"/><arg type="as" name="terms" direction="in"/><arg type="as" name="results" direction="out"/></method>
    <method name="GetResultMetas"><arg type="as" name="identifiers" direction="in"/><arg type="aa{sv}" name="metas" direction="out"/></method>
    <method name="ActivateResult"><arg type="s" name="identifier" direction="in"/><arg type="as" name="terms" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
    <method name="LaunchSearch"><arg type="as" name="terms" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
  </interface>
</node>
"#;

fn matches(text: &str, words: &[String]) -> bool {
    let text = text.to_lowercase();
    words.iter().all(|w| text.contains(w))
}

pub fn results_for(terms: &[String]) -> Vec<String> {
    let query = terms.join(" ").trim().to_string();
    if query.chars().count() < MIN_CHARS {
        return vec![];
    }
    let words: Vec<String> = terms.iter().map(|t| t.to_lowercase().trim_start_matches('/').to_string())
        .filter(|t| !t.is_empty()).collect();
    let mut out = vec![query];
    out.extend(store::skills().into_iter()
        .filter(|s| matches(&format!("{} {} {}", s.slug, s.title, s.description), &words))
        .take(MAX_SKILLS).map(|s| format!("{SKILL}{}", s.slug)));
    out.extend(store::list_chats("").into_iter().filter(|c| matches(&c.title, &words))
        .take(MAX_CHATS).map(|c| format!("{CHAT}{}", c.id)));
    out
}

pub fn meta_for(result: &str) -> (String, String, String) {
    if let Some(slug) = result.strip_prefix(SKILL) {
        if let Some(s) = store::find_skill(slug) {
            let desc = if s.description.is_empty() { format!("/{}", s.slug) } else { s.description.clone() };
            return (format!("Run skill: {}", s.title), desc, "view-list-bullet-symbolic".into());
        }
    } else if let Some(id) = result.strip_prefix(CHAT)
        && let Some(c) = store::load_chat(id) {
            return (c["title"].as_str().unwrap_or("Chat").into(), "Earlier chat".into(), "document-open-recent-symbolic".into());
        }
    (format!("Ask: {result}"), "Ask the assistant on your NPU".into(), APP_ID.into())
}

fn meta_variant(result: &str) -> Variant {
    let (name, desc, icon) = meta_for(result);
    let dict = glib::VariantDict::new(None);
    dict.insert("id", result);
    dict.insert("name", name);
    dict.insert("description", desc);
    dict.insert("gicon", icon);
    dict.end()
}

/// Is the window's process running (on the bus)?
pub fn app_running(conn: &gio::DBusConnection) -> bool {
    conn.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner",
                   Some(&(APP_ID,).to_variant()), Some(glib::VariantTy::new("(b)").unwrap()), gio::DBusCallFlags::NONE,
                   2000, None::<&gio::Cancellable>)
        .ok().and_then(|v| v.child_value(0).get::<bool>()).unwrap_or(false)
}

/// Run one of the window's actions (D-Bus starts the window if needed).
pub fn app_action(conn: &gio::DBusConnection, action: &str, value: &str) {
    let params: Vec<Variant> = vec![value.to_variant()];
    let platform = glib::VariantDict::new(None).end();
    let _ = conn.call_sync(Some(APP_ID), APP_PATH, "org.freedesktop.Application", "ActivateAction",
                           Some(&Variant::tuple_from_iter([action.to_variant(), params.to_variant(), platform])),
                           None, gio::DBusCallFlags::NONE, 10000,
                           None::<&gio::Cancellable>);
}

/// A notification under the app's name; clicking it opens that chat in the window.
pub fn notify(conn: &gio::DBusConnection, title: &str, body: &str, chat_id: &str, button: &str) {
    let n = glib::VariantDict::new(None);
    n.insert("title", title);
    n.insert("body", body);
    if let Some(icon) = gio::ThemedIcon::new(APP_ID).serialize() {
        n.insert_value("icon", &icon);
    }
    n.insert("default-action", "app.open-chat");
    n.insert_value("default-action-target", &chat_id.to_variant());
    let b = glib::VariantDict::new(None);
    b.insert("label", button);
    b.insert("action", "app.open-chat");
    b.insert_value("target", &chat_id.to_variant());
    n.insert_value("buttons", &Variant::array_from_iter_with_type(glib::VariantTy::VARDICT, [b.end()]));
    // GNOME Shell's own notification interface (what GApplication uses), so
    // notifications group under the app and replace each other by id.
    let sent = conn.call_sync(Some("org.gtk.Notifications"), "/org/gtk/Notifications", "org.gtk.Notifications",
                              "AddNotification", Some(&Variant::tuple_from_iter([APP_ID.to_variant(), "panda".to_variant(), n.end()])), None,
                              gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>);
    if let Err(e) = &sent {
        eprintln!("assistantd: GNOME Shell notification failed: {}", e.message());
        // Other desktops: the freedesktop interface, without actions.
        let hints = glib::VariantDict::new(None);
        hints.insert("desktop-entry", APP_ID);
        let _ = conn.call_sync(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications",
                               "org.freedesktop.Notifications", "Notify",
                               Some(&Variant::tuple_from_iter(["Assistant".to_variant(), 0u32.to_variant(), APP_ID.to_variant(),
                                   title.to_variant(), body.to_variant(), Vec::<String>::new().to_variant(), hints.end(), (-1i32).to_variant()])),
                               None, gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>);
    }
}

/// Set the clipboard with no window open, through the clipboard extension in
/// GNOME Shell (which, on Wayland, is what may set it for the session).
pub fn set_clipboard(conn: &gio::DBusConnection, text: &str) {
    let _ = conn.call_sync(Some("org.gnome.Shell"), "/io/github/wiertless/PandaAssistant/Clipboard",
                           "io.github.wiertless.PandaAssistant.Clipboard1", "Set", Some(&(text,).to_variant()), None,
                           gio::DBusCallFlags::NONE, 3000, None::<&gio::Cancellable>);
}

/// A reminder that's due: Snooze and Done act on it without opening a window.
pub fn notify_reminder(conn: &gio::DBusConnection, r: &crate::reminders::Reminder) {
    let n = glib::VariantDict::new(None);
    let timer = r.text == "Timer";
    n.insert("title", if timer { "Timer done" } else { "Reminder" });
    n.insert("body", if timer { "Your timer is up." } else { r.text.as_str() });
    n.insert("priority", "urgent");
    if let Some(icon) = gio::ThemedIcon::new("alarm-symbolic").serialize() {
        n.insert_value("icon", &icon);
    }
    n.insert("default-action", "app.done-reminder");
    n.insert_value("default-action-target", &r.id.to_variant());
    let button = |label: &str, action: &str| {
        let b = glib::VariantDict::new(None);
        b.insert("label", label);
        b.insert("action", action);
        b.insert_value("target", &r.id.to_variant());
        b.end()
    };
    let buttons = [button(&format!("Snooze {} min", crate::reminders::SNOOZE_MINUTES), "app.snooze-reminder"),
                   button("Done", "app.done-reminder")];
    n.insert_value("buttons", &Variant::array_from_iter_with_type(glib::VariantTy::VARDICT, buttons));
    let id = format!("reminder-{}", r.id);
    let sent = conn.call_sync(Some("org.gtk.Notifications"), "/org/gtk/Notifications", "org.gtk.Notifications",
                              "AddNotification", Some(&Variant::tuple_from_iter([APP_ID.to_variant(), id.to_variant(), n.end()])),
                              None, gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>);
    if sent.is_err() {
        notify(conn, "Reminder", &r.text, "", "Open Chat");
    }
}

/// Notification text: markdown marks removed, cut at a word.
pub fn plain(markdown: &str, limit: usize) -> String {
    let mut text = markdown.replace("```", "").replace("**", "").replace("__", "").replace('`', "");
    text = text.lines().map(|l| l.trim_start_matches('#').trim_start()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    if text.chars().count() > limit {
        let cut: String = text.chars().take(limit).collect();
        text = format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a));
    }
    text
}

fn activate(service: &Arc<Service>, conn: &gio::DBusConnection, result: &str) {
    if let Some(id) = result.strip_prefix(CHAT) {
        app_action(conn, "open-chat", id);
        return;
    }
    let text = match result.strip_prefix(SKILL) {
        Some(slug) => format!("/{slug}"),
        None => result.to_string(),
    };
    if app_running(conn) {
        app_action(conn, "ask", &text); // the window decides: show it there, or notify
    } else {
        service.ask_in_background(&text);
    }
}

/// Export the search provider on the service's connection.
pub fn register(service: &Arc<Service>, conn: &gio::DBusConnection) -> Result<gio::RegistrationId, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(INTROSPECTION)?;
    let iface = node.lookup_interface("org.gnome.Shell.SearchProvider2").expect("interface in XML");
    let service = service.clone();
    let c = conn.clone();
    conn.register_object(OBJECT_PATH, &iface)
        .method_call(move |_conn, _sender, _path, _iface, method, params, invocation| {
            service.touch();
            match method {
                "GetInitialResultSet" => {
                    let (terms,): (Vec<String>,) = params.get().unwrap_or_default();
                    invocation.return_value(Some(&(results_for(&terms),).to_variant()));
                }
                "GetSubsearchResultSet" => {
                    let (_prev, terms): (Vec<String>, Vec<String>) = params.get().unwrap_or_default();
                    invocation.return_value(Some(&(results_for(&terms),).to_variant()));
                }
                "GetResultMetas" => {
                    let (ids,): (Vec<String>,) = params.get().unwrap_or_default();
                    let metas = Variant::array_from_iter_with_type(glib::VariantTy::VARDICT, ids.iter().map(|i| meta_variant(i)));
                    invocation.return_value(Some(&Variant::tuple_from_iter([metas])));
                }
                "ActivateResult" => {
                    let (id, _terms, _ts): (String, Vec<String>, u32) = params.get().unwrap_or_default();
                    invocation.return_value(None);
                    activate(&service, &c, &id);
                }
                "LaunchSearch" => {
                    let (terms, _ts): (Vec<String>, u32) = params.get().unwrap_or_default();
                    invocation.return_value(None);
                    activate(&service, &c, &terms.join(" "));
                }
                _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
            }
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use serde_json::json;

    #[test]
    fn skills_and_chats_follow_the_ask_result() {
        let _h = temp_home();
        store::save_skill("Morning Routine", "1. dark mode", "Start the day").unwrap();
        store::save_chat("c1", &[json!({"role": "user", "content": "Plan my morning run"})], "m", "", 0).unwrap();
        store::save_chat("c2", &[json!({"role": "user", "content": "Something else"})], "m", "", 0).unwrap();
        let terms = vec!["morning".to_string()];
        assert_eq!(results_for(&terms), ["morning", "panda-skill:morning-routine", "panda-chat:c1"]);
        assert_eq!(meta_for("panda-skill:morning-routine").0, "Run skill: Morning Routine");
        assert_eq!(meta_for("panda-chat:c1").0, "Plan my morning run");
        assert_eq!(meta_for("what time").0, "Ask: what time");
        assert!(results_for(&["hi".to_string()]).is_empty());
        assert_eq!(results_for(&["/morning".into(), "routine".into()])[1], "panda-skill:morning-routine");
    }

    #[test]
    fn notification_text() {
        assert_eq!(plain("# Hi\n\n**Bold** and `code`", 100), "Hi\nBold and code");
        assert_eq!(plain("one two three four", 9), "one two…");
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;

    #[test]
    fn dictionaries_are_not_wrapped() {
        let dict = glib::VariantDict::new(None).end();
        assert_eq!(Variant::tuple_from_iter([APP_ID.to_variant(), "panda".to_variant(), dict.clone()]).type_().as_str(), "(ssa{sv})");
        assert_eq!(Variant::array_from_iter_with_type(glib::VariantTy::VARDICT, [dict]).type_().as_str(), "aa{sv}");
        assert_eq!(meta_variant("hi").type_().as_str(), "a{sv}");
    }
}
