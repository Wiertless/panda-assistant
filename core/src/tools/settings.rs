//! Desktop settings: dark mode, Night Light, sound, brightness, power mode,
//! radios, and any GNOME desktop setting within an allowlist.

use std::fs;

use gio::prelude::*;

use super::run::{dbus_call, run, set_property};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_bool, arg_percent, arg_str};

fn gsettings_set(schema: &str, key: &str, value: glib::Variant) -> Result<(), ToolError> {
    let src = gio::SettingsSchemaSource::default().ok_or("GNOME settings aren't available")?;
    if src.lookup(schema, true).is_none() {
        return Err(format!("there is no settings schema called {schema}").into());
    }
    gio::Settings::new(schema).set_value(key, &value).map_err(|e| ToolError(e.to_string()))?;
    gio::Settings::sync();
    Ok(())
}

fn gsettings_get(schema: &str, key: &str) -> Option<glib::Variant> {
    gio::SettingsSchemaSource::default()?.lookup(schema, true)?;
    Some(gio::Settings::new(schema).value(key))
}

/// Set a key and remember its old value for undo.
fn gsettings_change(ctx: &Context, what: &str, schema: &str, key: &str, value: glib::Variant) -> Result<(), ToolError> {
    let old = gsettings_get(schema, key);
    gsettings_set(schema, key, value)?;
    if let Some(old) = old {
        let (schema, key) = (schema.to_string(), key.to_string());
        ctx.can_undo(what, move || {
            gsettings_set(&schema, &key, old.clone())?;
            Ok(format!("{key} is back to {}.", old.print(false)))
        });
    }
    Ok(())
}

/// (volume percent, muted) of the default output.
fn volume_now() -> Option<(u32, bool)> {
    let out = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], 5).ok()?;
    let v: f64 = out.split_whitespace().nth(1)?.parse().ok()?;
    Some(((v * 100.0).round() as u32, out.contains("MUTED")))
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn set_dark_mode(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_change(ctx, &format!("Dark mode {}", on_off(on)), "org.gnome.desktop.interface", "color-scheme",
                     (if on { "prefer-dark" } else { "default" }).to_variant())?;
    Ok(format!("Dark mode {}.", on_off(on)))
}

pub fn set_do_not_disturb(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_change(ctx, &format!("Do Not Disturb {}", on_off(on)), "org.gnome.desktop.notifications", "show-banners",
                     (!on).to_variant())?;
    Ok(format!("Do Not Disturb {}.", on_off(on)))
}

pub fn set_night_light(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_change(ctx, &format!("Night Light {}", on_off(on)), "org.gnome.settings-daemon.plugins.color",
                     "night-light-enabled", on.to_variant())?;
    Ok(format!("Night Light {}.", on_off(on)))
}

pub fn set_volume(ctx: &Context, args: &Args) -> ToolResult {
    let pct = arg_percent(args, "percent", 0)?;
    let before = volume_now();
    run(&["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{pct}%")], 15)?;
    if let Some((old, _)) = before {
        ctx.can_undo(&format!("Volume {pct}%"), move || {
            run(&["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{old}%")], 15)?;
            Ok(format!("volume is back to {old}%."))
        });
    }
    Ok(format!("Volume set to {pct}%."))
}

