//! Saved networks (NetworkManager), paired Bluetooth devices (BlueZ),
//! Taildrop (Tailscale) and media players (MPRIS).

use std::collections::HashMap;
use std::time::Duration;

use glib::prelude::*;
use serde_json::Value;

use super::files::{home_path, short};
use super::pick;
use super::run::{dbus_call, run};
use super::settings::set_bluetooth;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

// -- networks ------------------------------------------------------------------------

fn nm_kind(t: &str) -> Option<&'static str> {
    Some(match t {
        "802-11-wireless" => "Wi-Fi",
        "802-3-ethernet" => "Wired",
        "vpn" | "wireguard" => "VPN",
        "gsm" => "Mobile",
        "bluetooth" => "Bluetooth tethering",
        _ => return None,
    })
}

/// One line of `nmcli -t` output: ':' separated, backslash escapes.
pub fn nm_fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for ch in line.chars() {
        if esc {
            out.last_mut().unwrap().push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == ':' {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(ch);
        }
    }
    out
}

struct Net {
    name: String,
    kind: &'static str,
    active: bool,
}

fn saved_networks() -> Result<Vec<Net>, ToolError> {
    Ok(run(&["nmcli", "-t", "-f", "NAME,TYPE,DEVICE", "connection", "show"], 15)?
        .lines()
        .filter_map(|l| {
            let f = nm_fields(l);
            let kind = nm_kind(f.get(1)?)?;
            (!f[0].is_empty()).then(|| Net { name: f[0].clone(), kind, active: f.get(2).is_some_and(|d| !d.is_empty()) })
        })
        .collect())
}

/// The names the user may mean ("my headphones", "the office Wi-Fi", "my
/// phone"), for the system prompt, so the model can match them without a
/// lookup: paired Bluetooth devices, Taildrop devices, saved networks. Each
/// part is left out when its service isn't there.
pub fn known_names() -> Vec<String> {
    let mut lines = vec![];
    if let Ok(objects) = bluez_objects() {
        let (devs, _) = bt_devices_from(&objects);
        if !devs.is_empty() {
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).take(10).collect();
            lines.push(format!("Paired Bluetooth devices: {}", names.join(", ")));
        }
    }
    if crate::flm::which("tailscale").is_some() && let Ok(peers) = tailnet_peers() && !peers.is_empty() {
        let names: Vec<String> = peers.iter().take(10)
            .map(|p| if p.os.is_empty() { p.name.clone() } else { format!("{} ({})", p.name, p.os) }).collect();
        lines.push(format!("The user's other devices (Taildrop): {}", names.join(", ")));
    }
    if let Ok(nets) = saved_networks() && !nets.is_empty() {
        let names: Vec<&str> = nets.iter().filter(|n| n.kind != "Wired").map(|n| n.name.as_str()).take(10).collect();
        if !names.is_empty() {
            lines.push(format!("Saved networks: {}", names.join(", ")));
        }
    }
    lines
}

pub fn list_networks(_: &Context, _: &Args) -> ToolResult {
    let mut nets = saved_networks()?;
    if nets.is_empty() {
        return Ok("No saved networks.".into());
    }
    let near: Vec<String> = run(&["nmcli", "-t", "-f", "SSID", "device", "wifi", "list", "--rescan", "no"], 15)
        .map(|o| o.lines().map(|l| nm_fields(l)[0].clone()).collect()).unwrap_or_default();
    nets.sort_by_key(|n| (!n.active, n.kind, n.name.to_lowercase()));
    Ok(nets.iter().map(|n| {
        let state = if n.active { "connected" } else if n.kind == "Wi-Fi" && near.contains(&n.name) { "in range" } else { "" };
        if state.is_empty() { format!("{} ({})", n.name, n.kind) } else { format!("{} ({}, {state})", n.name, n.kind) }
    }).collect::<Vec<_>>().join("\n"))
}

pub fn connect_network(_: &Context, args: &Args) -> ToolResult {
    let nets = saved_networks()?;
    let names: Vec<String> = nets.iter().map(|n| n.name.clone()).collect();
    let name = pick(&arg_str(args, "name"), &names, "saved network")?;
    if nets.iter().any(|n| n.name == name && n.active) {
        return Ok(format!("Already connected to {name}."));
    }
    run(&["nmcli", "--wait", "40", "connection", "up", "id", &name], 50)?;
    Ok(format!("Connected to {name}."))
}

