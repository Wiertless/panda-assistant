//! Application intents (freedesktop.org Application Intents Specification):
//! apps declare `Implements=` in their desktop file, often a D-Bus interface
//! such as org.freedesktop.FileManager1, and `intentapps.list` picks the
//! default app per intent, the way mimeapps.list does for file types.

use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::files::{home_path, short};
use super::run::dbus_call;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

pub const FILE_MANAGER: &str = "org.freedesktop.FileManager1";

fn desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':')
        .filter(|d| !d.is_empty()).map(|d| d.to_ascii_lowercase()).collect()
}

fn env_dirs(var: &str, default: &str) -> Vec<PathBuf> {
    std::env::var(var).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into())
        .split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
}

/// The intentapps.list files, most important first (spec section 2).
pub fn list_files() -> Vec<PathBuf> {
    let config_home = crate::paths::config_dir().parent().map(Path::to_path_buf).unwrap_or_default();
    let config_dirs = env_dirs("XDG_CONFIG_DIRS", "/etc/xdg");
    let data_dirs = env_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share");
    let desks = desktops();
    let mut out = vec![];
    let mut push = |dir: &Path, sub: &str| {
        for d in &desks {
            out.push(dir.join(sub).join(format!("{d}-intentapps.list")));
        }
        out.push(dir.join(sub).join("intentapps.list"));
    };
    push(&config_home, "");
    for d in &config_dirs {
        push(d, "");
    }
    for d in &data_dirs {
        push(d, "applications");
    }
    out
}

/// Desktop IDs listed for an intent under [Default Applications] in one file.
pub fn defaults_in(text: &str, intent: &str) -> Vec<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Default Applications]";
        } else if in_group
            && let Some((k, v)) = line.split_once('=')
                && k.trim() == intent {
                    return v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
                }
    }
    vec![]
}

fn implements(app: &gio::DesktopAppInfo, intent: &str) -> bool {
    app.string("Implements").is_some_and(|v| v.split(';').any(|i| i.trim() == intent))
}

/// The user's app for an intent: the first listed one that's installed and
/// really implements it, else the first implementation by desktop ID.
pub fn default_for(intent: &str) -> Option<gio::DesktopAppInfo> {
    for file in list_files() {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for id in defaults_in(&text, intent) {
            if let Some(app) = gio::DesktopAppInfo::new(&id).filter(|a| implements(a, intent)) {
                return Some(app);
            }
        }
    }
    let mut all = gio::DesktopAppInfo::implementations(intent);
    all.sort_by_key(|a| a.id().map(|s| s.to_string()).unwrap_or_default());
    all.into_iter().next()
}

