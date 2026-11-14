"""The window's link to assistantd, the Rust service that runs the model.

assistantd owns the model, the tools, memory, skills, chats and MCP; the
window only shows them. D-Bus starts the service on the first call. A turn
streams back as signals tagged with its chat id, which the window chooses,
so it can route them before the call returns.
"""

from __future__ import annotations

import json
from typing import Callable, Protocol

from gi.repository import Gio, GLib

BUS_NAME = "io.github.wiertless.PandaAssistant.Core"
OBJECT_PATH = "/io/github/wiertless/PandaAssistant/Core"
INTERFACE = "io.github.wiertless.PandaAssistant.Core1"
BUSY_ERROR = "io.github.wiertless.PandaAssistant.Error.Busy"


class CoreError(Exception):
    pass


class Turn(Protocol):
    """What the window does with a turn's events (runs on the GTK thread)."""

    def on_text(self, text: str) -> None: ...
    def on_reasoning(self, text: str) -> None: ...
    def on_tool_started(self, name: str, summary: str) -> None: ...
    def on_tool_finished(self, name: str, result: str, ok: bool) -> None: ...
    def on_notice(self, text: str) -> None: ...
    def on_handoff(self, model: str) -> None: ...
    def on_compacting(self) -> None: ...
    def on_confirm(self, request: int, name: str, summary: str, detail: str, can_always: bool) -> None: ...
    def on_finished(self, error: str) -> None: ...


class Core:
    def __init__(self, on_state: Callable[[str, str], None] = lambda m, s: None,
                 on_clipboard: Callable[[str], None] = lambda t: None,
                 on_step_aside: Callable[[], None] = lambda: None,
                 on_step_back: Callable[[], None] = lambda: None):
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.turns: dict[str, Turn] = {}
        self.on_state, self.on_clipboard = on_state, on_clipboard
        self.on_step_aside, self.on_step_back = on_step_aside, on_step_back
        self._sub = self.bus.signal_subscribe(BUS_NAME, INTERFACE, None, OBJECT_PATH, None,
                                              Gio.DBusSignalFlags.NONE, self._on_signal)

    # -- calls --------------------------------------------------------------
    def _call(self, method: str, args: GLib.Variant | None = None, reply: str | None = None,
              timeout_ms: int = 25000):
        try:
            result = self.bus.call_sync(BUS_NAME, OBJECT_PATH, INTERFACE, method, args,
                                        GLib.VariantType.new(reply) if reply else None,
                                        Gio.DBusCallFlags.NONE, timeout_ms, None)
        except GLib.Error as e:
            if BUSY_ERROR in (e.message or ""):
                raise CoreError("Still busy with the last question.") from None
            remote = Gio.DBusError.get_remote_error(e) or ""
            if "ServiceUnknown" in remote or "NameHasNoOwner" in remote:
                raise CoreError("The assistant service (assistantd) isn't installed.") from None
            Gio.DBusError.strip_remote_error(e)
            raise CoreError(e.message) from None
        return result.unpack() if result is not None else None

    def _call_async(self, method: str, args: GLib.Variant | None = None, reply: str | None = None,
                    done: Callable[[object | None, str | None], None] = lambda v, err: None, timeout_ms: int = 60000):
        def finish(bus, res):
            try:
                result = bus.call_finish(res)
            except GLib.Error as e:
                Gio.DBusError.strip_remote_error(e)
                done(None, e.message)
                return
            done(result.unpack()[0] if reply and result is not None else None, None)

        self.bus.call(BUS_NAME, OBJECT_PATH, INTERFACE, method, args,
                      GLib.VariantType.new(reply) if reply else None, Gio.DBusCallFlags.NONE, timeout_ms, None, finish)

    def ask(self, chat_id: str, text: str, images: list[str], turn: Turn) -> str:
        self.turns[chat_id] = turn
        try:
            return self._call("Ask", GLib.Variant("(ssas)", (chat_id, text, images)), "(s)")[0]
        except CoreError:
            self.turns.pop(chat_id, None)
            raise

    def confirm(self, request: int, answer: str) -> None:
        self._call_async("Confirm", GLib.Variant("(us)", (request, answer)))

    def stop(self) -> None:
        self._call_async("Stop")

    def load(self) -> None:
        self._call_async("Load")

    def unload(self) -> None:
        self._call_async("Unload")

    def reload(self, wait: bool = False) -> None:
        if wait:
            self._call("Reload")
        else:
            self._call_async("Reload")

    def status(self) -> dict:
        return json.loads(self._call("Status", None, "(s)")[0])

    def models(self, done: Callable[[list[dict] | None, str | None], None]) -> None:
        self._call_async("Models", None, "(s)", lambda v, err: done(json.loads(v) if v else None, err))

    def connections(self) -> dict:
        return json.loads(self._call("Connections", None, "(s)")[0])

    # -- signals --------------------------------------------------------------
    def _on_signal(self, _conn, _sender, _path, _iface, name, params):
        args = params.unpack()
        if name == "StateChanged":
            self.on_state(*args)
            return
        if name == "Clipboard":
            self.on_clipboard(args[0])
            return
        if name == "StepAside":
            self.on_step_aside()
            return
        if name == "StepBack":
            self.on_step_back()
            return
        turn = self.turns.get(args[0]) if args else None
        if turn is None:
            return
        rest = args[1:]
        if name == "Text":
            turn.on_text(*rest)
        elif name == "Reasoning":
            turn.on_reasoning(*rest)
        elif name == "ToolStarted":
            turn.on_tool_started(*rest)
        elif name == "ToolFinished":
            turn.on_tool_finished(*rest)
        elif name == "Notice":
            turn.on_notice(*rest)
        elif name == "Handoff":
            turn.on_handoff(*rest)
        elif name == "Compacting":
            turn.on_compacting()
        elif name == "ConfirmRequest":
            turn.on_confirm(*rest)
        elif name == "Finished":
            self.turns.pop(args[0], None)
            turn.on_finished(*rest)
