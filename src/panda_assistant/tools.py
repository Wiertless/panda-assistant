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

def _find_app(name: str):
    Gio = _gio()
    wanted = name.strip().lower()
    if not wanted:
        raise ToolError("no app name given")
    best = None
    for app in Gio.AppInfo.get_all():
        if not app.should_show():
            continue
        names = [(app.get_display_name() or "").lower(), (app.get_id() or "").lower().removesuffix(".desktop")]
        if wanted in names:
            return app
        if best is None and any(wanted in n for n in names):
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
    Tool("open_app", "Open an installed application by its name, e.g. 'Files' or 'Zed'.",
         _obj({"name": {"type": "string", "description": "App name"}}, ["name"]), SAFE, open_app,
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
    Tool("set_wifi", "Turn Wi-Fi on or off. Turning it off disconnects the network.", ENABLED, CONFIRM, set_wifi,
         _onoff("enabled", "Wi-Fi")),
    Tool("set_bluetooth", "Turn Bluetooth on or off. Turning it off disconnects Bluetooth devices.", ENABLED,
         CONFIRM, set_bluetooth, _onoff("enabled", "Bluetooth")),
]

BY_NAME = {t.name: t for t in TOOLS}


def schemas() -> list[dict]:
    return [t.schema() for t in TOOLS]


def summarize(name: str, args: dict) -> str:
    tool = BY_NAME.get(name)
    return tool.summary(args) if tool else name


def run_tool(ctx: Context, name: str, args: dict) -> str:
    tool = BY_NAME.get(name)
    if tool is None:
        raise ToolError(f"unknown tool '{name}'")
    return tool.run(ctx, args)
