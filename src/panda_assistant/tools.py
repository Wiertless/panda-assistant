"""Desktop actions the model can call.

Each tool has a JSON schema for the model, a risk level and a function.
"safe" tools run straight away; "confirm" tools wait for the user to press
Run. Arguments come from a model, so every tool validates them and nothing
goes through a shell.
"""

from __future__ import annotations

import datetime
import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlparse

SAFE = "safe"
CONFIRM = "confirm"


class ToolError(Exception):
    """A tool refused its arguments or failed; the message goes back to the model."""


@dataclass
class Context:
    """What tools may need from the app (kept out of the tools for testing)."""

    copy_to_clipboard: Callable[[str], None] | None = None


@dataclass
class Tool:
    name: str
    description: str
    parameters: dict
    risk: str
    run: Callable[[Context, dict], str]
    summary: Callable[[dict], str]

    def schema(self) -> dict:
        return {"type": "function", "function": {
            "name": self.name, "description": self.description, "parameters": self.parameters}}


def _obj(props: dict | None = None, required: list[str] | None = None) -> dict:
    return {"type": "object", "properties": props or {}, "required": required or []}


def _bool(args: dict, key: str) -> bool:
    value = args.get(key)
    if isinstance(value, bool):
        return value
    if isinstance(value, str) and value.lower() in ("true", "false", "on", "off", "yes", "no"):
        return value.lower() in ("true", "on", "yes")
    raise ToolError(f"'{key}' must be true or false")


def _percent(args: dict, key: str, low: int = 0) -> int:
    try:
        value = int(round(float(args.get(key))))
    except (TypeError, ValueError):
        raise ToolError(f"'{key}' must be a number from {low} to 100") from None
    if not low <= value <= 100:
        raise ToolError(f"'{key}' must be from {low} to 100")
    return value


def _run(cmd: list[str]) -> str:
    if not shutil.which(cmd[0]):
        raise ToolError(f"{cmd[0]} is not installed")
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
    if proc.returncode != 0:
        raise ToolError((proc.stderr or proc.stdout).strip() or f"{cmd[0]} failed")
    return proc.stdout.strip()


def _gio():
    from gi.repository import Gio  # imported lazily so tests run without GObject
    return Gio


# -- opening things -----------------------------------------------------------

# Everyday words for kinds of apps -> the content type whose default handler
# is "that" app, as in GNOME Settings -> Apps -> Default Apps.
APP_ROLES = {
    "browser": "x-scheme-handler/https", "email": "x-scheme-handler/mailto",
    "files": "inode/directory", "calendar": "text/calendar", "music": "audio/mpeg",
    "video": "video/mp4", "photos": "image/jpeg", "text editor": "text/plain",
    "pdf": "application/pdf", "terminal": None, "settings": None,
}
ROLE_WORDS = {
    "browser": ("browser", "web browser", "internet", "web"),
    "email": ("email", "e-mail", "mail", "mail app", "email app", "inbox"),
    "files": ("files", "file manager", "file browser", "folders", "explorer", "finder"),
    "calendar": ("calendar", "agenda"),
    "music": ("music", "music player", "audio player"),
    "video": ("video", "videos", "video player", "movies"),
    "photos": ("photos", "pictures", "images", "image viewer", "photo viewer"),
    "text editor": ("text editor", "editor", "notepad"),
    "pdf": ("pdf", "pdf viewer", "document viewer", "documents"),
    "terminal": ("terminal", "console", "command line", "shell"),
    "settings": ("settings", "system settings", "preferences", "control panel"),
}
FALLBACK_IDS = {"terminal": ("org.gnome.Ptyxis", "org.gnome.Console", "org.gnome.Terminal"),
                "settings": ("org.gnome.Settings",)}


def role_for(name: str) -> str | None:
    wanted = name.strip().lower().removeprefix("my ").removeprefix("the ").removesuffix(" app")
    for role, words in ROLE_WORDS.items():
        if wanted in words:
            return role
    return None


def default_app(role: str):
    Gio = _gio()
    ctype = APP_ROLES.get(role)
    app = None
    if ctype and ctype.startswith("x-scheme-handler/"):
        app = Gio.AppInfo.get_default_for_uri_scheme(ctype.split("/", 1)[1])
    elif ctype:
        app = Gio.AppInfo.get_default_for_type(ctype, False)
    if app is None:
        for app_id in FALLBACK_IDS.get(role, ()):
            try:
                app = Gio.DesktopAppInfo.new(app_id + ".desktop")
            except TypeError:
                app = None
            if app:
                break
    return app