/// Open the file manager on a file's folder with the file selected.
pub fn show_in_files(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    let uri = gio::File::for_path(&path).uri().to_string();
    dbus_call(false, FILE_MANAGER, "/org/freedesktop/FileManager1", FILE_MANAGER, "ShowItems",
              Some(&(vec![uri], "").to_variant()), None, 10000)
        .map_err(|e| ToolError(format!("the file manager didn't answer: {}", e.message())))?;
    Ok(format!("Showing {} in Files.", short(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_default_applications() {
        let text = "[Default Applications]\norg.freedesktop.FileManager1=org.gnome.Nautilus.desktop;nemo.desktop;\n\
                    [org.freedesktop.FileManager1]\nhttp=x.desktop;\n";
        assert_eq!(defaults_in(text, FILE_MANAGER), ["org.gnome.Nautilus.desktop", "nemo.desktop"]);
        assert!(defaults_in(text, "com.example.Calculator1").is_empty());
        assert!(defaults_in("[org.freedesktop.FileManager1]\norg.freedesktop.FileManager1=a;", FILE_MANAGER).is_empty());
    }

    #[test]
    fn lookup_order_follows_the_spec() {
        let _h = crate::store::testenv::temp_home();
        // SAFETY: tests that touch the environment hold the temp_home lock.
        unsafe {
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME:Unity");
            std::env::set_var("XDG_CONFIG_DIRS", "/etc/xdg");
            std::env::set_var("XDG_DATA_DIRS", "/usr/share");
        }
        let files: Vec<String> = list_files().iter().map(|p| p.display().to_string()).collect();
        let home = crate::paths::home();
        assert_eq!(files[0], format!("{}/config/gnome-intentapps.list", home.display()));
        assert_eq!(files[2], format!("{}/config/intentapps.list", home.display()));
        assert_eq!(files[3], "/etc/xdg/gnome-intentapps.list");
        assert_eq!(files.last().unwrap(), "/usr/share/applications/intentapps.list");
    }
}

// -- other apps' GNOME search (org.gnome.Shell.SearchProvider2) -----------------------
//
// Most GNOME apps answer the overview's search: Calculator does sums,
// Characters finds emoji, Settings finds panels, Weather gives forecasts,
// Files finds files. The assistant can ask them the same way the shell does.

const PROVIDER_IFACE: &str = "org.gnome.Shell.SearchProvider2";
const MAX_PER_APP: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub desktop_id: String,
    pub bus_name: String,
    pub object_path: String,
    pub default_disabled: bool,
}

/// A search-provider .ini file ([Shell Search Provider] group).
pub fn parse_provider(text: &str) -> Option<Provider> {
    let mut p = Provider { desktop_id: String::new(), bus_name: String::new(), object_path: String::new(), default_disabled: false };
    let mut version = 0;
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Shell Search Provider]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| in_group) else { continue };
        match k.trim() {
            "DesktopId" => p.desktop_id = v.trim().into(),
            "BusName" => p.bus_name = v.trim().into(),
            "ObjectPath" => p.object_path = v.trim().into(),
            "Version" => version = v.trim().parse().unwrap_or(0),
            "DefaultDisabled" => p.default_disabled = v.trim() == "true",
            _ => {}
        }
    }
    (version == 2 && !p.desktop_id.is_empty() && !p.bus_name.is_empty() && !p.object_path.is_empty()).then_some(p)
}

/// Installed providers the user hasn't turned off in Settings → Search (ours excluded).
pub fn providers() -> Vec<Provider> {
    let settings = gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup("org.gnome.desktop.search-providers", true))
        .map(|_| gio::Settings::new("org.gnome.desktop.search-providers"));
    if settings.as_ref().is_some_and(|s| s.boolean("disable-external")) {
        return vec![];
    }
    let disabled: Vec<String> = settings.as_ref().map(|s| s.strv("disabled").iter().map(|x| x.to_string()).collect()).unwrap_or_default();
    let enabled: Vec<String> = settings.as_ref().map(|s| s.strv("enabled").iter().map(|x| x.to_string()).collect()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for dir in env_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share") {
        let Ok(entries) = std::fs::read_dir(dir.join("gnome-shell/search-providers")) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let Some(p) = std::fs::read_to_string(&f).ok().and_then(|t| parse_provider(&t)) else { continue };
            let off = disabled.contains(&p.desktop_id) || (p.default_disabled && !enabled.contains(&p.desktop_id));
            if off || p.desktop_id.starts_with("io.github.wiertless.PandaAssistant") || !seen.insert(p.desktop_id.clone()) {
                continue;
            }
            out.push(p);
        }
    }
    out
}

fn app_name(desktop_id: &str) -> String {
    gio::DesktopAppInfo::new(desktop_id).map(|a| a.display_name().to_string())
        .unwrap_or_else(|| desktop_id.trim_end_matches(".desktop").to_string())
}

fn ask_provider(p: &Provider, terms: &[String]) -> Vec<(String, String, String)> {
    let call = |method: &str, args: glib::Variant, reply: &str| {
        dbus_call(false, &p.bus_name, &p.object_path, PROVIDER_IFACE, method, Some(&args),
                  Some(glib::VariantTy::new(reply).unwrap()), 2500).ok()
    };
    let Some(ids) = call("GetInitialResultSet", (terms.to_vec(),).to_variant(), "(as)")
        .and_then(|v| v.child_value(0).get::<Vec<String>>()) else { return vec![] };
    let ids: Vec<String> = ids.into_iter().take(MAX_PER_APP).collect();
    if ids.is_empty() {
        return vec![];
    }
    let Some(metas) = call("GetResultMetas", (ids,).to_variant(), "(aa{sv})") else { return vec![] };
    let metas = metas.child_value(0);
    (0..metas.n_children()).filter_map(|i| {
        let d = glib::VariantDict::new(Some(&metas.child_value(i)));
        let id = d.lookup::<String>("id").ok().flatten()?;
        let name = d.lookup::<String>("name").ok().flatten().unwrap_or_default();
        let desc = d.lookup::<String>("description").ok().flatten().unwrap_or_default();
        Some((id, name, desc))
    }).collect()
}

