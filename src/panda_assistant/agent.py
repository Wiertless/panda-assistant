"""The chat loop: send messages, run the tools the model asks for, repeat."""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from typing import Callable

from . import tools as T
from .flm import FlmClient, FlmError, strip_think

MAX_ROUNDS = 6

SYSTEM_PROMPT = """You are Panda, an assistant running locally on the user's laptop \
(Fedora Silverblue with GNOME) on its NPU. Be brief and direct. When the user asks you \
to do something on the computer, or asks about its state, call a tool instead of \
describing how to do it. Only call tools the request needs. Never invent tool results; \
if a tool fails, say what failed. Answer in the user's language."""


@dataclass
class Callbacks:
    """How the agent talks to the UI. Called from the worker thread."""

    text: Callable[[str], None] = lambda s: None
    reasoning: Callable[[str], None] = lambda s: None
    tool_started: Callable[[str, str], None] = lambda name, summary: None
    tool_finished: Callable[[str, str, bool], None] = lambda name, result, ok: None
    notice: Callable[[str], None] = lambda s: None
    # Blocks until the user answers; True means run it.
    confirm: Callable[[str, str], bool] = lambda name, summary: False


@dataclass
class Agent:
    client: FlmClient
    model: str
    use_tools: bool = True
    ctx: T.Context = field(default_factory=T.Context)
    messages: list[dict] = field(default_factory=list)
    cancel: threading.Event = field(default_factory=threading.Event)

    def reset(self) -> None:
        self.messages = []

    def _conversation(self) -> list[dict]:
        return [{"role": "system", "content": SYSTEM_PROMPT}] + self.messages

    def ask(self, text: str, cb: Callbacks) -> None:
        """Run one user turn to completion. Blocking; call it from a thread."""
        self.cancel.clear()
        self.messages.append({"role": "user", "content": text})
        for _ in range(MAX_ROUNDS):
            if self.cancel.is_set():
                return
            tool_schemas = T.schemas() if self.use_tools else None
            try:
                content, calls = self._stream(tool_schemas, cb)
            except FlmError as e:
                if tool_schemas and e.status in (400, 422):
                    self.use_tools = False
                    cb.notice(f"{self.model} can't use tools, so it can only chat.")
                    content, calls = self._stream(None, cb)
                else:
                    raise
            assistant = {"role": "assistant", "content": content}
            if calls:
                assistant["tool_calls"] = [{"id": c.id, "type": "function",
                                            "function": {"name": c.name, "arguments": c.arguments or "{}"}}
                                           for c in calls]
            self.messages.append(assistant)
            if not calls or self.cancel.is_set():
                return
            for call in calls:
                self.messages.append({"role": "tool", "tool_call_id": call.id,
                                      "content": self._run_call(call, cb)})
        cb.notice("Stopped after too many tool calls in a row.")

    def _stream(self, tool_schemas, cb: Callbacks):
        content = []
        calls = []
        stream = self.client.chat_stream(self.model, self._conversation(), tool_schemas)
        for event in stream:
            if self.cancel.is_set():
                stream.close()
                return strip_think("".join(content)), []
            if event.kind == "text":
                content.append(event.text)
                cb.text(event.text)
            elif event.kind == "reasoning":
                cb.reasoning(event.text)
            elif event.kind == "done":
                calls = event.tool_calls
                if event.finish_reason == "length" and calls:
                    cb.notice("The answer was cut off before the action was complete.")
                    calls = []
        return strip_think("".join(content)), calls

    def _run_call(self, call, cb: Callbacks) -> str:
        try:
            args = call.parsed_arguments()
        except (ValueError, json.JSONDecodeError):
            result = f"Error: arguments for {call.name} were not valid JSON."
            cb.tool_finished(call.name, result, False)
            return result
        tool = T.BY_NAME.get(call.name)
        summary = T.summarize(call.name, args)
        if tool is None:
            result = f"Error: there is no tool named {call.name}."
            cb.tool_finished(call.name, result, False)
            return result
        if tool.risk == T.CONFIRM and not cb.confirm(call.name, summary):
            result = "The user declined this action."
            cb.tool_finished(call.name, "Cancelled", False)
            return result
        cb.tool_started(call.name, summary)
        try:
            result = T.run_tool(self.ctx, call.name, args)
            cb.tool_finished(call.name, result, True)
            return result
        except T.ToolError as e:
            cb.tool_finished(call.name, str(e), False)
            return f"Error: {e}"
        except Exception as e:  # noqa: BLE001 - report anything back to the model and UI
            cb.tool_finished(call.name, f"{type(e).__name__}: {e}", False)
            return f"Error: {e}"
