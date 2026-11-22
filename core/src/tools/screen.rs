//! Read what's in a window through the accessibility tree (AT-SPI), which is
//! D-Bus underneath: the session bus tells where the accessibility bus is,
//! and every app answers there for its windows, widgets and text.
//!
//! That gives the text of the page, document or message in the focused window,
//! and anything selected, which suits small models better than a screenshot.

use std::time::{Duration, Instant};

use gio::prelude::*;

use super::run::dbus_call;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const MAX_NODES: usize = 3000;
const MAX_CHARS: usize = 4000;
const TIME_BUDGET: Duration = Duration::from_secs(4);
const OBJECT_CHAR: char = '\u{fffc}'; // stands for an embedded child in AT-SPI text
const SKIP_APPS: [&str; 6] = [
    "gnome-shell",
    "mutter-x11-frames",
    "xdg-desktop-portal-gnome",
    "ibus-extension-gtk3",
    "ibus-x11",
    "evolution-alarm-notify",
];
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";
// AT-SPI state bits (AtspiStateType).
const STATE_ACTIVE: u32 = 1;
const STATE_SHOWING: u32 = 25;

/// What the walk needs to know about one node.
pub struct NodeInfo<N> {
    pub showing: bool,
    pub role: String,
    pub name: String,
    pub text: Option<String>,
    pub selection: String,
    pub children: Vec<N>,
}

/// Walk an accessibility tree depth-first, gathering visible text and
/// selections, within node, character and time limits.
pub fn collect_text<N>(
    root: N,
    mut info: impl FnMut(&N) -> Option<NodeInfo<N>>,
    max_nodes: usize,
    max_chars: usize,
    budget: Duration,
) -> (String, String) {
    let deadline = Instant::now() + budget;
    let (mut parts, mut selected): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let (mut seen, mut total) = (0, 0);
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if seen >= max_nodes || total >= max_chars || Instant::now() > deadline {
            break;
        }
        seen += 1;
        let Some(n) = info(&node) else { continue }; // apps vanish mid-walk
        if !n.showing {
            continue;
        }
        let text = n.text.or_else(|| {
            matches!(
                n.role.as_str(),
                "label" | "heading" | "list item" | "table cell" | "link"
            )
            .then_some(n.name)
        });
        if let Some(t) = text {
            let t = t
                .replace(OBJECT_CHAR, " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !t.is_empty() && parts.last() != Some(&t) {
                total += t.chars().count();
                parts.push(t);
            }
        }
        if !n.selection.is_empty() {
            selected.push(n.selection);
        }
        stack.extend(n.children.into_iter().take(500).rev());
    }
    let clip = |s: String| s.chars().take(max_chars).collect::<String>();
    (clip(parts.join("\n")), clip(selected.join("\n")))
}

/// A node: the app's bus name and the object path.
type Node = (String, String);

struct A11y {
    bus: gio::DBusConnection,
}

impl A11y {
    fn connect() -> Result<Self, ToolError> {
        let addr = dbus_call(
            false,
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.a11y.Bus",
            "GetAddress",
            None,
            Some(glib::VariantTy::new("(s)").unwrap()),
            3000,
        )
        .map_err(|_| ToolError("accessibility isn't available".into()))?;
        let addr: String = addr.child_value(0).get().unwrap_or_default();
        let bus = gio::DBusConnection::for_address_sync(
            &addr,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None::<&gio::DBusAuthObserver>,
            None::<&gio::Cancellable>,
        )
        .map_err(|e| {
            ToolError(format!(
                "couldn't reach the accessibility bus: {}",
                e.message()
            ))
        })?;
        Ok(Self { bus })
    }

    fn call(
        &self,
        (name, path): &Node,
        iface: &str,
        method: &str,
        args: Option<&glib::Variant>,
        reply: &str,
    ) -> Option<glib::Variant> {
        self.bus
            .call_sync(
                Some(name),
                path,
                iface,
                method,
                args,
                Some(glib::VariantTy::new(reply).unwrap()),
                gio::DBusCallFlags::NONE,
                1000,
                None::<&gio::Cancellable>,
            )
            .ok()
    }