pub fn disconnect_network(_: &Context, args: &Args) -> ToolResult {
    let names: Vec<String> = saved_networks()?.into_iter().filter(|n| n.active).map(|n| n.name).collect();
    let name = pick(&arg_str(args, "name"), &names, "connected network")?;
    run(&["nmcli", "connection", "down", "id", &name], 30)?;
    Ok(format!("Disconnected from {name}."))
}

// -- Bluetooth --------------------------------------------------------------------------

pub struct BtDevice {
    pub path: String,
    pub name: String,
    pub connected: bool,
}

type Props = HashMap<String, glib::Variant>;
type Objects = HashMap<String, HashMap<String, Props>>;
type RawObjects = HashMap<glib::variant::ObjectPath, HashMap<String, Props>>;

fn bluez_objects() -> Result<Objects, ToolError> {
    let reply = dbus_call(true, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None,
                          Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()), 5000)
        .map_err(|_| ToolError::cannot("Bluetooth isn't available (BlueZ isn't running)"))?;
    let (objects,): (RawObjects,) =
        reply.get().ok_or("unexpected answer from BlueZ")?;
    Ok(objects.into_iter().map(|(k, v)| (k.as_str().to_string(), v)).collect())
}

/// Paired devices, and whether any adapter is powered.
pub fn bt_devices_from(objects: &Objects) -> (Vec<BtDevice>, bool) {
    let flag = |props: &HashMap<String, glib::Variant>, k: &str| props.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false);
    let powered = objects.values().any(|i| i.get("org.bluez.Adapter1").is_some_and(|a| flag(a, "Powered")));
    let mut devs: Vec<BtDevice> = objects.iter()
        .filter_map(|(path, ifaces)| {
            let d = ifaces.get("org.bluez.Device1")?;
            if !(flag(d, "Paired") || flag(d, "Bonded")) {
                return None;
            }
            let name = ["Alias", "Name", "Address"].iter().find_map(|k| d.get(*k)?.get::<String>()).unwrap_or("?".into());
            Some(BtDevice { path: path.clone(), name, connected: flag(d, "Connected") })
        })
        .collect();
    devs.sort_by_key(|d| (!d.connected, d.name.to_lowercase()));
    (devs, powered)
}

fn bt_call(path: &str, method: &str) -> Result<(), ToolError> {
    dbus_call(true, "org.bluez", path, "org.bluez.Device1", method, None, None, 30000).map(|_| ()).map_err(|e| {
        let msg = e.message().rsplit(':').next().unwrap_or("").trim().to_string();
        if msg.contains("page-timeout") || msg.contains("Host is down") || msg.to_lowercase().contains("not available") {
            ToolError("the device didn't answer; make sure it's on and nearby (for earbuds, open the case)".into())
        } else {
            ToolError(if msg.is_empty() { format!("{method} failed") } else { msg })
        }
    })
}

pub fn list_bluetooth(_: &Context, _: &Args) -> ToolResult {
    let (devs, powered) = bt_devices_from(&bluez_objects()?);
    if devs.is_empty() {
        return Ok("No paired Bluetooth devices.".into());
    }
    let list: Vec<String> = devs.iter()
        .map(|d| format!("{} ({})", d.name, if d.connected { "connected" } else { "not connected" })).collect();
    Ok(format!("{}{}", if powered { "" } else { "Bluetooth is off.\n" }, list.join("\n")))
}