pub fn set_mute(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "muted")?;
    let before = volume_now();
    run(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", if on { "1" } else { "0" }], 15)?;
    if let Some((_, was)) = before {
        ctx.can_undo(if on { "Mute" } else { "Unmute" }, move || {
            run(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", if was { "1" } else { "0" }], 15)?;
            Ok(if was { "muted again." } else { "unmuted again." }.into())
        });
    }
    Ok(if on { "Muted." } else { "Unmuted." }.into())
}

fn backlight() -> Result<(String, u32, u32), ToolError> {
    let mut devs: Vec<_> = fs::read_dir("/sys/class/backlight").into_iter().flatten().flatten().map(|e| e.path()).collect();
    devs.sort();
    for d in devs {
        let read = |f: &str| fs::read_to_string(d.join(f)).ok().and_then(|s| s.trim().parse::<u32>().ok());
        if let (Some(cur), Some(top)) = (read("brightness"), read("max_brightness")) {
            return Ok((d.file_name().unwrap().to_string_lossy().into(), cur, top));
        }
    }
    Err(ToolError::cannot("no screen backlight found"))
}

fn set_backlight(dev: &str, value: u32) -> Result<(), ToolError> {
    dbus_call(true, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session",
              "SetBrightness", Some(&("backlight", dev, value).to_variant()), None, 5000)
        .map(|_| ()).map_err(|e| ToolError(e.message().to_string()))
}

pub fn set_brightness(ctx: &Context, args: &Args) -> ToolResult {
    let pct = arg_percent(args, "percent", 1)?;
    let (dev, cur, top) = backlight()?;
    let value = ((top as f64 * pct as f64 / 100.0).round() as u32).max(1);
    set_backlight(&dev, value)?;
    ctx.can_undo(&format!("Brightness {pct}%"), move || {
        set_backlight(&dev, cur)?;
        Ok(format!("brightness is back to {}%.", (cur as f64 * 100.0 / top.max(1) as f64).round()))
    });
    Ok(format!("Brightness set to {pct}%."))
}

pub fn set_wifi(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    let was = run(&["nmcli", "radio", "wifi"], 5).ok().map(|s| s.trim() == "enabled");
    run(&["nmcli", "radio", "wifi", on_off(on)], 15)?;
    if let Some(was) = was {
        ctx.can_undo(&format!("Wi-Fi {}", on_off(on)), move || {
            run(&["nmcli", "radio", "wifi", on_off(was)], 15)?;
            Ok(format!("Wi-Fi is {} again.", on_off(was)))
        });
    }
    Ok(format!("Wi-Fi {}.", on_off(on)))
}

fn bluetooth_airplane(on: bool) -> Result<(), ToolError> {
    set_property(false, "org.gnome.SettingsDaemon.Rfkill", "/org/gnome/SettingsDaemon/Rfkill",
                 "org.gnome.SettingsDaemon.Rfkill", "BluetoothAirplaneMode", on.to_variant())
        .map_err(|e| ToolError(e.message().to_string()))
}

pub fn set_bluetooth(ctx: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    bluetooth_airplane(!on)?;
    ctx.can_undo(&format!("Bluetooth {}", on_off(on)), move || {
        bluetooth_airplane(on)?;
        Ok(format!("Bluetooth is {} again.", on_off(!on)))
    });
    Ok(format!("Bluetooth {}.", on_off(on)))
}

pub const POWER_MODES: [(&str, &str); 3] = [("power-saver", "Power Saver"), ("balanced", "Balanced"), ("performance", "Performance")];

pub fn set_power_mode(ctx: &Context, args: &Args) -> ToolResult {
    let was = super::run::get_property(true, "net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles",
                                       "ActiveProfile").ok().and_then(|v| v.get::<String>());
    let mode = arg_str(args, "mode").trim().to_lowercase().replace(' ', "-");
    let label = POWER_MODES.iter().find(|(m, _)| *m == mode).map(|(_, l)| *l)
        .ok_or("mode must be power-saver, balanced or performance")?;
    let mut last = String::from("no power profile service");
    // tuned-ppd and power-profiles-daemon both serve the new and the legacy name.
    for (name, path) in [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
                         ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")] {
        match set_property(true, name, path, name, "ActiveProfile", mode.to_variant()) {
            Ok(()) => {
                if let Some(was) = was.clone() {
                    let (name, path) = (name.to_string(), path.to_string());
                    ctx.can_undo(&format!("Power mode {label}"), move || {
                        set_property(true, &name, &path, &name, "ActiveProfile", was.to_variant())
                            .map_err(|e| ToolError(e.message().to_string()))?;
                        Ok(format!("power mode is back to {was}."))
                    });
                }
                return Ok(format!("Power mode set to {label}."));
            }
            Err(e) => last = e.message().to_string(),
        }
    }
    Err(format!("couldn't change the power mode: {last}").into())
}

pub fn get_status(_: &Context, _: &Args) -> ToolResult {
    let mut out = vec![];
    if let Some(v) = gsettings_get("org.gnome.desktop.interface", "color-scheme") {
        out.push(format!("dark mode: {}", on_off(v.str() == Some("prefer-dark"))));
    }
    if let Some(v) = gsettings_get("org.gnome.desktop.notifications", "show-banners") {
        out.push(format!("do not disturb: {}", on_off(!v.get::<bool>().unwrap_or(true))));
    }
    if let Ok(w) = run(&["nmcli", "radio", "wifi"], 5) {
        out.push(format!("wifi: {w}"));
    }
    if let Ok(v) = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], 5) {
        out.push(format!("volume: {v}"));
    }
    if let Ok((_, cur, top)) = backlight() {
        out.push(format!("brightness: {}%", (100.0 * cur as f64 / top.max(1) as f64).round()));
    }
    Ok(if out.is_empty() { "No status available.".into() } else { out.join("; ") })
}

