"""The chat loop: send messages, run the tools the model asks for, repeat."""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from typing import Callable

from . import store
from . import tools as T
from .context import system_context
from .flm import FlmClient, FlmError, strip_think

MAX_ROUNDS = 6

SYSTEM_PROMPT = """You are the user's assistant, running locally on the user's laptop \
(Fedora Silverblue with GNOME) on its NPU. Be brief and direct. When the user asks you \
to do something on the computer, or asks about its state, call a tool instead of \
describing how to do it. Only call tools the request needs. Never invent tool results; \
if a tool fails, say what failed. Answer in the user's language."""

WEB_PROMPT = """ For anything recent or that you aren't sure of, use web_search and read the \
most relevant result with read_web_page, then answer briefly and name the source."""

MEMORY_PROMPT = """ When the user tells you a lasting preference or fact about themselves, or asks you \
to remember something, call remember. Use what you remember without mentioning it."""

SKILL_PROMPT = """ The user's saved skills are listed below; when a request matches one, call use_skill \
and follow its steps. When the user asks to save a workflow, call save_skill."""

VISION_PROMPT = """ You can see images. Use look_at_screen when the user asks about what's on their \
screen, and look_at_image for image files."""

IMAGE_MARKER = "[image]"

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


def with_images(text: str, images: list[str] | None):
    """Message content: plain text, or text plus pictures (OpenAI format)."""
    if not images:
        return text
    return [{"type": "text", "text": text}] + [{"type": "image_url", "image_url": {"url": url}} for url in images]


