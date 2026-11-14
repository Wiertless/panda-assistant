//! Opening apps, web pages and what apps publish (desktop actions).
//!
//! Kinds of apps ("email", "files", "browser"…) open the user's default app
//! for them, as set in GNOME Settings → Apps → Default Apps.

use std::sync::LazyLock;

use gio::prelude::*;
use regex::Regex;

use super::{norm_name, pick};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const APP_ROLES: [(&str, Option<&str>); 11] = [
    ("browser", Some("x-scheme-handler/https")), ("email", Some("x-scheme-handler/mailto")),
    ("files", Some("inode/directory")), ("calendar", Some("text/calendar")), ("music", Some("audio/mpeg")),
    ("video", Some("video/mp4")), ("photos", Some("image/jpeg")), ("text editor", Some("text/plain")),
    ("pdf", Some("application/pdf")), ("terminal", None), ("settings", None),
];

const ROLE_WORDS: [(&str, &[&str]); 11] = [
    ("browser", &["browser", "web browser", "internet", "web"]),
    ("email", &["email", "e-mail", "mail", "mail app", "email app", "inbox"]),
    ("files", &["files", "file manager", "file browser", "folders", "explorer", "finder"]),
    ("calendar", &["calendar", "agenda"]),
    ("music", &["music", "music player", "audio player"]),
    ("video", &["video", "videos", "video player", "movies"]),
    ("photos", &["photos", "pictures", "images", "image viewer", "photo viewer"]),
    ("text editor", &["text editor", "editor", "notepad"]),
    ("pdf", &["pdf", "pdf viewer", "document viewer", "documents"]),
    ("terminal", &["terminal", "console", "command line", "shell"]),
    ("settings", &["settings", "system settings", "preferences", "control panel"]),
];

const FALLBACK_IDS: [(&str, &[&str]); 2] = [
    ("terminal", &["org.gnome.Ptyxis", "org.gnome.Console", "org.gnome.Terminal"]),
    ("settings", &["org.gnome.Settings"]),
];

pub fn role_for(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_lowercase();
    let wanted = lower.trim_start_matches("my ").trim_start_matches("the ").trim_end_matches(" app");
    ROLE_WORDS.iter().find(|(_, words)| words.contains(&wanted)).map(|(role, _)| *role)
}

pub fn default_app(role: &str) -> Option<gio::AppInfo> {
    let ctype = APP_ROLES.iter().find(|(r, _)| *r == role).and_then(|(_, c)| *c);
    let app = match ctype {
        Some(c) if c.starts_with("x-scheme-handler/") => gio::AppInfo::default_for_uri_scheme(&c[17..]),
        Some(c) => gio::AppInfo::default_for_type(c, false),
        None => None,
    };
    app.or_else(|| {
        FALLBACK_IDS.iter().find(|(r, _)| *r == role)?.1.iter()
            .find_map(|id| gio::DesktopAppInfo::new(&format!("{id}.desktop")).map(|a| a.upcast()))
    })
}

/// (role, app name) for the roles that have a default app here.
pub fn default_apps() -> Vec<(String, String)> {
    APP_ROLES.iter()
        .filter_map(|(role, _)| default_app(role).map(|a| (role.to_string(), a.display_name().to_string())))
        .collect()
}

pub fn find_app(name: &str) -> Result<gio::AppInfo, ToolError> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return Err("no app name given".into());
    }
    if let Some(app) = role_for(&wanted).and_then(default_app) {
        return Ok(app);
    }
    let mut best = None;
    for app in gio::AppInfo::all() {
        if !app.should_show() {
            continue;
        }
        let id = app.id().map(|s| s.to_lowercase()).unwrap_or_default();
        let names = [app.display_name().to_lowercase(), id.trim_end_matches(".desktop").to_string()];
        if names.contains(&wanted) {
            return Ok(app);
        }
        let extra: Vec<String> = app.downcast_ref::<gio::DesktopAppInfo>()
            .map(|d| {
                let mut v = vec![d.generic_name().map(|s| s.to_lowercase()).unwrap_or_default()];
                v.extend(d.keywords().iter().map(|k| k.to_lowercase()));
                v
            })
            .unwrap_or_default();
        if best.is_none() && (names.iter().any(|n| n.contains(&wanted)) || extra.contains(&wanted)) {
            best = Some(app);
        }
    }
    best.ok_or_else(|| format!("no installed app matches '{name}'").into())
}