fn terms_of(query: &str) -> Vec<String> {
    query.split_whitespace().map(String::from).collect()
}

/// Ask every app's search at once, like the overview does.
pub fn search_apps(_: &Context, args: &Args) -> ToolResult {
    let query = arg_str(args, "query");
    let terms = terms_of(&query);
    if terms.is_empty() {
        return Err("say what to search for".into());
    }
    let handles: Vec<_> = providers().into_iter().map(|p| {
        let terms = terms.clone();
        std::thread::spawn(move || (app_name(&p.desktop_id), p.desktop_id.clone(), ask_provider(&p, &terms)))
    }).collect();
    let mut lines = vec![];
    for h in handles {
        let Ok((app, desktop_id, hits)) = h.join() else { continue };
        for (id, name, desc) in hits {
            let desc = if desc.is_empty() { String::new() } else { format!(" — {desc}") };
            lines.push(format!("{app}: {name}{desc} [app {desktop_id}, id {id}]"));
        }
    }
    Ok(if lines.is_empty() { format!("No app has results for “{query}”.") } else {
        format!("{}\nopen_search_result opens one (with its app and id).", lines.join("\n"))
    })
}

pub fn open_search_result(_: &Context, args: &Args) -> ToolResult {
    let app = arg_str(args, "app");
    let id = arg_str(args, "id");
    let p = providers().into_iter().find(|p| p.desktop_id == app || p.desktop_id.trim_end_matches(".desktop") == app)
        .ok_or_else(|| ToolError(format!("no app search called '{app}'")))?;
    let stamp = (glib::monotonic_time() / 1000) as u32;
    dbus_call(false, &p.bus_name, &p.object_path, PROVIDER_IFACE, "ActivateResult",
              Some(&(id.as_str(), terms_of(&arg_str(args, "query")), stamp).to_variant()), None, 5000)
        .map_err(|e| ToolError(format!("{} didn't open it: {}", app_name(&p.desktop_id), e.message())))?;
    Ok(format!("Opened it in {}.", app_name(&p.desktop_id)))
}

// -- the Terminal intent (org.freedesktop.Terminal1) ------------------------------------

pub const TERMINAL: &str = "org.freedesktop.Terminal1";

/// Fill a method's arguments by name and type: the working directory where
/// one is asked for, empty values everywhere else (shell, no extra env).
pub fn terminal_args(arg_types: &[(String, String)], dir: &str) -> Option<glib::Variant> {
    let mut parts = vec![];
    for (name, ty) in arg_types {
        let v = match (ty.as_str(), name.as_str()) {
            ("ay", n) if n.contains("dir") => {
                let mut b = dir.as_bytes().to_vec();
                b.push(0); // bytestrings in D-Bus APIs are nul-terminated
                b.to_variant()
            }
            ("ay", _) => vec![0u8].to_variant(),
            ("aay", _) => Vec::<Vec<u8>>::new().to_variant(),
            ("s", n) if n.contains("dir") => dir.to_variant(),
            ("s", _) => "".to_variant(),
            ("as", _) => Vec::<String>::new().to_variant(),
            ("a{sv}", _) => glib::VariantDict::new(None).end(),
            _ => return None, // a shape we don't know: don't guess
        };
        parts.push(v);
    }
    Some(glib::Variant::tuple_from_iter(parts))
}

/// A method's in-arguments (name, type) from introspection XML. gio-rs
/// doesn't expose DBusArgInfo, and introspection XML is regular enough.
fn method_args(xml: &str, iface: &str, method: &str) -> Option<Vec<(String, String)>> {
    gio::DBusNodeInfo::for_xml(xml).ok()?.lookup_interface(iface)?.lookup_method(method)?;
    let attr = |tag: &str, key: &str| {
        let i = tag.find(&format!("{key}=\""))? + key.len() + 2;
        Some(tag[i..].split('"').next()?.to_string())
    };
    let body = &xml[xml.find(&format!("<interface name=\"{iface}\""))?..];
    let body = &body[body.find(&format!("<method name=\"{method}\""))?..];
    let body = &body[..body.find("</method>")?];
    Some(body.split("<arg").skip(1)
        .filter(|t| attr(t, "direction").is_none_or(|d| d == "in"))
        .filter_map(|t| Some((attr(t, "name").unwrap_or_default(), attr(t, "type")?)))
        .collect())
}

