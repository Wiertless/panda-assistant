"""The chat loop: send messages, run the tools the model asks for, repeat."""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from typing import Callable

from . import tools as T
from .context import system_context
from .flm import FlmClient, FlmError, strip_think

MAX_ROUNDS = 6

SYSTEM_PROMPT = """You are Panda, an assistant running locally on the user's laptop \
(Fedora Silverblue with GNOME) on its NPU. Be brief and direct. When the user asks you \
to do something on the computer, or asks about its state, call a tool instead of \
describing how to do it. Only call tools the request needs. Never invent tool results; \
if a tool fails, say what failed. Answer in the user's language."""

WEB_PROMPT = """ For anything recent or that you aren't sure of, use web_search and read the \
most relevant result with read_web_page, then answer briefly and name the source."""

ESCALATE = "ask_smarter_model"
ESCALATE_PROMPT = """ If a request needs more than you can do well (careful reasoning, \
longer writing, code, several steps, or you're unsure), call ask_smarter_model instead \
of guessing. It hands the task to a bigger local model, which answers the user directly."""


def escalate_schema(model: str) -> dict:
    return {"type": "function", "function": {
        "name": ESCALATE,
        "description": f"Hand this request to {model}, a bigger and smarter local model that can also "
                       "use the tools. Use it for hard requests; it answers the user itself.",
        "parameters": {"type": "object", "properties": {
            "task": {"type": "string", "description": "What the user wants, and anything you found out"}},
            "required": ["task"]}}}


@dataclass
class Callbacks:
    """How the agent talks to the UI. Called from the worker thread."""

    text: Callable[[str], None] = lambda s: None
    reasoning: Callable[[str], None] = lambda s: None
    tool_started: Callable[[str, str], None] = lambda name, summary: None
    tool_finished: Callable[[str, str, bool], None] = lambda name, result, ok: None
    notice: Callable[[str], None] = lambda s: None
    handoff: Callable[[str], None] = lambda model: None
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
    smarter_model: str | None = None
    web_enabled: bool = True
    machine: str | None = None  # system_context(), taken once per chat

    def reset(self) -> None:
        self.messages = []
        self.machine = None

    def _can_escalate(self) -> bool:
        return bool(self.use_tools and self.smarter_model and self.smarter_model != self.model)

    def _conversation(self) -> list[dict]:
        if self.machine is None:
            self.machine = system_context()
        system = SYSTEM_PROMPT
        if self.use_tools and self.web_enabled:
            system += WEB_PROMPT
        if self._can_escalate():
            system += ESCALATE_PROMPT
        # The machine description goes last and stays the same for the whole
        # chat, so the server can reuse the cached prompt between turns.
        return [{"role": "system", "content": system + "\n\n" + self.machine}] + self.messages

    def _tool_schemas(self):
        if not self.use_tools:
            return None
        schemas = T.schemas(exclude=set() if self.web_enabled else T.WEB_TOOLS)
        if self._can_escalate():
            schemas.append(escalate_schema(self.smarter_model))
        return schemas

    def ask(self, text: str, cb: Callbacks) -> None:
        """Run one user turn to completion. Blocking; call it from a thread."""
        self.cancel.clear()
        self.messages.append({"role": "user", "content": text})
        for _ in range(MAX_ROUNDS):
            if self.cancel.is_set():
                return
            tool_schemas = self._tool_schemas()
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
            escalation = next((c for c in calls if c.name == ESCALATE), None) if self._can_escalate() else None
            if escalation:
                self._escalate(escalation, text, cb)
                return
            for call in calls:
                self.messages.append({"role": "tool", "tool_call_id": call.id,
                                      "content": self._run_call(call, cb)})
        cb.notice("Stopped after too many tool calls in a row.")

    def _escalate(self, call, request: str, cb: Callbacks) -> None:
        """Let the smarter model finish this turn, then end it there.

        FLM keeps one model loaded, so going back to the small model for a
        closing sentence would mean loading it again; the big model's answer
        is the reply. The exchange is recorded as a tool result plus that
        answer, so the next turn's history stays valid.
        """
        try:
            task = str(call.parsed_arguments().get("task", "")).strip()
        except (ValueError, json.JSONDecodeError):
            task = ""
        cb.handoff(self.smarter_model)
        helper = Agent(self.client, self.smarter_model, use_tools=True, ctx=self.ctx, cancel=self.cancel,
                       web_enabled=self.web_enabled, machine=self.machine)
        # Earlier turns as plain text; tool traffic belongs to the other model.
        helper.messages = [{"role": m["role"], "content": m["content"]} for m in self.messages[:-2]
                           if m["role"] in ("user", "assistant") and m.get("content")]
        note = f"\n\n(A smaller assistant handed this to you. Its notes: {task})" if task else ""
        helper.ask(request + note, cb)
        answer = next((m["content"] for m in reversed(helper.messages)
                       if m["role"] == "assistant" and m.get("content")), "")
        self.messages.append({"role": "tool", "tool_call_id": call.id,
                              "content": f"{self.smarter_model} answered the user directly:\n{answer}"})
        self.messages.append({"role": "assistant", "content": answer})

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