pub fn open_app(_: &Context, args: &Args) -> ToolResult {
    let app = find_app(&arg_str(args, "name"))?;
    app.launch(&[], None::<&gio::AppLaunchContext>).map_err(|e| ToolError(e.to_string()))?;
    Ok(format!("Opened {}.", app.display_name()))
}

/// A safe http(s) URL; bare host names get https://.
pub fn check_url(raw: &str) -> Result<String, ToolError> {
    static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:(\D|$)").unwrap());
    let mut url = raw.trim().to_string();
    if SCHEME.is_match(&url) {
        let lower = url.to_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("only http(s) web addresses can be opened".into());
        }
    } else {
        url = format!("https://{url}");
    }
    let host = url.split_once("://").map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or("")).unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    if host.is_empty() || url.chars().any(char::is_whitespace) {
        return Err("that isn't a valid web address".into());
    }
    Ok(url)
}

pub fn open_url(_: &Context, args: &Args) -> ToolResult {
    let url = check_url(&arg_str(args, "url"))?;
    gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>).map_err(|e| ToolError(e.to_string()))?;
    Ok(format!("Opened {url}."))
}

fn desktop_app(name: &str) -> Result<gio::DesktopAppInfo, ToolError> {
    let app = find_app(name)?;
    app.downcast_ref::<gio::DesktopAppInfo>().cloned()
        .or_else(|| gio::DesktopAppInfo::new(&app.id()?))
        .ok_or_else(|| format!("{} has no actions", app.display_name()).into())
}

fn actions(app: &gio::DesktopAppInfo) -> Vec<(String, String)> {
    app.list_actions().iter().map(|a| (a.to_string(), app.action_name(a).to_string())).collect()
}

pub fn list_app_actions(_: &Context, args: &Args) -> ToolResult {
    let app = desktop_app(&arg_str(args, "app"))?;
    let acts = actions(&app);
    if acts.is_empty() {
        return Ok(format!("{} has no actions.", app.display_name()));
    }
    Ok(format!("{} can: {}", app.display_name(), acts.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join(", ")))
}

pub fn run_app_action(_: &Context, args: &Args) -> ToolResult {
    let app = desktop_app(&arg_str(args, "app"))?;
    let acts = actions(&app);
    if acts.is_empty() {
        return Err(format!("{} has no actions", app.display_name()).into());
    }
    let wanted = arg_str(args, "action");
    let id = match acts.iter().find(|(id, _)| norm_name(id) == norm_name(&wanted)) {
        Some((id, _)) => id.clone(),
        None => {
            let labels: Vec<String> = acts.iter().map(|(_, l)| l.clone()).collect();
            let label = pick(&wanted, &labels, &format!("action of {}", app.display_name()))?;
            acts.iter().find(|(_, l)| *l == label).unwrap().0.clone()
        }
    };
    app.launch_action(&id, None::<&gio::AppLaunchContext>);
    let label = &acts.iter().find(|(i, _)| *i == id).unwrap().1;
    Ok(format!("{}: {label}.", app.display_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(check_url("example.com/x").unwrap(), "https://example.com/x");
        assert_eq!(check_url("http://a.b").unwrap(), "http://a.b");
        assert_eq!(check_url("localhost:8080").unwrap(), "https://localhost:8080");
        assert!(check_url("javascript:alert(1)").is_err());
        assert!(check_url("file:///etc/passwd").is_err());
        assert!(check_url("https://").is_err());
        assert!(check_url("exa mple.com").is_err());
    }

    #[test]
    fn roles() {
        assert_eq!(role_for("my email app"), Some("email"));
        assert_eq!(role_for("Files"), Some("files"));
        assert_eq!(role_for("zed"), None);
    }
}