pub fn connect_bluetooth(ctx: &Context, args: &Args) -> ToolResult {
    let (devs, powered) = bt_devices_from(&bluez_objects()?);
    let names: Vec<String> = devs.iter().map(|d| d.name.clone()).collect();
    let name = pick(&arg_str(args, "device"), &names, "paired device")?;
    let dev = devs.iter().find(|d| d.name == name).unwrap();
    if dev.connected {
        return Ok(format!("{name} is already connected."));
    }
    if !powered {
        let on: Args = serde_json::from_value(serde_json::json!({"enabled": true})).unwrap();
        set_bluetooth(ctx, &on)?;
        let mut ready = false;
        for _ in 0..20 {
            if bt_devices_from(&bluez_objects()?).1 {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !ready {
            return Err("couldn't turn Bluetooth on".into());
        }
    }
    bt_call(&dev.path, "Connect")?;
    Ok(format!("Connected {name}."))
}

pub fn disconnect_bluetooth(_: &Context, args: &Args) -> ToolResult {
    let (devs, _) = bt_devices_from(&bluez_objects()?);
    let names: Vec<String> = devs.iter().filter(|d| d.connected).map(|d| d.name.clone()).collect();
    let name = pick(&arg_str(args, "device"), &names, "connected device")?;
    bt_call(&devs.iter().find(|d| d.name == name).unwrap().path, "Disconnect")?;
    Ok(format!("Disconnected {name}."))
}

// -- Taildrop -----------------------------------------------------------------------------

struct Peer {
    name: String,
    dns: String,
    os: String,
    online: bool,
}

fn tailnet_peers() -> Result<Vec<Peer>, ToolError> {
    let status: Value = serde_json::from_str(&run(&["tailscale", "status", "--json"], 15)?)
        .map_err(|_| ToolError("couldn't read Tailscale's status".into()))?;
    Ok(status["Peer"].as_object().into_iter().flatten().map(|(_, p)| Peer {
        name: p["HostName"].as_str().unwrap_or("").trim().to_string(),
        dns: p["DNSName"].as_str().unwrap_or("").split('.').next().unwrap_or("").to_string(),
        os: p["OS"].as_str().unwrap_or("").to_string(),
        online: p["Online"].as_bool().unwrap_or(false),
    }).collect())
}

pub fn list_devices(_: &Context, _: &Args) -> ToolResult {
    let mut peers = tailnet_peers()?;
    if peers.is_empty() {
        return Ok("No other devices on the tailnet.".into());
    }
    peers.sort_by_key(|p| (!p.online, p.name.to_lowercase()));
    Ok(peers.iter().map(|p| format!("{} ({}, {})", p.name, if p.os.is_empty() { "?" } else { &p.os },
                                    if p.online { "online" } else { "offline" })).collect::<Vec<_>>().join("\n"))
}

pub fn send_file(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    if !path.is_file() {
        return Err(format!("{} isn't a file; Taildrop sends files, not folders", short(&path)).into());
    }
    taildrop(&path, &arg_str(args, "device"))
}

/// A note made of text (an answer, a list, an address) sent as a .txt file,
/// which phones open in their files or notes app.
/// A file the "text" really means: a short note that's just a path (or a
/// path with a few words around it) to a file that exists. Models reach for
/// send_text with what find_files returned; the user wants the file.
pub fn file_in_note(text: &str) -> Option<std::path::PathBuf> {
    static PATH: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?:^|\s)((?:~/|/)[^\n]*?\.[A-Za-z0-9]{1,5})\b").unwrap());
    if text.chars().count() > 300 || text.lines().filter(|l| !l.trim().is_empty()).count() > 3 {
        return None;
    }
    let files: Vec<std::path::PathBuf> = PATH.captures_iter(text)
        .filter_map(|c| home_path(c[1].trim(), true).ok()).filter(|p| p.is_file()).collect();
    (files.len() == 1).then(|| files[0].clone())
}

pub fn send_text(_: &Context, args: &Args) -> ToolResult {
    let text = arg_str(args, "text");
    if text.trim().is_empty() {
        return Err("give the text to send".into());
    }
    if let Some(file) = file_in_note(&text) {
        return taildrop(&file, &arg_str(args, "device"))
            .map(|sent| format!("{sent} (the file itself, not a note with its path)"));
    }
    let raw = arg_str(args, "title");
    let title = if raw.trim().is_empty() { text.lines().next().unwrap_or("Note").to_string() } else { raw };
    let name: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
    let dir = glib::user_cache_dir().join("panda-assistant/notes");
    std::fs::create_dir_all(&dir).map_err(|e| ToolError(e.to_string()))?;
    let path = dir.join(format!("{}.txt", if name.is_empty() { "Note" } else { name.trim() }));
    std::fs::write(&path, format!("{}\n", text.trim_end())).map_err(|e| ToolError(e.to_string()))?;
    let sent = taildrop(&path, &arg_str(args, "device"));
    let _ = std::fs::remove_file(&path);
    sent
}

fn taildrop(path: &std::path::Path, wanted: &str) -> ToolResult {
    let wanted = wanted.to_string();
    let peers = tailnet_peers()?;
    let names: Vec<String> = peers.iter().map(|p| p.name.clone()).collect();
    let by_dns = peers.iter().find(|p| !p.dns.is_empty() && p.dns.eq_ignore_ascii_case(wanted.trim()));
    let peer = match by_dns {
        Some(p) => p,
        None => {
            let name = pick(&wanted, &names, "device on the tailnet").map_err(|e| {
                let all: Vec<String> = peers.iter()
                    .map(|p| format!("{} ({}{})", p.name, p.os, if p.online { "" } else { ", offline" })).collect();
                ToolError(format!("{} Devices: {}", e.0.split(" (known:").next().unwrap_or(&e.0),
                                  if all.is_empty() { "none".into() } else { all.join(", ") }))
            })?;
            peers.iter().find(|p| p.name == name).unwrap()
        }
    };
    if !peer.online {
        return Err(format!("{} is offline", peer.name).into());
    }
    let target = format!("{}:", if peer.dns.is_empty() { &peer.name } else { &peer.dns });
    let out = super::run::output(&["tailscale", "file", "cp", &path.to_string_lossy(), &target], Duration::from_secs(600), None)?;
    if out.code != 0 {
        let err = format!("{}{}", out.stderr, out.stdout).trim().to_string();
        if err.to_lowercase().contains("access denied") || err.to_lowercase().contains("operator") {
            return Err("Tailscale won't let this user send files yet. Run once in a terminal: \
sudo tailscale set --operator=$USER".into());
        }
        return Err(if err.is_empty() { "tailscale file cp failed".to_string() } else { err }.into());
    }
    Ok(format!("Sent {} to {} with Taildrop.", path.file_name().unwrap().to_string_lossy(), peer.name))
}

// -- media (MPRIS) ---------------------------------------------------------------------------

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone)]
pub struct MediaPlayer {
    pub bus: String,
    pub name: String,
    pub status: String,
    pub title: String,
    pub artist: String,
}

