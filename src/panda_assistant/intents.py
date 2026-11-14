"""What other apps let the assistant do: their published actions and media controls.

Desktop entries list actions such as "New Private Window" or "Compose New
Message" (the same items as the app icon's right-click menu). Media players
publish MPRIS on D-Bus, so any of them can be played, paused or skipped.
"""

from __future__ import annotations

from .tools import Context, ToolError, _find_app, _norm_name, pick

MPRIS_PREFIX = "org.mpris.MediaPlayer2."
MPRIS_PATH = "/org/mpris/MediaPlayer2"
PLAYER = "org.mpris.MediaPlayer2.Player"
MEDIA_ACTIONS = {"play": "Play", "pause": "Pause", "play_pause": "PlayPause", "toggle": "PlayPause",
                 "next": "Next", "previous": "Previous", "stop": "Stop"}


def _gio():
    from gi.repository import Gio
    return Gio


# -- desktop actions -------------------------------------------------------------

def _actions(info) -> dict[str, str]:
    """Action id -> its name as the user sees it."""
    return {a: info.get_action_name(a) or a for a in info.list_actions()}


def _desktop_app(name: str):
    info = _find_app(name)
    Gio = _gio()
    if not isinstance(info, Gio.DesktopAppInfo):
        info = Gio.DesktopAppInfo.new(info.get_id() or "") or info
    return info


def list_app_actions(ctx: Context, args: dict) -> str:
    info = _desktop_app(str(args.get("app", "")))
    actions = _actions(info)
    if not actions:
        return f"{info.get_name()} has no actions."
    return f"{info.get_name()} can: " + ", ".join(actions.values())


def run_app_action(ctx: Context, args: dict) -> str:
    info = _desktop_app(str(args.get("app", "")))
    actions = _actions(info)
    if not actions:
        raise ToolError(f"{info.get_name()} has no actions")
    by_label = {label: action for action, label in actions.items()}
    wanted = str(args.get("action", ""))
    # Match on what the user sees ("New Private Window") or the id ("new-private-window").
    if _norm_name(wanted) in {_norm_name(a) for a in actions}:
        action = next(a for a in actions if _norm_name(a) == _norm_name(wanted))
    else:
        action = by_label[pick(wanted, list(by_label), f"action of {info.get_name()}")]
    info.launch_action(action, None)
    return f"{info.get_name()}: {actions[action]}."


# -- media (MPRIS) -----------------------------------------------------------------

def _bus():
    Gio = _gio()
    return Gio.bus_get_sync(Gio.BusType.SESSION, None)


def _players() -> list[dict]:
    from gi.repository import GLib
    Gio = _gio()
    bus = _bus()
    names = bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames",
                          None, GLib.VariantType.new("(as)"), Gio.DBusCallFlags.NONE, 3000, None).unpack()[0]
    players = []
    for name in names:
        if not name.startswith(MPRIS_PREFIX):
            continue
        try:
            props = bus.call_sync(name, MPRIS_PATH, "org.freedesktop.DBus.Properties", "GetAll",
                                  GLib.Variant("(s)", (PLAYER,)), GLib.VariantType.new("(a{sv})"),
                                  Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
            ident = bus.call_sync(name, MPRIS_PATH, "org.freedesktop.DBus.Properties", "Get",
                                  GLib.Variant("(ss)", ("org.mpris.MediaPlayer2", "Identity")),
                                  GLib.VariantType.new("(v)"), Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
        except GLib.Error:
            continue
        meta = props.get("Metadata") or {}
        artists = meta.get("xesam:artist") or []
        players.append({"bus": name, "name": ident or name[len(MPRIS_PREFIX):],
                        "status": props.get("PlaybackStatus", "Stopped"),
                        "title": meta.get("xesam:title") or "",
                        "artist": ", ".join(artists) if isinstance(artists, list) else str(artists)})
    return players


def _describe(p: dict) -> str:
    what = " – ".join(x for x in (p["artist"], p["title"]) if x) or "nothing"
    return f"{p['name']}: {p['status'].lower()}, {what}"


def choose_player(players: list[dict], name: str = "") -> dict:
    if not players:
        raise ToolError("no media player is running")
    if name.strip():
        return next(p for p in players if p["name"] == pick(name, [p["name"] for p in players], "player"))
    # The one playing, else the one paused most recently (first listed), else any.
    for status in ("Playing", "Paused"):
        for p in players:
            if p["status"] == status:
                return p
    return players[0]


def media_control(ctx: Context, args: dict) -> str:
    action = str(args.get("action", "status")).strip().lower().replace("-", "_").replace(" ", "_")
    players = _players()
    if action in ("status", "now_playing", "what"):
        if not players:
            return "No media player is running."
        return "\n".join(_describe(p) for p in players)
    method = MEDIA_ACTIONS.get(action)
    if method is None:
        raise ToolError("action must be play, pause, play_pause, next, previous, stop or status")
    player = choose_player(players, str(args.get("player", "")))
    from gi.repository import GLib
    try:
        _bus().call_sync(player["bus"], MPRIS_PATH, PLAYER, method, None, None,
                         _gio().DBusCallFlags.NONE, 3000, None)
    except GLib.Error as e:
        raise ToolError(f"{player['name']} refused: {e.message}") from None
    return f"{player['name']}: {method.replace('PlayPause', 'play/pause').lower()}."