// Generic GNOME settings. Only desktop schemas, and never the keys that load
// code (extensions) or lock the user out (lockdown, screensaver lock).
const SETTINGS_PREFIXES: [&str; 5] = ["org.gnome.desktop.", "org.gnome.settings-daemon.plugins.", "org.gnome.mutter",
                                      "org.gnome.shell.keybindings", "org.gnome.nautilus."];
const BLOCKED_KEYS: [&str; 6] = ["enabled-extensions", "disabled-extensions", "disable-user-extensions",
                                 "development-tools", "allow-extension-installation", "custom-keybindings"];
// Settings that run commands (custom shortcuts, the default terminal), let
// others in (remote desktop, sharing) or guard the session stay out of reach.
const BLOCKED_SCHEMAS: [&str; 6] = ["org.gnome.desktop.lockdown", "org.gnome.desktop.screensaver",
    "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding", "org.gnome.desktop.default-applications",
    "org.gnome.desktop.remote-desktop", "org.gnome.settings-daemon.plugins.sharing"];

fn allowed_schema(id: &str) -> bool {
    SETTINGS_PREFIXES.iter().any(|p| id.starts_with(p)) && !BLOCKED_SCHEMAS.iter().any(|b| id.starts_with(b))
}

fn schema(name: &str) -> Result<gio::SettingsSchema, ToolError> {
    let name = name.trim();
    if !allowed_schema(name) {
        return Err(format!("{} isn't a desktop setting the assistant may use",
                           if name.is_empty() { "that schema" } else { name }).into());
    }
    let s = gio::SettingsSchemaSource::default().and_then(|s| s.lookup(name, true))
        .ok_or_else(|| ToolError(format!("there is no settings schema called {name}; find_settings finds the right one")))?;
    // Relocatable schemas (per-app, per-shortcut) need a path and aren't for the assistant.
    if s.path().is_none() {
        return Err(format!("{name} isn't a desktop setting the assistant may use").into());
    }
    Ok(s)
}

fn key(schema: &gio::SettingsSchema, key: &str) -> Result<gio::SettingsSchemaKey, ToolError> {
    let key = key.trim();
    if BLOCKED_KEYS.contains(&key) {
        return Err(format!("{key} can't be changed by the assistant").into());
    }
    if !schema.has_key(key) {
        let keys: Vec<String> = schema.list_keys().iter().map(|k| k.to_string()).collect();
        let close = super::close_to(key, &keys, 6);
        return Err(format!("{} has no key {key}. {}", schema.id(),
                           if close.is_empty() { "find_settings looks keys up by what they do".into() }
                           else { format!("Close: {}", close.join(", ")) }).into());
    }
    Ok(schema.key(key))
}

pub fn find_settings(_: &Context, args: &Args) -> ToolResult {
    let words: Vec<String> = arg_str(args, "query").to_lowercase().split_whitespace().map(String::from).collect();
    if words.is_empty() {
        return Err("say what to look for, e.g. 'font scaling'".into());
    }
    let src = gio::SettingsSchemaSource::default().ok_or("GNOME settings aren't available")?;
    let (mut ids, _) = src.list_schemas(true);
    ids.sort();
    let mut hits = vec![];
    for id in ids.iter().filter(|i| allowed_schema(i)) {
        let Some(s) = src.lookup(id, true) else { continue };
        for k in s.list_keys() {
            let summary = s.key(&k).summary().map(|x| x.to_string()).unwrap_or_default();
            let text = format!("{id} {k} {summary}").to_lowercase();
            if words.iter().all(|w| text.contains(w)) && !BLOCKED_KEYS.contains(&k.as_str()) {
                hits.push(format!("{id} {k}: {summary}"));
            }
        }
    }
    Ok(if hits.is_empty() { "No matching settings.".into() } else { hits.into_iter().take(12).collect::<Vec<_>>().join("\n") })
}

pub fn get_setting(_: &Context, args: &Args) -> ToolResult {
    let s = schema(&arg_str(args, "schema"))?;
    let k = key(&s, &arg_str(args, "key"))?;
    let value = gio::Settings::new_full(&s, None::<&gio::SettingsBackend>, None).value(&k.name());
    Ok(format!("{} {} = {}", s.id(), k.name(), value.print(false)))
}