@dataclass
class Callbacks:
    """How the agent talks to the UI. Called from the worker thread."""

    text: Callable[[str], None] = lambda s: None
    reasoning: Callable[[str], None] = lambda s: None
    tool_started: Callable[[str, str], None] = lambda name, summary: None
    tool_finished: Callable[[str, str, bool], None] = lambda name, result, ok: None
    notice: Callable[[str], None] = lambda s: None
    handoff: Callable[[str], None] = lambda model: None
    # Blocks until the user answers; True means run it. detail is the exact
    # command or value, shown to the user (may be empty).
    confirm: Callable[[str, str, str], bool] = lambda name, summary, detail: False


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
    memory_enabled: bool = True
    # Which models can look at images, and one to hand images to otherwise.
    sees: Callable[[str], bool] = lambda model: False
    vision_model: str | None = None
    notes: str | None = None  # memory and skills, taken once per chat
    # Tools from other apps (MCP servers), looked up at each request.
    extra_tools: Callable[[], list] = lambda: []
    # Tools the user chose "Always Allow" for (shared with the app).
    always_allow: set[str] = field(default_factory=set)

    def reset(self) -> None:
        self.messages = []
        self.machine = None
        self.notes = None

    def _sees(self) -> bool:
        return bool(self.sees(self.model))

    def _notes(self) -> str:
        """Remembered facts and saved skills, for the end of the system prompt."""
        parts = []
        if self.memory_enabled:
            facts = store.facts()
            if facts:
                parts.append("What you remember about the user:\n" + "\n".join(f"- {f}" for f in facts))
        if self.use_tools:
            skills = store.skills()
            if skills:
                parts.append("Saved skills:\n" + "\n".join(
                    f"- {s.title}" + (f": {s.description}" if s.description else "") for s in skills[:20]))
        return "\n\n".join(parts)

    def _can_escalate(self) -> bool:
        return bool(self.use_tools and self.smarter_model and self.smarter_model != self.model)

    def _conversation(self) -> list[dict]:
        if self.machine is None:
            self.machine = system_context()
        if self.notes is None:
            self.notes = self._notes()
        system = SYSTEM_PROMPT
        if self.use_tools and self.web_enabled:
            system += WEB_PROMPT
        if self.use_tools and self.memory_enabled:
            system += MEMORY_PROMPT
        if self.use_tools:
            system += SKILL_PROMPT
        if self.use_tools and self._sees():
            system += VISION_PROMPT
        if self._can_escalate():
            system += ESCALATE_PROMPT
        # The machine description and notes go last and stay the same for the
        # whole chat, so the server can reuse the cached prompt between turns.
        tail = "\n\n".join(p for p in (self.machine, self.notes) if p)
        return [{"role": "system", "content": system + "\n\n" + tail}] + self.messages

    def _tool_schemas(self):
        if not self.use_tools:
            return None
        exclude = set()
        if not self.web_enabled:
            exclude |= T.WEB_TOOLS
        if not self.memory_enabled:
            exclude |= T.MEMORY_TOOLS
        if not self._sees():
            exclude |= T.VISION_TOOLS
        schemas = T.schemas(exclude=exclude) + [t.schema() for t in self.extra_tools()]
        if self._can_escalate():
            schemas.append(escalate_schema(self.smarter_model))
        return schemas

    def ask(self, text: str, cb: Callbacks, images: list[str] | None = None) -> None:
        """Run one user turn to completion. Blocking; call it from a thread.

        images are data: URLs attached to the message.
        """
        self.cancel.clear()
        self._drop_old_images()
        if images and not self._sees():
            if self.vision_model and self.vision_model != self.model:
                self._hand_images(text, images, cb)
                return
            cb.notice(f"{self.model} can't see images, so they were left out.")
            images = None
        self.messages.append({"role": "user", "content": with_images(text, images)})
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
            if self.ctx.images:
                # Tool results are text only, so pictures a tool took (a
                # screenshot) follow as a message of their own.
                shots, self.ctx.images[:] = list(self.ctx.images), []
                if self._sees():
                    self.messages.append({"role": "user", "content": with_images("(Image from the tool.)", shots)})
        cb.notice("Stopped after too many tool calls in a row.")

    def _drop_old_images(self) -> None:
        """Earlier pictures cost hundreds of tokens each; keep a marker only."""
        for m in self.messages:
            if isinstance(m.get("content"), list):
                m["content"] = store.plain_text(m["content"])

    def _hand_images(self, text: str, images: list[str], cb: Callbacks) -> None:
        """This model is blind: let the vision model answer this turn."""
        cb.handoff(self.vision_model)
        helper = Agent(self.client, self.vision_model, use_tools=self.use_tools, ctx=self.ctx, cancel=self.cancel,
                       web_enabled=self.web_enabled, machine=self.machine, memory_enabled=self.memory_enabled,
                       sees=lambda m: True, notes=self.notes, extra_tools=self.extra_tools,
                       always_allow=self.always_allow)
        helper.messages = [{"role": m["role"], "content": m["content"]} for m in self.messages
                           if m["role"] in ("user", "assistant") and m.get("content")]
        helper.ask(text, cb, images)
        answer = next((m["content"] for m in reversed(helper.messages)
                       if m["role"] == "assistant" and m.get("content")), "")
        self.messages.append({"role": "user", "content": f"{text}\n{IMAGE_MARKER * len(images)}".strip()})
        self.messages.append({"role": "assistant", "content": answer})

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
                       web_enabled=self.web_enabled, machine=self.machine, memory_enabled=self.memory_enabled,
                       sees=self.sees, notes=self.notes, extra_tools=self.extra_tools,
                       always_allow=self.always_allow)
        # Earlier turns as plain text; tool traffic belongs to the other model.
        helper.messages = [{"role": m["role"], "content": store.plain_text(m["content"])} for m in self.messages[:-2]
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
        tool = T.BY_NAME.get(call.name) or next((t for t in self.extra_tools() if t.name == call.name), None)
        if tool is None:
            result = f"Error: there is no tool named {call.name}."
            cb.tool_finished(call.name, result, False)
            return result
        summary = tool.summary(args)
        detail = tool.detail(args) if tool.detail else T.detail(call.name, args)
        needs_ok = tool.risk == T.CONFIRM and (call.name in T.ALWAYS_ASK or call.name not in self.always_allow)
        if needs_ok and not cb.confirm(call.name, summary, detail):
            result = "The user declined this action."
            cb.tool_finished(call.name, "Cancelled", False)
            return result
        cb.tool_started(call.name, summary)
        try:
            result = tool.run(self.ctx, args)
            cb.tool_finished(call.name, result, True)
            return result
        except T.ToolError as e:
            cb.tool_finished(call.name, str(e), False)
            return f"Error: {e}"
        except Exception as e:  # noqa: BLE001 - report anything back to the model and UI
            cb.tool_finished(call.name, f"{type(e).__name__}: {e}", False)
            return f"Error: {e}"