fn launch_via_intent(dir: &Path) -> Option<String> {
    let app = default_for(TERMINAL)?;
    let id = app.id()?.trim_end_matches(".desktop").to_string();
    let path = format!("/{}", id.replace('.', "/").replace('-', "_"));
    let xml = dbus_call(false, &id, &path, "org.freedesktop.DBus.Introspectable", "Introspect", None,
                        Some(glib::VariantTy::new("(s)").unwrap()), 5000).ok()?.child_value(0).get::<String>()?;
    let args = terminal_args(&method_args(&xml, TERMINAL, "LaunchCommand")?, &dir.to_string_lossy())?;
    dbus_call(false, &id, &path, TERMINAL, "LaunchCommand", Some(&args), None, 10000).ok()?;
    Some(app.display_name().to_string())
}

/// Open the user's terminal in a folder (no command: commands go through run_command).
pub fn open_terminal(_: &Context, args: &Args) -> ToolResult {
    let raw = arg_str(args, "folder");
    let dir = home_path(if raw.trim().is_empty() { "~" } else { &raw }, true)?;
    if !dir.is_dir() {
        return Err(format!("{} isn't a folder", short(&dir)).into());
    }
    if let Some(app) = launch_via_intent(&dir) {
        return Ok(format!("Opened {app} in {}.", short(&dir)));
    }
    // No terminal implements the intent yet: the default-terminal helper, then known terminals.
    let d = dir.to_string_lossy().to_string();
    let tries: [(&str, Vec<String>); 4] = [
        ("xdg-terminal-exec", vec![format!("--dir={d}")]),
        ("ptyxis", vec!["--new-window".into(), "--working-directory".into(), d.clone()]),
        ("kgx", vec!["--working-directory".into(), d.clone()]),
        ("gnome-terminal", vec![format!("--working-directory={d}")]),
    ];
    for (cmd, a) in tries {
        if crate::flm::which(cmd).is_some() {
            let mut argv = vec![cmd];
            argv.extend(a.iter().map(String::as_str));
            super::spawn_detached(&argv)?;
            return Ok(format!("Opened a terminal in {}.", short(&dir)));
        }
    }
    Err(ToolError::cannot("no terminal app found"))
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn search_provider_files() {
        let ini = "[Shell Search Provider]\nDesktopId=org.gnome.Calculator.desktop\nObjectPath=/org/gnome/Calculator/SearchProvider\n\
                   BusName=org.gnome.Calculator.SearchProvider\nVersion=2\n";
        let p = parse_provider(ini).unwrap();
        assert_eq!((p.desktop_id.as_str(), p.default_disabled), ("org.gnome.Calculator.desktop", false));
        assert!(parse_provider(&ini.replace("Version=2", "Version=1")).is_none());
        assert!(parse_provider("[Other]\nDesktopId=x\nBusName=y\nObjectPath=/z\nVersion=2").is_none());
        assert!(parse_provider(&format!("{ini}DefaultDisabled=true\n")).unwrap().default_disabled);
    }

    #[test]
    fn terminal_arguments_by_name() {
        let shape = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let args = terminal_args(&shape(&[("exec", "aay"), ("working_directory", "ay"), ("env", "aay"), ("options", "a{sv}"),
                                          ("platform_data", "a{sv}")]), "/home/u/proj").unwrap();
        assert_eq!(args.type_().as_str(), "(aayayaaya{sv}a{sv})");
        assert_eq!(args.child_value(1).fixed_array::<u8>().unwrap(), b"/home/u/proj\0");
        assert!(terminal_args(&shape(&[("x", "(ii)")]), "/").is_none());
        let xml = r#"<node><interface name="org.freedesktop.Terminal1"><method name="LaunchCommand">
            <arg type="aay" name="exec" direction="in"/><arg type="ay" name="working_directory" direction="in"/>
            </method></interface></node>"#;
        assert_eq!(method_args(xml, TERMINAL, "LaunchCommand").unwrap(), shape(&[("exec", "aay"), ("working_directory", "ay")]));
    }
}
