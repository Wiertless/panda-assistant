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

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn set_dark_mode(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_set("org.gnome.desktop.interface", "color-scheme", (if on { "prefer-dark" } else { "default" }).to_variant())?;
    Ok(format!("Dark mode {}.", on_off(on)))
}

pub fn set_do_not_disturb(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_set("org.gnome.desktop.notifications", "show-banners", (!on).to_variant())?;
    Ok(format!("Do Not Disturb {}.", on_off(on)))
}

pub fn set_night_light(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    gsettings_set("org.gnome.settings-daemon.plugins.color", "night-light-enabled", on.to_variant())?;
    Ok(format!("Night Light {}.", on_off(on)))
}

pub fn set_volume(_: &Context, args: &Args) -> ToolResult {
    let pct = arg_percent(args, "percent", 0)?;
    run(&["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{pct}%")], 15)?;
    Ok(format!("Volume set to {pct}%."))
}

pub fn set_mute(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "muted")?;
    run(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", if on { "1" } else { "0" }], 15)?;
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
    Err("no screen backlight found".into())
}

pub fn set_brightness(_: &Context, args: &Args) -> ToolResult {
    let pct = arg_percent(args, "percent", 1)?;
    let (dev, _, top) = backlight()?;
    let value = ((top as f64 * pct as f64 / 100.0).round() as u32).max(1);
    dbus_call(true, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session",
              "SetBrightness", Some(&("backlight", dev.as_str(), value).to_variant()), None, 5000)
        .map_err(|e| ToolError(e.message().to_string()))?;
    Ok(format!("Brightness set to {pct}%."))
}

pub fn set_wifi(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    run(&["nmcli", "radio", "wifi", on_off(on)], 15)?;
    Ok(format!("Wi-Fi {}.", on_off(on)))
}

pub fn set_bluetooth(_: &Context, args: &Args) -> ToolResult {
    let on = arg_bool(args, "enabled")?;
    set_property(false, "org.gnome.SettingsDaemon.Rfkill", "/org/gnome/SettingsDaemon/Rfkill",
                 "org.gnome.SettingsDaemon.Rfkill", "BluetoothAirplaneMode", (!on).to_variant())
        .map_err(|e| ToolError(e.message().to_string()))?;
    Ok(format!("Bluetooth {}.", on_off(on)))
}

pub const POWER_MODES: [(&str, &str); 3] = [("power-saver", "Power Saver"), ("balanced", "Balanced"), ("performance", "Performance")];

pub fn set_power_mode(_: &Context, args: &Args) -> ToolResult {
    let mode = arg_str(args, "mode").trim().to_lowercase().replace(' ', "-");
    let label = POWER_MODES.iter().find(|(m, _)| *m == mode).map(|(_, l)| *l)
        .ok_or("mode must be power-saver, balanced or performance")?;
    let mut last = String::from("no power profile service");
    // tuned-ppd and power-profiles-daemon both serve the new and the legacy name.
    for (name, path) in [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
                         ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")] {
        match set_property(true, name, path, name, "ActiveProfile", mode.to_variant()) {
            Ok(()) => return Ok(format!("Power mode set to {label}.")),
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
const BLOCKED_KEYS: [&str; 5] = ["enabled-extensions", "disabled-extensions", "disable-user-extensions",
                                 "development-tools", "allow-extension-installation"];
const BLOCKED_SCHEMAS: [&str; 2] = ["org.gnome.desktop.lockdown", "org.gnome.desktop.screensaver"];

fn allowed_schema(id: &str) -> bool {
    SETTINGS_PREFIXES.iter().any(|p| id.starts_with(p)) && !BLOCKED_SCHEMAS.iter().any(|b| id.starts_with(b))
}

fn schema(name: &str) -> Result<gio::SettingsSchema, ToolError> {
    let name = name.trim();
    if !allowed_schema(name) {
        return Err(format!("{} isn't a desktop setting the assistant may use",
                           if name.is_empty() { "that schema" } else { name }).into());
    }
    gio::SettingsSchemaSource::default().and_then(|s| s.lookup(name, true))
        .ok_or_else(|| format!("there is no settings schema called {name}").into())
}

fn key(schema: &gio::SettingsSchema, key: &str) -> Result<gio::SettingsSchemaKey, ToolError> {
    let key = key.trim();
    if BLOCKED_KEYS.contains(&key) {
        return Err(format!("{key} can't be changed by the assistant").into());
    }
    if !schema.has_key(key) {
        return Err(format!("{} has no key {key}", schema.id()).into());
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

pub fn change_setting(_: &Context, args: &Args) -> ToolResult {
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
    gio::Settings::new_full(&s, None::<&gio::SettingsBackend>, None).set_value(&k.name(), &value)
        .map_err(|e| ToolError(e.to_string()))?;
    gio::Settings::sync();
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
        return Err("GNOME Settings isn't installed".into());
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
