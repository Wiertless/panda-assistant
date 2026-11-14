"""GNOME Shell search provider.

Shows "Ask: <query>", plus saved skills and earlier chats that match. It
never talks to the model itself: GNOME queries providers on every
keystroke, so results come from plain files; the NPU runs only when the
user picks one. The process is D-Bus activated and exits when idle.
"""

from __future__ import annotations

import os
import subprocess
import sys

from gi.repository import Gio, GLib

from . import APP_ID, store

BUS_NAME = APP_ID + ".SearchProvider"
OBJECT_PATH = "/" + APP_ID.replace(".", "/") + "/SearchProvider"
IDLE_EXIT_SECONDS = 60
MIN_CHARS = 3
MAX_SKILLS = 2
MAX_CHATS = 3
# Result ids: a plain query means "Ask"; these prefixes mark the others.
SKILL = "panda-skill:"
CHAT = "panda-chat:"

INTERFACE_XML = """
<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet">
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetSubsearchResultSet">
      <arg type="as" name="previous_results" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetResultMetas">
      <arg type="as" name="identifiers" direction="in"/>
      <arg type="aa{sv}" name="metas" direction="out"/>
    </method>
    <method name="ActivateResult">
      <arg type="s" name="identifier" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="LaunchSearch">
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
  </interface>
</node>
"""


def _matches(text: str, words: list[str]) -> bool:
    text = text.casefold()
    return all(w in text for w in words)


def results_for(terms: list[str]) -> list[str]:
    query = " ".join(terms).strip()
    if len(query) < MIN_CHARS:
        return []
    words = [t.casefold().lstrip("/") for t in terms if t.strip("/")]
    out = [query]
    try:
        out += [SKILL + s.slug for s in store.skills()
                if _matches(f"{s.slug} {s.title} {s.description}", words)][:MAX_SKILLS]
        out += [CHAT + c.id for c in store.list_chats() if _matches(c.title, words)][:MAX_CHATS]
    except OSError:
        pass
    return out


def meta_for(result: str) -> dict:
    name, desc, icon = f"Ask: {result}", "Ask the assistant on your NPU", APP_ID
    if result.startswith(SKILL):
        skill = store.find_skill(result[len(SKILL):])
        if skill:
            name, desc = f"Run skill: {skill.title}", skill.description or f"/{skill.slug}"
            icon = "view-list-bullet-symbolic"
    elif result.startswith(CHAT):
        chat = store.load_chat(result[len(CHAT):])
        if chat:
            name, desc = chat.get("title") or "Chat", "Earlier chat"
            icon = "document-open-recent-symbolic"
    return {
        "id": GLib.Variant("s", result),
        "name": GLib.Variant("s", name),
        "description": GLib.Variant("s", desc),
        "gicon": GLib.Variant("s", icon),
    }


def activate(result: str) -> None:
    """Hand a picked result to the app."""
    if result.startswith(SKILL):
        launch_app("ask", "/" + result[len(SKILL):])
    elif result.startswith(CHAT):
        launch_app("open-chat", result[len(CHAT):])
    else:
        launch_app("ask", result)


def launch_app(action: str, value: str) -> None:
    """Run an action of the (D-Bus activatable) app; start it the classic way if that fails."""
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        bus.call_sync(APP_ID, "/" + APP_ID.replace(".", "/"), "org.freedesktop.Application", "ActivateAction",
                      GLib.Variant("(sava{sv})", (action, [GLib.Variant("s", value)], {})),
                      None, Gio.DBusCallFlags.NONE, 10000, None)
        return
    except GLib.Error:
        pass
    exe = os.environ.get("PANDA_ASSISTANT_BIN", "/usr/bin/panda-assistant")
    flag = "--open-chat" if action == "open-chat" else "--ask"
    subprocess.Popen([exe, flag, value], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)


class SearchProvider:
    def __init__(self):
        self.loop = GLib.MainLoop()
        self.idle_id = 0
        self.node = Gio.DBusNodeInfo.new_for_xml(INTERFACE_XML)

    def _touch(self) -> None:
        if self.idle_id:
            GLib.source_remove(self.idle_id)
        self.idle_id = GLib.timeout_add_seconds(IDLE_EXIT_SECONDS, self._quit)

    def _quit(self) -> bool:
        self.loop.quit()
        return GLib.SOURCE_REMOVE

    def _on_call(self, conn, sender, path, iface, method, params, invocation):
        self._touch()
        args = params.unpack()
        if method == "GetInitialResultSet":
            invocation.return_value(GLib.Variant("(as)", (results_for(args[0]),)))
        elif method == "GetSubsearchResultSet":
            invocation.return_value(GLib.Variant("(as)", (results_for(args[1]),)))
        elif method == "GetResultMetas":
            invocation.return_value(GLib.Variant("(aa{sv})", ([meta_for(i) for i in args[0]],)))
        elif method == "ActivateResult":
            activate(args[0])
            invocation.return_value(None)
        elif method == "LaunchSearch":
            launch_app("ask", " ".join(args[0]))
            invocation.return_value(None)
        else:
            invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)

    def _on_bus(self, conn, name):
        conn.register_object(OBJECT_PATH, self.node.interfaces[0], self._on_call, None, None)

    def run(self) -> int:
        Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.NONE,
                         self._on_bus, None, lambda *a: self.loop.quit())
        self._touch()
        self.loop.run()
        return 0


def main() -> int:
    return SearchProvider().run()


if __name__ == "__main__":
    sys.exit(main())