fn players() -> Result<Vec<MediaPlayer>, ToolError> {
    let reply = dbus_call(false, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames",
                          None, Some(glib::VariantTy::new("(as)").unwrap()), 3000)
        .map_err(|e| ToolError(e.message().to_string()))?;
    let (names,): (Vec<String>,) = reply.get().unwrap_or_default();
    let mut out = vec![];
    for bus in names.into_iter().filter(|n| n.starts_with(MPRIS_PREFIX)) {
        let Ok(props) = dbus_call(false, &bus, MPRIS_PATH, "org.freedesktop.DBus.Properties", "GetAll",
                                  Some(&(PLAYER,).to_variant()), Some(glib::VariantTy::new("(a{sv})").unwrap()), 2000) else { continue };
        let (props,): (HashMap<String, glib::Variant>,) = props.get().unwrap_or_default();
        let ident = dbus_call(false, &bus, MPRIS_PATH, "org.freedesktop.DBus.Properties", "Get",
                              Some(&("org.mpris.MediaPlayer2", "Identity").to_variant()), Some(glib::VariantTy::new("(v)").unwrap()), 2000)
            .ok().and_then(|v| v.child_value(0).as_variant()?.get::<String>())
            .unwrap_or_else(|| bus[MPRIS_PREFIX.len()..].to_string());
        let meta: HashMap<String, glib::Variant> = props.get("Metadata").and_then(|m| m.get()).unwrap_or_default();
        let get_s = |k: &str| meta.get(k).and_then(|v| v.get::<String>()).unwrap_or_default();
        let artist = meta.get("xesam:artist").and_then(|v| v.get::<Vec<String>>()).map(|a| a.join(", ")).unwrap_or_default();
        out.push(MediaPlayer {
            name: ident,
            status: props.get("PlaybackStatus").and_then(|v| v.get::<String>()).unwrap_or("Stopped".into()),
            title: get_s("xesam:title"),
            artist,
            bus,
        });
    }
    Ok(out)
}

pub fn choose_player(players: &[MediaPlayer], name: &str) -> Result<MediaPlayer, ToolError> {
    if players.is_empty() {
        return Err(ToolError::cannot("no music or video player is running. Open one first (\"open Spotify\"), or ask to play a song from your library"));
    }
    if !name.trim().is_empty() {
        let names: Vec<String> = players.iter().map(|p| p.name.clone()).collect();
        let n = pick(name, &names, "player")?;
        return Ok(players.iter().find(|p| p.name == n).unwrap().clone());
    }
    // The one playing, else a paused one, else any.
    Ok(["Playing", "Paused"].iter().find_map(|s| players.iter().find(|p| p.status == *s)).unwrap_or(&players[0]).clone())
}