pub fn change_setting(ctx: &Context, args: &Args) -> ToolResult {
    let s = schema(&arg_str(args, "schema"))?;
    let k = key(&s, &arg_str(args, "key"))?;
    let raw = arg_str(args, "value").trim().to_string();
    let vtype = k.value_type();
    let value = match glib::Variant::parse(Some(&vtype), &raw) {
        Ok(v) => v,
        Err(_) if vtype.as_str() == "s" => raw.trim_matches(|c| c == '\'' || c == '"').to_variant(),
        Err(_) => return Err(format!("{raw} isn't a valid {} value", vtype.as_str()).into()),
    };
    if !k.range_check(&value) {
        return Err(format!("{raw} isn't an allowed value for {}", k.name()).into());
    }
    let settings = gio::Settings::new_full(&s, None::<&gio::SettingsBackend>, None);
    let old = settings.value(&k.name());
    settings.set_value(&k.name(), &value).map_err(|e| ToolError(e.to_string()))?;
    gio::Settings::sync();
    let (id, name) = (s.id().to_string(), k.name().to_string());
    ctx.can_undo(&format!("{name} = {}", value.print(false)), move || {
        gio::Settings::new(&id).set_value(&name, &old).map_err(|e| ToolError(e.to_string()))?;
        gio::Settings::sync();
        Ok(format!("{name} is back to {}.", old.print(false)))
    });
    Ok(format!("Set {} {} to {}.", s.id(), k.name(), value.print(false)))
}

pub const SETTINGS_PANELS: [&str; 26] = ["wifi", "network", "bluetooth", "display", "sound", "power", "background",
    "appearance", "notifications", "search", "multitasking", "applications", "privacy", "online-accounts", "sharing",
    "mouse", "keyboard", "printers", "color", "accessibility", "system", "users", "datetime", "region", "wacom", "about"];

pub fn open_settings(_: &Context, args: &Args) -> ToolResult {
    let panel = arg_str(args, "panel").trim().to_lowercase();
    if !SETTINGS_PANELS.contains(&panel.as_str()) {
        return Err(format!("unknown Settings page; one of: {}", SETTINGS_PANELS.join(", ")).into());
    }
    if crate::flm::which("gnome-control-center").is_none() {
        return Err(ToolError::cannot("GNOME Settings isn't installed"));
    }
    super::spawn_detached(&["gnome-control-center", &panel])?;
    Ok(format!("Opened Settings at {panel}."))
}

pub fn copy_to_clipboard(ctx: &Context, args: &Args) -> ToolResult {
    let text = arg_str(args, "text");
    if text.is_empty() {
        return Err("nothing to copy".into());
    }
    let copy = ctx.copy_to_clipboard.as_ref().ok_or("the clipboard isn't available")?;
    copy(&text);
    Ok("Copied to the clipboard.".into())
}

// -- selected text (the Assistant Clipboard shell extension) ----------------------------

const REWRITE: &str = "Rewrite the text as the instruction says. Keep its language unless told otherwise, keep \
names, numbers and formatting. Output only the rewritten text, nothing before or after it.";

/// The text highlighted right now in any app (the primary selection, through GNOME Shell).
pub fn selected_text() -> Result<String, ToolError> {
    let reply = super::run::dbus_call(false, "org.gnome.Shell", CLIP_PATH, CLIP_IFACE, "Selection", None,
                                      Some(glib::VariantTy::new("(s)").unwrap()), 3000)
        .map_err(|_| ToolError::cannot("reading the selection needs the Assistant Clipboard extension (turn it on in the Extensions app)"))?;
    Ok(reply.child_value(0).get::<String>().unwrap_or_default())
}

/// rewrite_selection: proofread, shorten, translate… the selected text, in a
/// fresh context with no tools, and put the result on the clipboard.
pub fn rewrite_selection(ctx: &Context, args: &Args) -> ToolResult {
    let instruction = arg_str(args, "instruction");
    let instruction = if instruction.trim().is_empty() { "Fix spelling, grammar and punctuation.".to_string() } else { instruction };
    let text = match arg_str(args, "text") {
        t if !t.trim().is_empty() => t,
        _ => selected_text()?,
    };
    if text.trim().is_empty() {
        return Err("nothing is selected: select the text first, then ask again".into());
    }
    let ask = ctx.ask_model.as_ref().ok_or("the model isn't available")?;
    let out = ask(REWRITE, &format!("Instruction: {instruction}\n\nText:\n{text}"))?;
    let out = out.trim().trim_matches('"').trim().to_string();
    if out.is_empty() {
        return Err("the rewrite came back empty".into());
    }
    let copy = ctx.copy_to_clipboard.as_ref().ok_or("the clipboard isn't available")?;
    copy(&out);
    let preview: String = out.chars().take(160).collect();
    Ok(format!("Copied the result to the clipboard; paste it with Ctrl+V. It reads: {preview}{}",
               if out.chars().count() > 160 { "…" } else { "" }))
}