def default_apps() -> dict[str, str]:
    """{role: app name} for the roles that have a default app here."""
    out = {}
    for role in APP_ROLES:
        try:
            app = default_app(role)
        except Exception:  # noqa: BLE001 - no GIO or no desktop session
            app = None
        if app:
            out[role] = app.get_display_name()
    return out


def _find_app(name: str):
    Gio = _gio()
    wanted = name.strip().lower()
    if not wanted:
        raise ToolError("no app name given")
    role = role_for(wanted)
    if role:
        app = default_app(role)
        if app:
            return app
    best = None
    for app in Gio.AppInfo.get_all():
        if not app.should_show():
            continue
        names = [(app.get_display_name() or "").lower(), (app.get_id() or "").lower().removesuffix(".desktop")]
        if wanted in names:
            return app
        extra = []
        if isinstance(app, Gio.DesktopAppInfo):
            extra = [(app.get_generic_name() or "").lower()] + [k.lower() for k in (app.get_keywords() or [])]
        if best is None and (any(wanted in n for n in names) or wanted in extra):
            best = app
    if best is None:
        raise ToolError(f"no installed app matches '{name}'")
    return best


def open_app(ctx: Context, args: dict) -> str:
    app = _find_app(str(args.get("name", "")))
    app.launch([], None)
    return f"Opened {app.get_display_name()}."


_SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:(?!\d)")


def check_url(raw: str) -> str:
    """Return a safe http(s) URL, adding https:// to bare host names."""
    url = raw.strip()
    if _SCHEME.match(url):
        if not url.lower().startswith(("http://", "https://")):
            raise ToolError("only http(s) web addresses can be opened")
    else:
        url = "https://" + url
    parts = urlparse(url)
    if parts.scheme not in ("http", "https") or not parts.hostname or any(c.isspace() for c in url):
        raise ToolError("that isn't a valid web address")
    return url


def open_url(ctx: Context, args: dict) -> str:
    url = check_url(str(args.get("url", "")))
    _gio().AppInfo.launch_default_for_uri(url, None)
    return f"Opened {url}."


def _home_path(raw: str) -> Path:
    path = Path(os.path.expanduser(raw.strip())).resolve()
    home = Path.home().resolve()
    if path != home and home not in path.parents:
        raise ToolError("only files in your home folder can be opened")
    if not path.exists():
        raise ToolError(f"{path} doesn't exist")
    return path


def open_file(ctx: Context, args: dict) -> str:
    path = _home_path(str(args.get("path", "")))
    _gio().AppInfo.launch_default_for_uri(path.as_uri(), None)
    return f"Opened {path}."


# -- reading state --------------------------------------------------------------

def get_battery(ctx: Context, args: dict) -> str:
    batteries = sorted(Path("/sys/class/power_supply").glob("BAT*"))
    if not batteries:
        return "No battery found."
    parts = []
    for bat in batteries:
        read = lambda f: (bat / f).read_text().strip() if (bat / f).exists() else "?"
        parts.append(f"{bat.name}: {read('capacity')}% ({read('status')})")
    return "; ".join(parts)


def get_system_info(ctx: Context, args: dict) -> str:
    info = {}
    try:
        for line in Path("/etc/os-release").read_text().splitlines():
            if line.startswith("PRETTY_NAME="):
                info["os"] = line.split("=", 1)[1].strip('"')
    except OSError:
        pass
    info["kernel"] = platform.release()
    info["host"] = platform.node()
    try:
        uptime = float(Path("/proc/uptime").read_text().split()[0])
        info["uptime"] = str(datetime.timedelta(seconds=int(uptime)))
        mem = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines() if ":" in line)
        total = int(mem["MemTotal"].split()[0]) // 1024
        avail = int(mem["MemAvailable"].split()[0]) // 1024
        info["memory"] = f"{total - avail} MiB used of {total} MiB"
    except (OSError, KeyError, ValueError):
        pass
    for label, where in (("disk /", "/"), ("disk home", str(Path.home()))):
        try:
            du = shutil.disk_usage(where)
            info[label] = f"{du.free // 2**30} GiB free of {du.total // 2**30} GiB"
        except OSError:
            pass
    return "; ".join(f"{k}: {v}" for k, v in info.items())