pub fn describe(p: &MediaPlayer) -> String {
    let what: Vec<&str> = [p.artist.as_str(), p.title.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
    format!("{}: {}, {}", p.name, p.status.to_lowercase(), if what.is_empty() { "nothing".into() } else { what.join(" – ") })
}

pub fn media_control(_: &Context, args: &Args) -> ToolResult {
    let action = arg_str(args, "action").trim().to_lowercase().replace(['-', ' '], "_");
    let action = if action.is_empty() { "status".to_string() } else { action };
    let all = players()?;
    if matches!(action.as_str(), "status" | "now_playing" | "what") {
        return Ok(if all.is_empty() { "No media player is running.".into() } else {
            all.iter().map(describe).collect::<Vec<_>>().join("\n")
        });
    }
    let method = match action.as_str() {
        "play" => "Play", "pause" => "Pause", "play_pause" | "toggle" => "PlayPause",
        "next" | "next_track" | "skip" => "Next", "previous" | "previous_track" | "back" => "Previous", "stop" => "Stop",
        _ => return Err("action must be play, pause, play_pause, next, previous, stop or status".into()),
    };
    let player = choose_player(&all, &arg_str(args, "player"))?;
    dbus_call(false, &player.bus, MPRIS_PATH, PLAYER, method, None, None, 3000)
        .map_err(|e| ToolError(format!("{} refused: {}", player.name, e.message())))?;
    Ok(format!("{}: {}.", player.name, if method == "PlayPause" { "play/pause".into() } else { method.to_lowercase() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_that_is_a_path_means_the_file() {
        let _h = crate::store::testenv::temp_home();
        let home = crate::tools::files::real_home();
        std::fs::create_dir_all(home.join("Dokumenty")).unwrap();
        std::fs::write(home.join("Dokumenty/lease-2026.pdf"), "pdf").unwrap();
        let file = Some(home.join("Dokumenty/lease-2026.pdf"));
        assert_eq!(file_in_note("~/Dokumenty/lease-2026.pdf"), file);
        assert_eq!(file_in_note("Here is the file: ~/Dokumenty/lease-2026.pdf"), file);
        assert_eq!(file_in_note(&format!("{}/Dokumenty/lease-2026.pdf", home.display())), file);
        assert_eq!(file_in_note("~/Dokumenty/missing.pdf"), None, "only files that exist");
        assert_eq!(file_in_note("Milk, eggs, bread"), None);
        let long = format!("~/Dokumenty/lease-2026.pdf\n{}", "a long answer that mentions a file\n".repeat(5));
        assert_eq!(file_in_note(&long), None, "a real note that mentions a path stays a note");
    }

    #[test]
    fn nmcli_fields() {
        assert_eq!(nm_fields(r"Home\:5G:802-11-wireless:wlp1s0"), ["Home:5G", "802-11-wireless", "wlp1s0"]);
        assert_eq!(nm_fields("Cafe:vpn:"), ["Cafe", "vpn", ""]);
    }

    #[test]
    fn bluetooth_devices() {
        let v = |b: bool| b.to_variant();
        let mut objects: Objects = HashMap::new();
        objects.insert("/org/bluez/hci0".into(), [("org.bluez.Adapter1".to_string(), [("Powered".to_string(), v(false))].into())].into());
        objects.insert("/org/bluez/hci0/dev_1".into(), [("org.bluez.Device1".to_string(), [
            ("Alias".to_string(), "Wiertless's AirPods".to_variant()), ("Paired".to_string(), v(true)),
            ("Connected".to_string(), v(true))].into())].into());
        objects.insert("/org/bluez/hci0/dev_2".into(), [("org.bluez.Device1".to_string(), [
            ("Alias".to_string(), "Stranger TV".to_variant()), ("Paired".to_string(), v(false))].into())].into());
        let (devs, powered) = bt_devices_from(&objects);
        assert!(!powered);
        assert_eq!(devs.len(), 1);
        assert_eq!((devs[0].name.as_str(), devs[0].connected), ("Wiertless's AirPods", true));
    }

    #[test]
    fn players() {
        let p = |name: &str, status: &str| MediaPlayer { bus: format!("{MPRIS_PREFIX}{name}"), name: name.into(),
                                                        status: status.into(), title: "Song".into(), artist: "Band".into() };
        let list = vec![p("VLC", "Paused"), p("Spotify", "Playing")];
        assert_eq!(choose_player(&list, "").unwrap().name, "Spotify");
        assert_eq!(choose_player(&list, "vlc").unwrap().name, "VLC");
        assert!(choose_player(&[], "").is_err());
        assert_eq!(describe(&list[1]), "Spotify: playing, Band – Song");
    }
}