// -- clipboard history (the Assistant Clipboard shell extension) -------------------------

const CLIP_PATH: &str = "/io/github/wiertless/PandaAssistant/Clipboard";
const CLIP_IFACE: &str = "io.github.wiertless.PandaAssistant.Clipboard1";
const CLIP_SHOWN: usize = 300;

/// Recent copies, newest first, optionally only those mentioning `query`.
pub fn format_clips(clips: &[String], query: &str) -> String {
    let q = query.trim().to_lowercase();
    let hits: Vec<(usize, &String)> = clips.iter().enumerate()
        .filter(|(_, c)| q.is_empty() || q.split_whitespace().all(|w| c.to_lowercase().contains(w))).collect();
    if hits.is_empty() {
        return if q.is_empty() { "Nothing copied yet.".into() } else {
            format!("Nothing copied recently mentions “{query}”. Newest: {}", clips.first().map(|c| short_clip(c)).unwrap_or_default())
        };
    }
    hits.iter().map(|(i, c)| format!("{}. {}", i + 1, short_clip(c))).collect::<Vec<_>>().join("\n")
}

fn short_clip(c: &str) -> String {
    let one: String = c.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > CLIP_SHOWN { format!("{}…", one.chars().take(CLIP_SHOWN).collect::<String>()) } else { one }
}

pub fn clipboard_history(_: &Context, args: &Args) -> ToolResult {
    let reply = super::run::dbus_call(false, "org.gnome.Shell", CLIP_PATH, CLIP_IFACE, "History", None,
                                      Some(glib::VariantTy::new("(as)").unwrap()), 3000)
        .map_err(|_| ToolError::cannot("clipboard history is off: turn on the Assistant Clipboard extension in the Extensions app"))?;
    let clips: Vec<String> = reply.child_value(0).get().unwrap_or_default();
    Ok(format_clips(&clips, &arg_str(args, "query")))
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn settings_that_run_commands_or_open_the_machine_are_blocked() {
        for id in ["org.gnome.settings-daemon.plugins.media-keys.custom-keybinding", "org.gnome.desktop.default-applications.terminal",
                   "org.gnome.desktop.remote-desktop.rdp", "org.gnome.desktop.screensaver", "org.gnome.system.proxy"] {
            assert!(!allowed_schema(id), "{id}");
        }
        assert!(allowed_schema("org.gnome.desktop.interface"));
        assert!(BLOCKED_KEYS.contains(&"custom-keybindings"));
    }
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn rewrites_go_to_the_clipboard() {
        let copied = Arc::new(Mutex::new(String::new()));
        let c = copied.clone();
        let ctx = Context {
            ask_model: Some(Arc::new(|_: &str, user: &str| {
                assert!(user.contains("Instruction: proofread") && user.contains("i will be their at five"));
                Ok("\"I will be there at five.\"".into())
            })),
            copy_to_clipboard: Some(Arc::new(move |t: &str| *c.lock().unwrap() = t.to_string())),
            ..Default::default()
        };
        let args: Args = serde_json::from_value(serde_json::json!({"instruction": "proofread", "text": "i will be their at five"})).unwrap();
        let out = rewrite_selection(&ctx, &args).unwrap();
        assert_eq!(*copied.lock().unwrap(), "I will be there at five.");
        assert!(out.contains("Ctrl+V") && out.contains("I will be there"));
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    #[test]
    fn clips_newest_first_and_filtered() {
        let clips: Vec<String> = ["https://example.com/flat", "ul. Długa 5\n00-001 Warszawa", &"x".repeat(500)].map(String::from).to_vec();
        assert_eq!(format_clips(&clips, "warszawa"), "2. ul. Długa 5 00-001 Warszawa");
        assert!(format_clips(&clips, "").starts_with("1. https://example.com/flat\n2."));
        assert!(format_clips(&clips, "").ends_with("…"));
        assert!(format_clips(&clips, "nothing").starts_with("Nothing copied recently"));
        assert_eq!(format_clips(&[], ""), "Nothing copied yet.");
    }
}