def get_datetime(ctx: Context, args: dict) -> str:
    return datetime.datetime.now().astimezone().strftime("%A %Y-%m-%d %H:%M %Z")


def _gsettings_get(schema: str, key: str):
    Gio = _gio()
    return Gio.Settings.new(schema).get_value(key).unpack()


def get_status(ctx: Context, args: dict) -> str:
    out = []
    try:
        out.append(f"dark mode: {'on' if _gsettings_get('org.gnome.desktop.interface', 'color-scheme') == 'prefer-dark' else 'off'}")
        out.append(f"do not disturb: {'off' if _gsettings_get('org.gnome.desktop.notifications', 'show-banners') else 'on'}")
    except Exception:  # noqa: BLE001 - schema may be missing outside GNOME
        pass
    for label, cmd in (("wifi", ["nmcli", "radio", "wifi"]), ("volume", ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])):
        try:
            out.append(f"{label}: {_run(cmd)}")
        except (ToolError, OSError, subprocess.TimeoutExpired):
            pass
    try:
        cur, top = _backlight()
        out.append(f"brightness: {round(100 * cur / top)}%")
    except ToolError:
        pass
    return "; ".join(out) or "No status available."


def web_search(ctx: Context, args: dict) -> str:
    from . import web
    try:
        return web.web_search(str(args.get("query", "")))
    except web.WebError as e:
        raise ToolError(str(e)) from None


def read_web_page(ctx: Context, args: dict) -> str:
    from . import web
    try:
        return web.read_web_page(check_url(str(args.get("url", ""))))
    except web.WebError as e:
        raise ToolError(str(e)) from None


def copy_to_clipboard(ctx: Context, args: dict) -> str:
    text = str(args.get("text", ""))
    if not text:
        raise ToolError("nothing to copy")
    if not ctx.copy_to_clipboard:
        raise ToolError("clipboard isn't available")
    ctx.copy_to_clipboard(text)
    return "Copied to the clipboard."


# -- changing settings ----------------------------------------------------------

def _gsettings_set(schema: str, key: str, value) -> None:
    from gi.repository import GLib
    settings = _gio().Settings.new(schema)
    settings.set_value(key, GLib.Variant(settings.get_value(key).get_type_string(), value))
    _gio().Settings.sync()


def set_dark_mode(ctx: Context, args: dict) -> str:
    on = _bool(args, "enabled")
    _gsettings_set("org.gnome.desktop.interface", "color-scheme", "prefer-dark" if on else "default")
    return f"Dark mode {'on' if on else 'off'}."


def set_do_not_disturb(ctx: Context, args: dict) -> str:
    on = _bool(args, "enabled")
    _gsettings_set("org.gnome.desktop.notifications", "show-banners", not on)
    return f"Do Not Disturb {'on' if on else 'off'}."


def set_volume(ctx: Context, args: dict) -> str:
    pct = _percent(args, "percent")
    _run(["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", f"{pct}%"])
    return f"Volume set to {pct}%."


def set_mute(ctx: Context, args: dict) -> str:
    on = _bool(args, "muted")
    _run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1" if on else "0"])
    return "Muted." if on else "Unmuted."


def _backlight() -> tuple[int, int]:
    for dev in sorted(Path("/sys/class/backlight").glob("*")):
        try:
            return int((dev / "brightness").read_text()), int((dev / "max_brightness").read_text())
        except (OSError, ValueError):
            continue
    raise ToolError("no screen backlight found")


def set_brightness(ctx: Context, args: dict) -> str:
    from gi.repository import GLib
    pct = _percent(args, "percent", low=1)
    devices = sorted(Path("/sys/class/backlight").glob("*"))
    if not devices:
        raise ToolError("no screen backlight found")
    dev = devices[0]
    top = int((dev / "max_brightness").read_text())
    value = max(1, round(top * pct / 100))
    bus = _gio().bus_get_sync(_gio().BusType.SYSTEM, None)
    bus.call_sync("org.freedesktop.login1", "/org/freedesktop/login1/session/auto",
                  "org.freedesktop.login1.Session", "SetBrightness",
                  GLib.Variant("(ssu)", ("backlight", dev.name, value)), None,
                  _gio().DBusCallFlags.NONE, 5000, None)
    return f"Brightness set to {pct}%."