    fn prop(&self, node: &Node, iface: &str, prop: &str) -> Option<glib::Variant> {
        self.call(
            node,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(iface, prop).to_variant()),
            "(v)",
        )?
        .child_value(0)
        .as_variant()
    }

    fn children(&self, node: &Node) -> Vec<Node> {
        self.call(node, ACCESSIBLE, "GetChildren", None, "(a(so))")
            .and_then(|v| {
                v.child_value(0)
                    .get::<Vec<(String, glib::variant::ObjectPath)>>()
            })
            .map(|c| {
                c.into_iter()
                    .map(|(n, p)| (n, p.as_str().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn states(&self, node: &Node) -> u64 {
        self.call(node, ACCESSIBLE, "GetState", None, "(au)")
            .and_then(|v| v.child_value(0).get::<Vec<u32>>())
            .map(|w| {
                w.first().copied().unwrap_or(0) as u64
                    | (w.get(1).copied().unwrap_or(0) as u64) << 32
            })
            .unwrap_or(0)
    }

    fn has(states: u64, bit: u32) -> bool {
        states & (1u64 << bit) != 0
    }

    fn name(&self, node: &Node) -> String {
        self.prop(node, ACCESSIBLE, "Name")
            .and_then(|v| v.get::<String>())
            .unwrap_or_default()
    }

    fn pid(&self, bus_name: &str) -> u32 {
        self.bus
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                Some(&(bus_name,).to_variant()),
                Some(glib::VariantTy::new("(u)").unwrap()),
                gio::DBusCallFlags::NONE,
                1000,
                None::<&gio::Cancellable>,
            )
            .ok()
            .and_then(|v| v.child_value(0).get::<u32>())
            .unwrap_or(0)
    }

    fn info(&self, node: &Node) -> Option<NodeInfo<Node>> {
        let states = self.states(node);
        let role = self
            .call(node, ACCESSIBLE, "GetRoleName", None, "(s)")?
            .child_value(0)
            .get::<String>()?;
        let ifaces: Vec<String> = self
            .call(node, ACCESSIBLE, "GetInterfaces", None, "(as)")
            .and_then(|v| v.child_value(0).get())
            .unwrap_or_default();
        let (mut text, mut selection) = (None, String::new());
        if ifaces.iter().any(|i| i == TEXT) {
            let count = self
                .prop(node, TEXT, "CharacterCount")
                .and_then(|v| v.get::<i32>())
                .unwrap_or(0)
                .min(20000);
            text = self
                .call(
                    node,
                    TEXT,
                    "GetText",
                    Some(&(0i32, count).to_variant()),
                    "(s)",
                )
                .and_then(|v| v.child_value(0).get::<String>());
            let n = self
                .call(node, TEXT, "GetNSelections", None, "(i)")
                .and_then(|v| v.child_value(0).get::<i32>())
                .unwrap_or(0);
            if n > 0
                && let Some((start, end)) = self
                    .call(
                        node,
                        TEXT,
                        "GetSelection",
                        Some(&(0i32,).to_variant()),
                        "(ii)",
                    )
                    .and_then(|v| v.get::<(i32, i32)>())
                    .filter(|(s, e)| e > s)
            {
                selection = self
                    .call(
                        node,
                        TEXT,
                        "GetText",
                        Some(&(start, end).to_variant()),
                        "(s)",
                    )
                    .and_then(|v| v.child_value(0).get::<String>())
                    .unwrap_or_default();
            }
        }
        let name = if text.is_none() {
            self.name(node)
        } else {
            String::new()
        };
        Some(NodeInfo {
            showing: Self::has(states, STATE_SHOWING),
            role,
            name,
            text,
            selection,
            children: self.children(node),
        })
    }
}

struct Window {
    app: String,
    title: String,
    node: Node,
    active: bool,
    showing: bool,
}

fn windows(a: &A11y, skip_pid: u32) -> Vec<Window> {
    let root = (
        "org.a11y.atspi.Registry".to_string(),
        "/org/a11y/atspi/accessible/root".to_string(),
    );
    let mut out = vec![];
    for app in a.children(&root) {
        let app_name = a.name(&app);
        if SKIP_APPS.contains(&app_name.as_str())
            || (skip_pid != 0 && a.pid(&app.0) == skip_pid)
            || a.pid(&app.0) == std::process::id()
        {
            continue;
        }
        for win in a.children(&app) {
            let st = a.states(&win);
            out.push(Window {
                app: app_name.clone(),
                title: a.name(&win),
                active: A11y::has(st, STATE_ACTIVE),
                showing: A11y::has(st, STATE_SHOWING),
                node: win,
            });
        }
    }
    out
}

pub fn format_result(app: &str, title: &str, text: &str, selected: &str) -> String {
    let mut out = format!("Window: {title} ({app})");
    if !selected.trim().is_empty() {
        out.push_str(&format!("\nSelected text:\n{}", selected.trim()));
    }
    if text.trim().is_empty() {
        out.push_str("\n(This window shares no text. Try look_at_screen instead.)");
    } else {
        out.push_str(&format!("\nWindow text:\n{}", text.trim()));
    }
    out
}

pub fn read_window(ctx: &Context, args: &Args) -> ToolResult {
    // If the assistant's own window has focus, step aside so the window the
    // user means (the one behind) becomes the focused one.
    if let Some(hide) = &ctx.hide_window {
        hide();
        std::thread::sleep(Duration::from_millis(400));
    }
    let result = read(
        &arg_str(args, "app"),
        ctx.client_pid.load(std::sync::atomic::Ordering::SeqCst),
    );
    if let Some(show) = &ctx.show_window {
        show();
    }
    result
}

fn read(app_name: &str, skip_pid: u32) -> ToolResult {
    let a = A11y::connect()?;
    let mut wins = windows(&a, skip_pid);
    let chosen: Vec<&Window> = if app_name.trim().is_empty() {
        wins.iter().filter(|w| w.active).collect()
    } else {
        let want = app_name.trim().to_lowercase();
        // The name GNOME shows (the pop-up's "Terminal") can differ from the program's
        // ("ptyxis"): the installed apps that answer to it, by their desktop file ids.
        let ids: Vec<String> = gio::DesktopAppInfo::search(&want)
            .into_iter()
            .flatten()
            .map(|id| id.to_lowercase())
            .collect();
        wins.sort_by_key(|w| (!w.active, !w.showing));
        wins.iter()
            .filter(|w| {
                let app = w.app.to_lowercase();
                app.contains(&want)
                    || w.title.to_lowercase().contains(&want)
                    || (!app.is_empty() && ids.iter().any(|id| id.contains(&app)))
            })
            .collect()
    };
    let Some(win) = chosen.first() else {
        let names: Vec<String> = wins
            .iter()
            .filter(|w| w.showing && !w.title.is_empty())
            .take(15)
            .map(|w| format!("{}: {}", w.app, w.title))
            .collect();
        let lead = if app_name.trim().is_empty() {
            "No window is focused.".to_string()
        } else {
            format!("No window matches “{}”.", app_name.trim())
        };
        return Ok(format!(
            "{lead} Ask which one, or call again with app set to one of these:\n{}",
            if names.is_empty() {
                "none".into()
            } else {
                names.join("\n")
            }
        ));
    };
    let (text, selected) = collect_text(
        win.node.clone(),
        |n| a.info(n),
        MAX_NODES,
        MAX_CHARS,
        TIME_BUDGET,
    );
    Ok(format_result(&win.app, &win.title, &text, &selected))
}

#[cfg(test)]
mod tests {
    use super::*;

    // (role, text, name, showing, selection, children)
    type T = (
        &'static str,
        Option<&'static str>,
        &'static str,
        bool,
        &'static str,
        Vec<usize>,
    );

    fn walk(tree: &[T], max_chars: usize) -> (String, String) {
        collect_text(
            0usize,
            |&i| {
                let (role, text, name, showing, sel, kids) = &tree[i];
                Some(NodeInfo {
                    showing: *showing,
                    role: role.to_string(),
                    name: name.to_string(),
                    text: text.map(String::from),
                    selection: sel.to_string(),
                    children: kids.clone(),
                })
            },
            100,
            max_chars,
            Duration::from_secs(5),
        )
    }

    #[test]
    fn visible_text_and_selection() {
        let tree: Vec<T> = vec![
            ("frame", None, "Win", true, "", vec![1, 2, 4]),
            ("label", None, "Invoice total", true, "", vec![]),
            ("panel", None, "", false, "", vec![3]),
            ("text", Some("hidden tab"), "", true, "", vec![]),
            (
                "document web",
                Some("Pay by \u{fffc} today"),
                "",
                true,
                "Pay by",
                vec![5],
            ),
            ("link", Some("30 Nov"), "", true, "", vec![]),
        ];
        assert_eq!(
            walk(&tree, 4000),
            (
                "Invoice total\nPay by today\n30 Nov".to_string(),
                "Pay by".to_string()
            )
        );
        assert_eq!(walk(&tree, 5).0, "Invoi");
    }

    #[test]
    fn formatting() {
        assert_eq!(
            format_result("Firefox", "News", "Body", "Sel"),
            "Window: News (Firefox)\nSelected text:\nSel\nWindow text:\nBody"
        );
        assert!(format_result("x", "y", "", "").contains("look_at_screen"));
    }
}