def set_wifi(ctx: Context, args: dict) -> str:
    on = _bool(args, "enabled")
    _run(["nmcli", "radio", "wifi", "on" if on else "off"])
    return f"Wi-Fi {'on' if on else 'off'}."


POWER_MODES = {"power-saver": "Power Saver", "balanced": "Balanced", "performance": "Performance"}


def set_power_mode(ctx: Context, args: dict) -> str:
    from gi.repository import GLib
    mode = str(args.get("mode", "")).strip().lower().replace(" ", "-")
    if mode not in POWER_MODES:
        raise ToolError("mode must be power-saver, balanced or performance")
    bus = _gio().bus_get_sync(_gio().BusType.SYSTEM, None)
    last = None
    # tuned-ppd and power-profiles-daemon both serve the new and the legacy name.
    for name, path in (("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
                       ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")):
        try:
            bus.call_sync(name, path, "org.freedesktop.DBus.Properties", "Set",
                          GLib.Variant("(ssv)", (name, "ActiveProfile", GLib.Variant("s", mode))),
                          None, _gio().DBusCallFlags.NONE, 5000, None)
            return f"Power mode set to {POWER_MODES[mode]}."
        except GLib.Error as e:
            last = e
    raise ToolError(f"couldn't change the power mode: {last.message if last else 'no power profile service'}")


def set_night_light(ctx: Context, args: dict) -> str:
    on = _bool(args, "enabled")
    _gsettings_set("org.gnome.settings-daemon.plugins.color", "night-light-enabled", on)
    return f"Night Light {'on' if on else 'off'}."


# Generic GNOME settings. Only desktop schemas, and never the keys that load
# code (extensions) or lock the user out (lockdown, screensaver lock).
SETTINGS_PREFIXES = ("org.gnome.desktop.", "org.gnome.settings-daemon.plugins.", "org.gnome.mutter",
                     "org.gnome.shell.keybindings", "org.gnome.nautilus.")
BLOCKED_KEYS = {"enabled-extensions", "disabled-extensions", "disable-user-extensions", "development-tools",
                "allow-extension-installation"}
BLOCKED_SCHEMAS = ("org.gnome.desktop.lockdown", "org.gnome.desktop.screensaver")


def _schema(name: str):
    name = name.strip()
    if not name.startswith(SETTINGS_PREFIXES) or name.startswith(BLOCKED_SCHEMAS):
        raise ToolError(f"{name or 'that schema'} isn't a desktop setting Panda may use")
    src = _gio().SettingsSchemaSource.get_default()
    schema = src.lookup(name, True) if src else None
    if schema is None:
        raise ToolError(f"there is no settings schema called {name}")
    return schema


def _key(schema, key: str):
    key = key.strip()
    if key in BLOCKED_KEYS:
        raise ToolError(f"{key} can't be changed by Panda")
    if not schema.has_key(key):
        raise ToolError(f"{schema.get_id()} has no key {key}")
    return schema.get_key(key)


def find_settings(ctx: Context, args: dict) -> str:
    words = [w for w in str(args.get("query", "")).lower().split() if w]
    if not words:
        raise ToolError("say what to look for, e.g. 'font scaling'")
    src = _gio().SettingsSchemaSource.get_default()
    hits = []
    for sid in sorted(src.list_schemas(True)[0]):
        if not sid.startswith(SETTINGS_PREFIXES) or sid.startswith(BLOCKED_SCHEMAS):
            continue
        schema = src.lookup(sid, True)
        for key in schema.list_keys():
            k = schema.get_key(key)
            text = f"{sid} {key} {k.get_summary() or ''}".lower()
            if all(w in text for w in words) and key not in BLOCKED_KEYS:
                hits.append(f"{sid} {key}: {k.get_summary() or ''}")
    return "\n".join(hits[:12]) if hits else "No matching settings."


def get_setting(ctx: Context, args: dict) -> str:
    schema = _schema(str(args.get("schema", "")))
    key = _key(schema, str(args.get("key", ""))).get_name()
    value = _gio().Settings.new_full(schema, None, None).get_value(key)
    return f"{schema.get_id()} {key} = {value.print_(False)}"


def change_setting(ctx: Context, args: dict) -> str:
    from gi.repository import GLib
    schema = _schema(str(args.get("schema", "")))
    key = _key(schema, str(args.get("key", "")))
    raw = str(args.get("value", "")).strip()
    vtype = key.get_value_type()
    try:
        value = GLib.Variant.parse(vtype, raw, None, None)
    except GLib.Error:
        if vtype.dup_string() == "s":
            value = GLib.Variant("s", raw.strip("'\""))
        else:
            raise ToolError(f"{raw} isn't a valid {vtype.dup_string()} value") from None
    if not key.range_check(value):
        raise ToolError(f"{raw} isn't an allowed value for {key.get_name()}")
    settings = _gio().Settings.new_full(schema, None, None)
    settings.set_value(key.get_name(), value)
    _gio().Settings.sync()
    return f"Set {schema.get_id()} {key.get_name()} to {value.print_(False)}."


SETTINGS_PANELS = {"wifi", "network", "bluetooth", "display", "sound", "power", "background", "appearance",
                   "notifications", "search", "multitasking", "applications", "privacy", "online-accounts",
                   "sharing", "mouse", "keyboard", "printers", "color", "accessibility", "system", "users",
                   "datetime", "region", "wacom", "about"}


def open_settings(ctx: Context, args: dict) -> str:
    panel = str(args.get("panel", "")).strip().lower()
    if panel not in SETTINGS_PANELS:
        raise ToolError("unknown Settings page; one of: " + ", ".join(sorted(SETTINGS_PANELS)))
    if not shutil.which("gnome-control-center"):
        raise ToolError("GNOME Settings isn't installed")
    subprocess.Popen(["gnome-control-center", panel], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)
    return f"Opened Settings at {panel}."


def set_bluetooth(ctx: Context, args: dict) -> str:
    from gi.repository import GLib
    on = _bool(args, "enabled")
    bus = _gio().bus_get_sync(_gio().BusType.SESSION, None)
    bus.call_sync("org.gnome.SettingsDaemon.Rfkill", "/org/gnome/SettingsDaemon/Rfkill",
                  "org.freedesktop.DBus.Properties", "Set",
                  GLib.Variant("(ssv)", ("org.gnome.SettingsDaemon.Rfkill", "BluetoothAirplaneMode",
                                         GLib.Variant("b", not on))),
                  None, _gio().DBusCallFlags.NONE, 5000, None)
    return f"Bluetooth {'on' if on else 'off'}."


def _onoff(key: str, what: str) -> Callable[[dict], str]:
    def summary(args: dict) -> str:
        try:
            return f"Turn {what} {'on' if _bool(args, key) else 'off'}"
        except ToolError:
            return f"Change {what}"
    return summary


ENABLED = _obj({"enabled": {"type": "boolean"}}, ["enabled"])

TOOLS: list[Tool] = [
    Tool("open_app", "Open an app by its name ('Zed') or by what it is: browser, email, files, calendar, "
         "music, video, photos, text editor, pdf, terminal or settings opens the user's default app for it.",
         _obj({"name": {"type": "string", "description": "App name or kind of app"}}, ["name"]), SAFE, open_app,
         lambda a: f"Open {a.get('name', 'an app')}"),
    Tool("open_url", "Open a web address in the default browser.",
         _obj({"url": {"type": "string"}}, ["url"]), SAFE, open_url, lambda a: f"Open {a.get('url', 'a link')}"),
    Tool("open_file", "Open a file or folder in the user's home folder with its default app.",
         _obj({"path": {"type": "string", "description": "Path, may start with ~"}}, ["path"]), SAFE, open_file,
         lambda a: f"Open {a.get('path', 'a file')}"),
    Tool("get_battery", "Get the battery charge level and charging state.", _obj(), SAFE, get_battery,
         lambda a: "Read battery"),
    Tool("get_system_info", "Get OS, kernel, uptime, memory and disk space.", _obj(), SAFE, get_system_info,
         lambda a: "Read system info"),
    Tool("get_datetime", "Get the current local date and time.", _obj(), SAFE, get_datetime,
         lambda a: "Read the time"),
    Tool("get_status", "Get dark mode, Do Not Disturb, Wi-Fi, volume and brightness state.", _obj(), SAFE,
         get_status, lambda a: "Read settings"),
    Tool("web_search", "Search the web for current information, facts, news, prices or how-tos. "
         "Returns titles, links and short snippets.",
         _obj({"query": {"type": "string", "description": "What to search for"}}, ["query"]), SAFE, web_search,
         lambda a: f"Search the web for “{a.get('query', '')}”"),
    Tool("read_web_page", "Read the text of a web page, e.g. a search result, to answer from it.",
         _obj({"url": {"type": "string"}}, ["url"]), SAFE, read_web_page, lambda a: f"Read {a.get('url', 'a page')}"),
    Tool("copy_to_clipboard", "Copy text to the clipboard.", _obj({"text": {"type": "string"}}, ["text"]),
         SAFE, copy_to_clipboard, lambda a: "Copy to clipboard"),
    Tool("set_dark_mode", "Turn the dark style on or off.", ENABLED, SAFE, set_dark_mode,
         _onoff("enabled", "dark mode")),
    Tool("set_do_not_disturb", "Turn Do Not Disturb (hide notification banners) on or off.", ENABLED, SAFE,
         set_do_not_disturb, _onoff("enabled", "Do Not Disturb")),
    Tool("set_volume", "Set the output volume in percent (0-100).",
         _obj({"percent": {"type": "integer", "minimum": 0, "maximum": 100}}, ["percent"]), SAFE, set_volume,
         lambda a: f"Set volume to {a.get('percent')}%"),
    Tool("set_mute", "Mute or unmute the sound output.", _obj({"muted": {"type": "boolean"}}, ["muted"]), SAFE,
         set_mute, _onoff("muted", "mute")),
    Tool("set_brightness", "Set the screen brightness in percent (1-100).",
         _obj({"percent": {"type": "integer", "minimum": 1, "maximum": 100}}, ["percent"]), SAFE, set_brightness,
         lambda a: f"Set brightness to {a.get('percent')}%"),
    Tool("set_power_mode", "Set the power mode: power-saver, balanced or performance.",
         _obj({"mode": {"type": "string", "enum": list(POWER_MODES)}}, ["mode"]), SAFE, set_power_mode,
         lambda a: f"Set power mode to {POWER_MODES.get(str(a.get('mode')), a.get('mode'))}"),
    Tool("set_night_light", "Turn Night Light (warmer screen colors) on or off.", ENABLED, SAFE, set_night_light,
         _onoff("enabled", "Night Light")),
    Tool("find_settings", "Find GNOME settings by words, e.g. 'text scaling' or 'touchpad tap'. "
         "Returns schema and key names to use with get_setting and change_setting.",
         _obj({"query": {"type": "string"}}, ["query"]), SAFE, find_settings,
         lambda a: f"Look up settings for “{a.get('query', '')}”"),
    Tool("get_setting", "Read the current value of a GNOME setting.",
         _obj({"schema": {"type": "string"}, "key": {"type": "string"}}, ["schema", "key"]), SAFE, get_setting,
         lambda a: f"Read {a.get('key', 'a setting')}"),
    Tool("change_setting", "Change a GNOME desktop setting. value is in GVariant text form, "
         "e.g. true, 1.25, 'prefer-dark', 300.",
         _obj({"schema": {"type": "string"}, "key": {"type": "string"}, "value": {"type": "string"}},
              ["schema", "key", "value"]), CONFIRM, change_setting,
         lambda a: f"Set {a.get('key', 'a setting')} to {a.get('value')}"),
    Tool("open_settings", "Open GNOME Settings at a page, e.g. display, power, keyboard, sound.",
         _obj({"panel": {"type": "string"}}, ["panel"]), SAFE, open_settings,
         lambda a: f"Open Settings: {a.get('panel', '')}"),
    Tool("set_wifi", "Turn Wi-Fi on or off. Turning it off disconnects the network.", ENABLED, CONFIRM, set_wifi,
         _onoff("enabled", "Wi-Fi")),
    Tool("set_bluetooth", "Turn Bluetooth on or off. Turning it off disconnects Bluetooth devices.", ENABLED,
         CONFIRM, set_bluetooth, _onoff("enabled", "Bluetooth")),
]

BY_NAME = {t.name: t for t in TOOLS}
WEB_TOOLS = {"web_search", "read_web_page"}


def schemas(exclude: set[str] | frozenset = frozenset()) -> list[dict]:
    return [t.schema() for t in TOOLS if t.name not in exclude]


def summarize(name: str, args: dict) -> str:
    tool = BY_NAME.get(name)
    return tool.summary(args) if tool else name


def run_tool(ctx: Context, name: str, args: dict) -> str:
    tool = BY_NAME.get(name)
    if tool is None:
        raise ToolError(f"unknown tool '{name}'")
    return tool.run(ctx, args)
