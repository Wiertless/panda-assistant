"""GTK4 / libadwaita chat window, laid out after the GNOME HIG."""

from __future__ import annotations

import json
import os
import re
import threading
import time
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, GObject, Gtk, Pango  # noqa: E402

from . import APP_ID, APP_NAME, DEFAULT_MODEL, VERSION  # noqa: E402
from . import mcp, store  # noqa: E402
from . import tools as T  # noqa: E402
from .core_client import Core, CoreError  # noqa: E402
from .flm import (DEFAULT_CTX_LEN, Model, pick_smarter_model,  # noqa: E402
                  pick_vision_model)
from .markup import to_pango  # noqa: E402

CONFIG = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "panda-assistant" / "config.json"
REPO_URL = "https://github.com/Wiertless/panda-assistant"
CTX_CHOICES = [(4096, "4K tokens"), (8192, "8K tokens"), (16384, "16K tokens"), (32768, "32K tokens")]
# A bigger model (hand-off, pictures) is unloaded this soon after it's done.
BIG_MODEL_IDLE_SECONDS = 60
IDLE_CHOICES = [(1, "After 1 Minute"), (5, "After 5 Minutes"), (15, "After 15 Minutes"),
                (30, "After 30 Minutes"), (0, "Never")]

CSS = """
.greeting { font-size: 24pt; font-weight: 800; }
.suggestion { border-radius: 999px; padding: 6px 16px; font-weight: normal; }
.composer {
  background: @card_bg_color;
  border-radius: 26px;
  padding: 5px 5px 5px 18px;
  box-shadow: 0 0 0 1px alpha(currentColor, 0.07), 0 2px 6px alpha(black, 0.08), 0 8px 24px alpha(black, 0.06);
}
.composer entry, .composer entry:focus-within { background: none; box-shadow: none; outline: none; min-height: 36px; }
.bubble-user {
  background: @accent_bg_color;
  color: @accent_fg_color;
  border-radius: 20px 20px 6px 20px;
  padding: 9px 15px;
}
.bubble-user selection { background: alpha(white, 0.35); color: @accent_fg_color; }
.assistant-name { font-weight: 700; }
.assistant-text { padding: 2px 0; }
.pill { border-radius: 999px; padding: 4px 12px 4px 9px; font-size: 0.92em; background: alpha(@accent_color, 0.10); }
.pill.failed { background: alpha(@warning_color, 0.16); }
.pill.quiet { background: alpha(currentColor, 0.06); }
.pill image { color: @accent_color; }
.pill.failed image { color: @warning_color; }
.thought { font-size: 0.9em; }
.confirm-card { padding: 6px; }
.confirm-detail { padding: 8px 12px; border-radius: 8px; background: alpha(currentColor, 0.06); }
.attachment { border-radius: 10px; }
.chat-row-time { font-size: 0.85em; }
"""

SUGGESTIONS = ["How much memory is free?", "Turn on Night Light", "What's on my screen?",
               "Fedora 45 news", "Open Downloads", "Bigger text"]

TOOL_ICONS = {
    "web_search": "system-search-symbolic", "read_web_page": "web-browser-symbolic",
    "open_url": "web-browser-symbolic", "open_app": "application-x-executable-symbolic",
    "open_file": "folder-open-symbolic", "get_battery": "battery-good-symbolic",
    "get_system_info": "computer-symbolic", "get_datetime": "preferences-system-time-symbolic",
    "set_dark_mode": "weather-clear-night-symbolic", "set_night_light": "night-light-symbolic",
    "set_power_mode": "power-profile-balanced-symbolic", "set_volume": "audio-volume-high-symbolic",
    "set_mute": "audio-volume-muted-symbolic", "set_brightness": "display-brightness-symbolic",
    "set_wifi": "network-wireless-symbolic", "set_bluetooth": "bluetooth-active-symbolic",
    "set_do_not_disturb": "notifications-disabled-symbolic", "copy_to_clipboard": "edit-copy-symbolic",
    "list_networks": "network-wireless-symbolic", "connect_network": "network-wireless-symbolic",
    "disconnect_network": "network-offline-symbolic", "list_bluetooth": "bluetooth-active-symbolic",
    "connect_bluetooth": "bluetooth-active-symbolic", "disconnect_bluetooth": "bluetooth-disabled-symbolic",
    "remember": "starred-symbolic", "forget": "non-starred-symbolic", "use_skill": "view-list-bullet-symbolic",
    "save_skill": "document-save-symbolic", "look_at_screen": "camera-photo-symbolic",
    "look_at_image": "image-x-generic-symbolic", "run_command": "utilities-terminal-symbolic",
    "list_folder": "folder-symbolic", "find_files": "system-search-symbolic", "read_file": "text-x-generic-symbolic",
    "send_file": "send-to-symbolic", "list_devices": "network-workgroup-symbolic",
    "read_window": "view-reveal-symbolic", "search_file_contents": "system-search-symbolic",
    "read_document": "x-office-document-symbolic", "get_events": "x-office-calendar-symbolic",
    "find_contact": "avatar-default-symbolic", "list_app_actions": "view-more-symbolic",
    "run_app_action": "application-x-executable-symbolic", "media_control": "media-playback-start-symbolic",
}
IMAGE_TYPES = ("image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/tiff")
MAX_ATTACHMENTS = 4
# A question from GNOME search starts a new chat when the last one is older.
STALE_MINUTES = 30


def plain(markdown: str, limit: int = 400) -> str:
    """Notification text: markdown marks removed, cut at a word."""
    text = re.sub(r"```\w*\n?", "", markdown)
    text = re.sub(r"(\*\*|__|`|^#+\s*)", "", text, flags=re.M)
    text = re.sub(r"\n{2,}", "\n", text).strip()
    if len(text) > limit:
        text = text[:limit].rsplit(" ", 1)[0] + "…"
    return text


def relative_time(stamp: float) -> str:
    from datetime import datetime
    then, now = datetime.fromtimestamp(stamp), datetime.now()
    days = (now.date() - then.date()).days
    if days == 0:
        return then.strftime("%H:%M")
    if days == 1:
        return "Yesterday"
    if days < 7:
        return then.strftime("%A")
    return then.strftime("%-d %b" if then.year == now.year else "%-d %b %Y")


def greeting() -> str:
    from datetime import datetime
    hour = datetime.now().hour
    return ("Good morning" if 5 <= hour < 12 else "Good afternoon" if hour < 18
            else "Good evening" if hour < 23 else "Hello, night owl")


def _shown_user_text(content: str) -> str:
    """How a saved user message reads in the chat: skill runs as /name."""
    if content.startswith("Follow my saved skill “"):
        title = content[len("Follow my saved skill “"):].split("”", 1)[0]
        details = content.rpartition("\n\nDetails: ")[2] if "\n\nDetails: " in content else ""
        return f"/{store.slugify(title)}" + (f" {details}" if details else "")
    return content


IMAGE_MARKER = "[image]"
# After answering a question from search with no window, the window process
# quits this soon; the model stays loaded in the service.
HIDDEN_QUIT_SECONDS = 30


class ChatSettings:
    """What the window shows about the chat; the agent itself runs in assistantd."""

    def __init__(self, cfg: dict):
        self.model = cfg.get("model", DEFAULT_MODEL)
        self.web_enabled = bool(cfg.get("web_search", True))
        self.memory_enabled = bool(cfg.get("memory", True))
        self.always_allow = set(cfg.get("always_allow", []))
        self.smarter_model: str | None = None
        self.vision_model: str | None = None
        self.use_tools = True
        self.messages: list[dict] = []

    def reset(self):
        self.messages = []


def load_config() -> dict:
    try:
        return json.loads(CONFIG.read_text())
    except (OSError, ValueError):
        return {}


def save_config(cfg: dict) -> None:
    # The service adds to "always_allow" (Always Allow); keep what it wrote.
    on_disk = load_config()
    if "always_allow" in on_disk and "always_allow" not in cfg.get("_changed", ()):
        cfg["always_allow"] = on_disk["always_allow"]
    cfg.pop("_changed", None)
    try:
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        CONFIG.write_text(json.dumps(cfg, indent=2))
    except OSError:
        pass


def on_main(fn, *args):
    """Run fn on the GTK thread and wait for it."""
    if threading.current_thread() is threading.main_thread():
        return fn(*args)
    done = threading.Event()
    box = {}

    def call():
        box["v"] = fn(*args)
        done.set()
        return GLib.SOURCE_REMOVE

    GLib.idle_add(call)
    done.wait()
    return box.get("v")


def spinner() -> Gtk.Widget:
    if hasattr(Adw, "Spinner"):  # libadwaita 1.6+
        return Adw.Spinner(halign=Gtk.Align.START, width_request=24, height_request=24)
    return Gtk.Spinner(spinning=True, halign=Gtk.Align.START)


class Window(Adw.ApplicationWindow):
    def __init__(self, app: "App"):
        self.cfg = load_config()
        super().__init__(application=app, title=APP_NAME,
                         default_width=self.cfg.get("width", 600), default_height=self.cfg.get("height", 760),
                         width_request=360, height_request=294)
        if self.cfg.get("maximized"):
            self.maximize()
        self.app = app
        self.models: list[Model] = []
        self.busy = False
        self.agent = ChatSettings(self.cfg)
        self.core = Core(on_state=self._on_core_state, on_clipboard=self.copy_text,
                         on_step_aside=self._step_aside, on_step_back=self._step_back)
        self.reply: "Reply | None" = None
        self.chat_id = store.new_chat_id()
        self.last_activity = 0.0  # when the last answer finished
        self.attachments: list[tuple[Gdk.Texture | None, str]] = []  # (thumbnail, data: URL)
        self.smarter_choice = self.cfg.get("smarter_model", "auto")  # "auto", "off" or a model
        self._apply_smarter()  # "auto" is resolved again once the model list has loaded
        self.ctx_len = int(self.cfg.get("ctx_len", DEFAULT_CTX_LEN))
        # 0 = keep the default (small) model loaded for quick answers.
        self.unload_after = int(self.cfg.get("unload_after_minutes", 0))
        self.state = "Not loaded"
        self._loading = False
        self._idle_id = 0

        # Header bar: new chat at the start, primary menu at the end, model as subtitle.
        self.title = Adw.WindowTitle(title=APP_NAME, subtitle=self.agent.model)
        header = Adw.HeaderBar(title_widget=self.title)
        # Chats beside the conversation; on narrow windows they're two pages, and
        # libadwaita's back swipe (touchpad or touchscreen) returns to the list.
        self.split = Adw.NavigationSplitView(min_sidebar_width=240, max_sidebar_width=320, show_content=True)
        new_btn = Gtk.Button(icon_name="list-add-symbolic", tooltip_text="New Chat", action_name="win.new-chat")
        header.pack_start(new_btn)
        self.model_menu = Gio.Menu()
        menu = Gio.Menu()
        menu.append_submenu("_Model", self.model_menu)
        memory = Gio.Menu()
        memory.append("_Unload Model", "win.unload")
        menu.append_section(None, memory)
        tail = Gio.Menu()
        tail.append("_Preferences", "win.preferences")
        tail.append("_Keyboard Shortcuts", "win.show-help-overlay")
        tail.append(f"_About {APP_NAME}", "app.about")
        menu.append_section(None, tail)
        header.pack_end(Gtk.MenuButton(icon_name="open-menu-symbolic", menu_model=menu,
                                       tooltip_text="Main Menu", primary=True))

        # Conversation.
        self.messages = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18,
                                margin_top=18, margin_bottom=18, margin_start=12, margin_end=12)
        clamp = Adw.Clamp(maximum_size=720, tightening_threshold=560, child=self.messages)
        self.scroller = Gtk.ScrolledWindow(child=clamp, vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER)
        self.empty = self._build_start_page()
        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
        self.stack.add_named(self.empty, "empty")
        self.stack.add_named(self.scroller, "chat")

        # Composer: a rounded floating bar, with attached pictures above it.
        self.entry = Gtk.Entry(hexpand=True, placeholder_text="Message, or type / for skills…")
        # Pasting a picture attaches it (the signal lives on the inner GtkText).
        self.entry.get_delegate().connect("paste-clipboard", self._on_paste)
        self.entry.connect("activate", lambda *_: self.send())
        # Start loading the model while the question is being typed.
        self.entry.connect("changed", lambda e: e.get_text().strip() and self.preload())
        self.send_btn = Gtk.Button(icon_name="go-up-symbolic", tooltip_text="Send", valign=Gtk.Align.CENTER)
        self.send_btn.add_css_class("suggested-action")
        self.send_btn.add_css_class("circular")
        self.send_btn.connect("clicked", lambda *_: self.send_or_stop())
        attach_btn = Gtk.Button(icon_name="mail-attachment-symbolic", tooltip_text="Attach Image",
                                valign=Gtk.Align.CENTER, action_name="win.attach")
        attach_btn.add_css_class("flat")
        attach_btn.add_css_class("circular")
        row = Gtk.Box(spacing=6)
        row.append(self.entry)
        row.append(attach_btn)
        row.append(self.send_btn)
        self.attach_box = Gtk.Box(spacing=8, margin_top=6, margin_bottom=2)
        self.attach_revealer = Gtk.Revealer(child=self.attach_box, reveal_child=False)
        composer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        composer.add_css_class("composer")
        composer.append(self.attach_revealer)
        composer.append(row)
        composer_clamp = Adw.Clamp(maximum_size=720, tightening_threshold=560, child=composer,
                                   margin_top=6, margin_bottom=16, margin_start=12, margin_end=12)

        self.banner = Adw.Banner(revealed=False)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.append(self.banner)
        content.append(self.stack)
        view = Adw.ToolbarView(content=content)
        view.add_top_bar(header)
        view.add_bottom_bar(composer_clamp)
        view.set_bottom_bar_style(Adw.ToolbarStyle.FLAT)
        self.split.set_content(Adw.NavigationPage(title=APP_NAME, tag="chat", child=view))
        self.split.set_sidebar(Adw.NavigationPage(title="Chats", tag="chats", child=self._build_sidebar()))
        self.toasts = Adw.ToastOverlay(child=self.split)
        self.set_content(self.toasts)
        narrow = Adw.Breakpoint.new(Adw.BreakpointCondition.parse("max-width: 680sp"))
        narrow.add_setter(self.split, "collapsed", GObject.Value(bool, True))
        self.add_breakpoint(narrow)
        self._setup_zoom()
        drop = Gtk.DropTarget.new(Gdk.FileList, Gdk.DragAction.COPY)
        drop.connect("drop", self._on_drop)
        self.add_controller(drop)

        self._add_action("new-chat", self.new_chat)
        self._add_action("focus-entry", lambda: self.entry.grab_focus())
        self._add_action("stop", self.stop)
        self._add_action("show-help-overlay", self.show_shortcuts)
        self._add_action("preferences", self.show_preferences)
        self._add_action("attach", self.choose_image)
        self._add_action("zoom-in", lambda: self.set_text_scale(self.text_scale + 0.1))
        self._add_action("zoom-out", lambda: self.set_text_scale(self.text_scale - 0.1))
        self._add_action("zoom-reset", lambda: self.set_text_scale(1.0))
        self._add_action("search-chats", self._focus_search)
        self.unload_action = self._add_action("unload", lambda: self.unload(manual=True))
        self.unload_action.set_enabled(False)
        self.model_action = Gio.SimpleAction.new_stateful("model", GLib.VariantType.new("s"),
                                                          GLib.Variant("s", self.agent.model))
        self.model_action.connect("activate", self._on_model)
        self.add_action(self.model_action)

        self.connect("close-request", self._on_close)
        self.entry.grab_focus()
        GLib.idle_add(lambda: self._load_models() and GLib.SOURCE_REMOVE)

    def _build_start_page(self) -> Gtk.Widget:
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, valign=Gtk.Align.CENTER,
                      margin_start=24, margin_end=24, margin_top=24, margin_bottom=24)
        box.append(Gtk.Image(icon_name=APP_ID, pixel_size=96, margin_bottom=12))
        self.greeting = Gtk.Label(label=greeting(), wrap=True, justify=Gtk.Justification.CENTER)
        self.greeting.add_css_class("greeting")
        box.append(self.greeting)
        sub = Gtk.Label(label="Ask anything. It runs privately on your NPU and can search the web, "
                              "see your screen, change settings and work with your apps and files.",
                        wrap=True, justify=Gtk.Justification.CENTER, max_width_chars=46)
        sub.add_css_class("dim-label")
        box.append(sub)
        if hasattr(Adw, "WrapBox"):  # libadwaita 1.7+: chips flow like words
            chips = Adw.WrapBox(child_spacing=8, line_spacing=8, margin_top=22, halign=Gtk.Align.CENTER,
                                justify=Adw.JustifyMode.NONE)
            add = chips.append
        else:
            chips = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, min_children_per_line=2,
                                max_children_per_line=3, column_spacing=8, row_spacing=8, margin_top=22)
            add = chips.append
        prompts = {"Fedora 45 news": "Search the web for Fedora 45 news", "Open Downloads": "Open my Downloads folder",
                   "Bigger text": "Make the text a bit bigger"}
        for text in SUGGESTIONS:
            chip = Gtk.Button(label=text)
            chip.add_css_class("suggestion")
            chip.connect("clicked", lambda _b, t=prompts.get(text, text): self.ask(t))
            add(chip)
        box.append(chips)
        clamp = Adw.Clamp(maximum_size=620, child=box, valign=Gtk.Align.CENTER)
        return Gtk.ScrolledWindow(child=clamp, hscrollbar_policy=Gtk.PolicyType.NEVER, vexpand=True)

    def append_animated(self, widget: Gtk.Widget):
        """Add a row to the conversation with a short slide-in."""
        rev = Gtk.Revealer(child=widget, transition_type=Gtk.RevealerTransitionType.SLIDE_UP,
                           transition_duration=220)
        self.messages.append(rev)
        GLib.idle_add(lambda: (rev.set_reveal_child(True), GLib.SOURCE_REMOVE)[1])
        self.scroll_down()

    def _add_action(self, name, fn):
        action = Gio.SimpleAction.new(name, None)
        action.connect("activate", lambda *_: fn())
        self.add_action(action)
        return action

    def toast(self, text: str):
        self.toasts.add_toast(Adw.Toast(title=GLib.markup_escape_text(text), timeout=3))
        return GLib.SOURCE_REMOVE

    # -- text size: pinch, Ctrl+scroll, Ctrl+plus/minus -------------------
    def _setup_zoom(self):
        self.text_scale = float(self.cfg.get("text_scale", 1.0))
        self._zoom_css = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), self._zoom_css,
                                                  Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        self.messages.add_css_class("conversation")
        pinch = Gtk.GestureZoom()
        start = {"scale": self.text_scale}
        pinch.connect("begin", lambda *_: start.update(scale=self.text_scale))
        pinch.connect("scale-changed", lambda _g, s: self.set_text_scale(start["scale"] * s, save=False))
        pinch.connect("end", lambda *_: self.set_text_scale(self.text_scale))
        self.scroller.add_controller(pinch)
        scroll = Gtk.EventControllerScroll(flags=Gtk.EventControllerScrollFlags.VERTICAL)
        scroll.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)

        def on_scroll(ctrl, _dx, dy):
            if not ctrl.get_current_event_state() & Gdk.ModifierType.CONTROL_MASK:
                return False
            self.set_text_scale(self.text_scale - dy * 0.05)
            return True

        scroll.connect("scroll", on_scroll)
        self.scroller.add_controller(scroll)
        self.set_text_scale(self.text_scale, save=False)

    def set_text_scale(self, scale: float, save: bool = True):
        self.text_scale = round(min(2.0, max(0.7, scale)), 2)
        css = f".conversation {{ font-size: {self.text_scale * 100:.0f}%; }}"
        if hasattr(self._zoom_css, "load_from_string"):
            self._zoom_css.load_from_string(css)
        else:
            self._zoom_css.load_from_data(css.encode())
        if save:
            self.cfg["text_scale"] = self.text_scale
            save_config(self.cfg)

    # -- chat history sidebar ---------------------------------------------
    def _build_sidebar(self) -> Gtk.Widget:
        header = Adw.HeaderBar()
        self.search = Gtk.SearchEntry(placeholder_text="Search Chats", margin_start=12, margin_end=12,
                                      margin_bottom=6)
        self.search.connect("search-changed", lambda *_: self._refresh_chats())
        self.search.connect("stop-search", lambda *_: self.search.set_text(""))
        self.chat_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.chat_list.add_css_class("navigation-sidebar")
        self.chat_list.connect("row-activated", lambda _l, row: self.open_chat(row.chat_id))
        self.no_chats = Adw.StatusPage(icon_name="chat-symbolic" if Gtk.IconTheme.get_for_display(
            Gdk.Display.get_default()).has_icon("chat-symbolic") else "mail-unread-symbolic",
            title="No Chats", description="Your conversations are saved here")
        self.no_chats.add_css_class("compact")
        self.chat_stack = Gtk.Stack()
        self.chat_stack.add_named(Gtk.ScrolledWindow(child=self.chat_list, vexpand=True,
                                                     hscrollbar_policy=Gtk.PolicyType.NEVER), "list")
        self.chat_stack.add_named(self.no_chats, "empty")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.append(self.search)
        box.append(self.chat_stack)
        view = Adw.ToolbarView(content=box)
        view.add_top_bar(header)
        self._refresh_chats()
        return view

    def _refresh_chats(self):
        query = self.search.get_text().strip()
        self.chat_list.remove_all()
        chats = store.list_chats(query)
        for chat in chats:
            row = Gtk.ListBoxRow()
            row.chat_id = chat.id
            box = Gtk.Box(spacing=6)
            texts = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, hexpand=True, valign=Gtk.Align.CENTER)
            title = Gtk.Label(label=chat.title, xalign=0, ellipsize=Pango.EllipsizeMode.END)
            when = Gtk.Label(label=relative_time(chat.updated), xalign=0)
            when.add_css_class("dim-label")
            when.add_css_class("chat-row-time")
            texts.append(title)
            texts.append(when)
            delete = Gtk.Button(icon_name="user-trash-symbolic", tooltip_text="Delete Chat",
                                valign=Gtk.Align.CENTER)
            delete.add_css_class("flat")
            delete.add_css_class("circular")
            delete.connect("clicked", lambda _b, cid=chat.id: self.delete_chat(cid))
            box.append(texts)
            box.append(delete)
            row.set_child(box)
            self.chat_list.append(row)
            if chat.id == self.chat_id:
                self.chat_list.select_row(row)
        if chats:
            self.chat_stack.set_visible_child_name("list")
        else:
            self.no_chats.set_title("No Results" if query else "No Chats")
            self.no_chats.set_description(None if query else "Your conversations are saved here")
            self.chat_stack.set_visible_child_name("empty")
        return GLib.SOURCE_REMOVE

    def _focus_search(self):
        self.split.set_show_content(False)
        self.search.grab_focus()

    def delete_chat(self, chat_id: str):
        store.delete_chat(chat_id)
        if chat_id == self.chat_id:
            self.new_chat()
        self._refresh_chats()
        self.toast("Chat deleted")

    def open_chat(self, chat_id: str):
        if self.busy or chat_id == self.chat_id:
            self.split.set_show_content(True)
            return
        data = store.load_chat(chat_id)
        if data is None:
            self.toast("Couldn't open that chat")
            self._refresh_chats()
            return
        self.new_chat()
        self.chat_id = chat_id
        self.agent.messages = data["messages"]
        self._render_history(data["messages"], data.get("model") or self.agent.model)
        self.stack.set_visible_child_name("chat")
        self.split.set_show_content(True)
        self._refresh_chats()

    def _render_history(self, messages: list[dict], model: str):
        """Draw a saved chat: user bubbles, answers and finished action pills."""
        results = {m.get("tool_call_id"): m.get("content", "") for m in messages if m.get("role") == "tool"}
        reply = None
        for m in messages:
            role, content = m.get("role"), store.plain_text(m.get("content"))
            if role == "user":
                if content == "(Image from the tool.)":
                    continue
                if reply:
                    reply.finish(notify=False)
                    reply = None
                self._add_user(_shown_user_text(content), animate=False)
            elif role == "assistant":
                if reply is None:
                    reply = Reply(self, live=False, model=model)
                for call in m.get("tool_calls") or []:
                    fn = call.get("function", {})
                    try:
                        args = json.loads(fn.get("arguments") or "{}")
                    except ValueError:
                        args = {}
                    name = fn.get("name", "")
                    result = results.get(call.get("id"), "")
                    ok = not (result.startswith("Error") or result == "The user declined this action.")
                    reply.summaries[name] = T.summarize(name, args) if name in T.BY_NAME else name
                    reply._action(name, result if ok else (result.removeprefix("Error: ") or "Failed"), ok)
                if content:
                    reply._text(content)
                    reply.label = None
                    reply.answer += "\n"
        if reply:
            reply.finish(notify=False)
        self.scroll_down()

    # -- pictures ---------------------------------------------------------
    def _model_sees(self, name: str) -> bool:
        model = next((m for m in self.models if m.name == name), None)
        return bool(model and model.can_see)

    def _can_attach(self) -> bool:
        if self._model_sees(self.agent.model) or self.agent.vision_model:
            return True
        self.toast("No installed model can look at images")
        return False

    def choose_image(self):
        if not self._can_attach():
            return
        filters = Gio.ListStore.new(Gtk.FileFilter)
        images = Gtk.FileFilter(name="Images")
        for mime in IMAGE_TYPES:
            images.add_mime_type(mime)
        filters.append(images)
        dialog = Gtk.FileDialog(title="Attach Image", filters=filters, modal=True)

        def done(dlg, res):
            try:
                files = dlg.open_multiple_finish(res)
            except GLib.Error:
                return  # cancelled
            for i in range(files.get_n_items()):
                self._attach_file(files.get_item(i))

        dialog.open_multiple(self, None, done)

    def _attach_file(self, gfile: Gio.File) -> bool:
        path = gfile.get_path()
        if not path:
            return False
        try:
            info = gfile.query_info("standard::content-type", Gio.FileQueryInfoFlags.NONE, None)
            if info.get_content_type() not in IMAGE_TYPES:
                self.toast(f"{gfile.get_basename()} isn't an image")
                return False
            url = T.image_data_url(Path(path))
            thumb = Gdk.Texture.new_from_file(gfile)
        except (GLib.Error, T.ToolError) as e:
            self.toast(str(getattr(e, "message", e)))
            return False
        self._add_attachment(thumb, url)
        return True

    def _add_attachment(self, thumb, url: str):
        if len(self.attachments) >= MAX_ATTACHMENTS:
            self.toast(f"Up to {MAX_ATTACHMENTS} images per message")
            return
        self.attachments.append((thumb, url))
        self._show_attachments()
        self.preload()

    def _show_attachments(self):
        child = self.attach_box.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.attach_box.remove(child)
            child = nxt
        for index, (thumb, _url) in enumerate(self.attachments):
            pic = Gtk.Picture(paintable=thumb, content_fit=Gtk.ContentFit.COVER, width_request=56,
                              height_request=56, overflow=Gtk.Overflow.HIDDEN)
            pic.add_css_class("attachment")
            remove = Gtk.Button(icon_name="window-close-symbolic", tooltip_text="Remove",
                                halign=Gtk.Align.END, valign=Gtk.Align.START)
            remove.add_css_class("circular")
            remove.add_css_class("osd")
            remove.connect("clicked", lambda _b, i=index: self._remove_attachment(i))
            overlay = Gtk.Overlay(child=pic)
            overlay.add_overlay(remove)
            self.attach_box.append(overlay)
        self.attach_revealer.set_reveal_child(bool(self.attachments))

    def _remove_attachment(self, index: int):
        if 0 <= index < len(self.attachments):
            del self.attachments[index]
        self._show_attachments()

    def _on_paste(self, entry):
        clipboard = entry.get_clipboard()
        if not clipboard.get_formats().contain_gtype(Gdk.Texture.__gtype__):
            return
        entry.stop_emission_by_name("paste-clipboard")
        if not self._can_attach():
            return

        def done(cb, res):
            try:
                texture = cb.read_texture_finish(res)
            except GLib.Error:
                return
            if texture is None:
                return
            cache = Path(GLib.get_user_cache_dir()) / "panda-assistant"
            cache.mkdir(parents=True, exist_ok=True)
            path = cache / "pasted.png"
            texture.save_to_png(str(path))
            try:
                self._add_attachment(texture, T.image_data_url(path))
            except T.ToolError as e:
                self.toast(str(e))
            finally:
                path.unlink(missing_ok=True)

        clipboard.read_texture_async(None, done)

    def _on_drop(self, _target, value, _x, _y):
        files = value.get_files() if hasattr(value, "get_files") else []
        if not files or not self._can_attach():
            return False
        added = [self._attach_file(f) for f in files]
        return any(added)

    # -- models -----------------------------------------------------------
    def _load_models(self):
        try:
            status = self.core.status()
            self._set_state(status.get("state", "Not loaded"))
        except CoreError as e:
            self.banner.set_title(str(e))
            self.banner.set_revealed(True)
            return

        def done(models, err):
            if err:
                self.toast(f"Couldn't list models: {err}")
            self._fill_models([Model(m["name"], True, list(m.get("labels") or [])) for m in (models or [])])

        self.core.models(done)

    def _fill_models(self, models: list[Model]):
        self.models = [m for m in models if m.installed]
        if not self.models:
            self.banner.set_title("No models found. Is FastFlowLM installed?")
            self.banner.set_revealed(True)
        if self.agent.model not in [m.name for m in self.models]:
            self.models.insert(0, Model(self.agent.model, True, []))
        self.model_menu.remove_all()
        for m in self.models:
            label = m.name + ("" if m.can_use_tools or not m.labels else " (chat only)")
            item = Gio.MenuItem.new(label, None)
            item.set_action_and_target_value("win.model", GLib.Variant("s", m.name))
            self.model_menu.append_item(item)
        self._apply_model()
        return GLib.SOURCE_REMOVE

    def _apply_smarter(self):
        if not hasattr(self, "models"):
            self.models = []
        if self.smarter_choice == "off":
            self.agent.smarter_model = None
        elif self.smarter_choice == "auto":
            self.agent.smarter_model = pick_smarter_model(self.models, self.agent.model)
        else:
            self.agent.smarter_model = self.smarter_choice

    def _on_model(self, action, value):
        name = value.unpack()
        if self.busy or name == self.agent.model:
            return
        action.set_state(value)
        self.agent.model = name
        self.cfg["model"] = name
        save_config(self.cfg)
        self.core.reload()
        self.new_chat()
        self._apply_model()
        self.toast(f"Switched to {name}")

    def _apply_model(self):
        self._show_state()
        self._apply_smarter()
        self.agent.vision_model = pick_vision_model(self.models, self.agent.model)
        model = next((m for m in self.models if m.name == self.agent.model), None)
        self.agent.use_tools = True
        self.banner.set_revealed(False)
        if model and model.labels:
            self.agent.use_tools = model.can_use_tools
            if model.single_turn:
                self.banner.set_title("This model doesn't remember earlier messages and can't take actions")
                self.banner.set_revealed(True)
            elif not model.can_use_tools:
                self.banner.set_title("This model can chat but can't take actions")
                self.banner.set_revealed(True)

    # -- memory: the service loads and unloads models ---------------------
    def _on_core_state(self, model: str, state: str):
        self._set_state(state)

    def _set_state(self, state: str):
        self.state = state
        self._show_state()
        self.unload_action.set_enabled(state == "Loaded" and not self.busy)
        return GLib.SOURCE_REMOVE

    def _show_state(self):
        self.title.set_subtitle(f"{self.agent.model} · {self.state}")

    def preload(self):
        """Ask the service to load the model while the question is typed."""
        if self.state in ("Loaded", "Loading…") or self._loading:
            return
        self._loading = True
        GLib.timeout_add_seconds(5, lambda: setattr(self, "_loading", False) or GLib.SOURCE_REMOVE)
        self.core.load()

    def _cancel_idle(self):
        if self._idle_id:
            GLib.source_remove(self._idle_id)
            self._idle_id = 0

    def _schedule_idle(self):
        """With no window, quit soon after answering; the service keeps the model."""
        self._cancel_idle()
        if not self.busy and not self.get_visible():
            self._idle_id = GLib.timeout_add_seconds(HIDDEN_QUIT_SECONDS, self._on_idle)
        return GLib.SOURCE_REMOVE

    def _on_idle(self):
        self._idle_id = 0
        if not self.busy and not self.get_visible():
            self.destroy()  # the last window: the app exits
        return GLib.SOURCE_REMOVE

    def unload(self, manual: bool):
        """Free the model's memory (the service stops FastFlowLM)."""
        if self.busy:
            return
        self.core.unload()
        if manual:
            self.toast("Model unloaded")

    def reload_core(self):
        """Settings changed: the service reads them before the next question."""
        self.core.reload()

    def show_preferences(self):
        page = Adw.PreferencesPage(title="General", icon_name="preferences-system-symbolic")
        group = Adw.PreferencesGroup(title="Memory", description="The default model stays loaded so answers start "
                                     "quickly; set a timer to unload it when idle. Bigger models never stay.")
        ctx_row = Adw.ComboRow(title="Context Length", subtitle="Longer conversations need more memory",
                               model=Gtk.StringList.new([label for _, label in CTX_CHOICES]))
        ctx_values = [v for v, _ in CTX_CHOICES]
        ctx_row.set_selected(ctx_values.index(self.ctx_len) if self.ctx_len in ctx_values else 1)
        idle_row = Adw.ComboRow(title="Unload Model", subtitle="Frees memory when the assistant isn't used",
                                model=Gtk.StringList.new([label for _, label in IDLE_CHOICES]))
        idle_values = [v for v, _ in IDLE_CHOICES]
        idle_row.set_selected(idle_values.index(self.unload_after) if self.unload_after in idle_values else len(idle_values) - 1)

        def on_ctx(row, _p):
            self.ctx_len = ctx_values[row.get_selected()]
            self.cfg["ctx_len"] = self.ctx_len
            save_config(self.cfg)
            self.reload_core()  # the service restarts FLM with the new length on next use

        def on_idle(row, _p):
            self.unload_after = idle_values[row.get_selected()]
            self.cfg["unload_after_minutes"] = self.unload_after
            save_config(self.cfg)
            self.reload_core()

        abilities = Adw.PreferencesGroup(title="Abilities")
        web_row = Adw.SwitchRow(title="Search the Web", active=self.agent.web_enabled,
                                subtitle="Look things up with DuckDuckGo and read web pages")

        def on_web(row, _p):
            self.agent.web_enabled = row.get_active()
            self.cfg["web_search"] = self.agent.web_enabled
            save_config(self.cfg)
            self.reload_core()

        web_row.connect("notify::active", on_web)
        abilities.add(web_row)

        allowed = Adw.PreferencesGroup(title="Allowed Without Asking",
                                       description="Actions you chose “Always Allow” for")
        allowed_rows: list[Gtk.Widget] = []

        def ask_again(name: str):
            self.agent.always_allow.discard(name)
            self.cfg["always_allow"] = sorted(self.agent.always_allow)
            self.cfg["_changed"] = ("always_allow",)
            save_config(self.cfg)
            self.reload_core()
            fill_allowed()

        def fill_allowed():
            for r in allowed_rows:
                allowed.remove(r)
            allowed_rows.clear()
            # The service writes Always Allow choices to the shared settings file.
            self.agent.always_allow = set(load_config().get("always_allow", []))
            for name in sorted(self.agent.always_allow):
                server, _, tool = name.partition("__")
                title = f"{server}: {tool}" if tool else name.replace("_", " ").capitalize()
                row = Adw.ActionRow(title=GLib.markup_escape_text(title))
                ask = Gtk.Button(label="Ask Again", valign=Gtk.Align.CENTER)
                ask.connect("clicked", lambda _b, n=name: ask_again(n))
                row.add_suffix(ask)
                allowed_rows.append(row)
                allowed.add(row)
            allowed.set_visible(bool(allowed_rows))

        fill_allowed()

        smart_group = Adw.PreferencesGroup(
            title="Smarter Model",
            description="When a request is too hard, the model can hand it to a bigger one. "
                        "Only one model is loaded at a time, so the hand-off takes a moment. "
                        "A minute after it's done, the bigger model is unloaded and the default one "
                        "is loaded again.")
        candidates = [m.name for m in self.models
                      if m.installed and m.can_use_tools and not m.single_turn and m.name != self.agent.model]
        auto_pick = pick_smarter_model(self.models, self.agent.model)
        smart_values = ["auto", "off"] + candidates
        smart_labels = [f"Automatic ({auto_pick})" if auto_pick else "Automatic (none available)", "Off"] + candidates
        smart_row = Adw.ComboRow(title="Hand Hard Requests To", model=Gtk.StringList.new(smart_labels))
        smart_row.set_selected(smart_values.index(self.smarter_choice) if self.smarter_choice in smart_values else 0)

        def on_smart(row, _p):
            self.smarter_choice = smart_values[row.get_selected()]
            self.cfg["smarter_model"] = self.smarter_choice
            save_config(self.cfg)
            self._apply_smarter()
            self.reload_core()

        smart_row.connect("notify::selected", on_smart)
        smart_group.add(smart_row)

        ctx_row.connect("notify::selected", on_ctx)
        idle_row.connect("notify::selected", on_idle)
        group.add(ctx_row)
        group.add(idle_row)
        page.add(abilities)
        page.add(allowed)
        page.add(smart_group)
        page.add(group)
        pages = [page, self._memory_page(), self._skills_page(), self._connections_page()]
        if hasattr(Adw, "PreferencesDialog"):  # libadwaita 1.5+
            dialog = Adw.PreferencesDialog()
            for p in pages:
                dialog.add(p)
            dialog.present(self)
        else:
            win = Adw.PreferencesWindow(transient_for=self, modal=True)
            for p in pages:
                win.add(p)
            win.present()

    def _memory_page(self) -> Adw.PreferencesPage:
        page = Adw.PreferencesPage(title="Memory", icon_name="starred-symbolic")
        top = Adw.PreferencesGroup(description="The assistant remembers what you ask it to, in every new chat. "
                                               "It's kept on this computer only.")
        switch = Adw.SwitchRow(title="Remember Things", active=self.agent.memory_enabled)

        def on_switch(row, _p):
            self.agent.memory_enabled = row.get_active()
            self.cfg["memory"] = self.agent.memory_enabled
            save_config(self.cfg)
            self.reload_core()

        switch.connect("notify::active", on_switch)
        top.add(switch)
        page.add(top)
        facts = Adw.PreferencesGroup(title="Remembered")
        page.add(facts)
        rows: list[Gtk.Widget] = []

        def fill():
            for r in rows:
                facts.remove(r)
            rows.clear()
            add = Adw.EntryRow(title="Add Something to Remember", show_apply_button=True)
            add.connect("apply", lambda row: (self._remember(row.get_text()), fill()))
            rows.append(add)
            for fact in store.facts():
                row = Adw.ActionRow(title=GLib.markup_escape_text(fact), title_lines=0)
                delete = Gtk.Button(icon_name="user-trash-symbolic", tooltip_text="Forget",
                                    valign=Gtk.Align.CENTER)
                delete.add_css_class("flat")
                delete.connect("clicked", lambda _b, f=fact: (store.forget(f), self._notes_changed(), fill()))
                row.add_suffix(delete)
                rows.append(row)
            for r in rows:
                facts.add(r)

        fill()
        return page

    def _connections_page(self) -> Adw.PreferencesPage:
        page = Adw.PreferencesPage(title="Connections", icon_name="network-server-symbolic")
        group = Adw.PreferencesGroup(
            title="Apps and Services",
            description="The assistant can use tools that other apps and services offer through the Model Context "
                        "Protocol (MCP). Tools that change things ask first. Each one uses some of the "
                        f"model's memory, so at most {mcp.MAX_TOOLS}.")
        buttons = Gtk.Box(spacing=6)
        add = Gtk.Button(icon_name="list-add-symbolic", tooltip_text="Add Server", valign=Gtk.Align.CENTER)
        add.add_css_class("flat")
        edit = Gtk.Button(icon_name="document-edit-symbolic", tooltip_text="Edit Configuration File",
                          valign=Gtk.Align.CENTER)
        edit.add_css_class("flat")
        buttons.append(edit)
        buttons.append(add)
        group.set_header_suffix(buttons)
        page.add(group)
        rows: list[Gtk.Widget] = []

        def status(server: dict) -> str:
            if server["disabled"]:
                return "Off"
            if server["error"]:
                return f"Couldn't connect: {server['error']}"
            if server["running"]:
                n = server["tools"]
                return f"{n} tool{'s' if n != 1 else ''}" + (" · trusted" if server["trust"] else "")
            return "Connects when you send a message"

        def changed():
            try:
                self.core.reload(wait=True)
            except CoreError as e:
                self.toast(str(e))
            fill()

        def set_enabled(name: str, on: bool):
            configs = mcp.load_config()
            for c in configs:
                if c.name == name:
                    c.disabled = not on
            mcp.save_config(configs)
            changed()

        def remove(name: str):
            mcp.save_config([c for c in mcp.load_config() if c.name != name])
            changed()

        def fill():
            for r in rows:
                group.remove(r)
            rows.clear()
            try:
                info = self.core.connections()
            except CoreError as e:
                info = {"servers": [], "config_error": str(e)}
            if info.get("config_error"):
                rows.append(Adw.ActionRow(title="The configuration file has a mistake",
                                          subtitle=GLib.markup_escape_text(info["config_error"])))
            elif not info["servers"]:
                rows.append(Adw.ActionRow(title="No servers yet", subtitle="Add one with the + button"))
            for server in info["servers"]:
                row = Adw.ActionRow(title=GLib.markup_escape_text(server["name"]),
                                    subtitle=GLib.markup_escape_text(f"{status(server)}\n{server['target']}"),
                                    subtitle_lines=3)
                switch = Gtk.Switch(active=not server["disabled"], valign=Gtk.Align.CENTER)
                switch.connect("notify::active", lambda sw, _p, n=server["name"]: set_enabled(n, sw.get_active()))
                delete = Gtk.Button(icon_name="user-trash-symbolic", tooltip_text="Remove Server",
                                    valign=Gtk.Align.CENTER)
                delete.add_css_class("flat")
                delete.connect("clicked", lambda _b, n=server["name"]: remove(n))
                row.add_suffix(switch)
                row.add_suffix(delete)
                rows.append(row)
            for r in rows:
                group.add(r)

        def open_file(*_):
            path = mcp.config_path()
            if not path.exists():
                mcp.save_config([])
            Gtk.FileLauncher.new(Gio.File.new_for_path(str(path))).launch(self, None, None)

        def add_server(*_):
            name = Adw.EntryRow(title="Name")
            target = Adw.EntryRow(title="Command or URL")
            box = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
            box.add_css_class("boxed-list")
            box.append(name)
            box.append(target)
            dialog = Adw.AlertDialog(heading="Add Server",
                                     body="The command that starts the server, e.g. “npx -y "
                                          "@modelcontextprotocol/server-memory”, or its web address.",
                                     extra_child=box)
            dialog.add_response("cancel", "_Cancel")
            dialog.add_response("add", "_Add")
            dialog.set_response_appearance("add", Adw.ResponseAppearance.SUGGESTED)
            dialog.set_default_response("add")

            def on_response(_d, response):
                if response != "add":
                    return
                try:
                    cfg = mcp.parse_server(name.get_text(), target.get_text())
                    configs = [c for c in mcp.load_config() if c.name != cfg.name]
                    mcp.save_config(configs + [cfg])
                except (mcp.McpError, OSError) as e:
                    self.toast(str(e).capitalize())
                    return
                changed()

            dialog.connect("response", on_response)
            dialog.present(page.get_root())

        add.connect("clicked", add_server)
        edit.connect("clicked", open_file)
        fill()
        return page

    def _remember(self, text: str):
        try:
            store.remember(text)
        except ValueError as e:
            self.toast(str(e).capitalize())
        self._notes_changed()

    def _notes_changed(self):
        if not self.busy:
            self.reload_core()  # the service picks up memory and skills on the next message

    def _skills_page(self) -> Adw.PreferencesPage:
        page = Adw.PreferencesPage(title="Skills", icon_name="view-list-bullet-symbolic")
        group = Adw.PreferencesGroup(
            title="Skills",
            description="Saved workflows. Ask “save that as a skill”, or add Markdown files to the "
                        "skills folder. Run one by typing / and its name.")
        folder = Gtk.Button(icon_name="folder-open-symbolic", tooltip_text="Open Skills Folder",
                            valign=Gtk.Align.CENTER)
        folder.add_css_class("flat")

        def open_folder(*_):
            store.skills_dir().mkdir(parents=True, exist_ok=True)
            Gtk.FileLauncher.new(Gio.File.new_for_path(str(store.skills_dir()))).launch(self, None, None)

        folder.connect("clicked", open_folder)
        group.set_header_suffix(folder)
        page.add(group)
        rows: list[Gtk.Widget] = []

        def fill():
            for r in rows:
                group.remove(r)
            rows.clear()
            skills = store.skills()
            if not skills:
                rows.append(Adw.ActionRow(title="No skills yet", subtitle="Try: “Save a skill called morning: "
                                          "turn on dark mode, set volume to 30%”"))
            for skill in skills:
                row = Adw.ActionRow(title=GLib.markup_escape_text(skill.title),
                                    subtitle=GLib.markup_escape_text(f"/{skill.slug}" + (
                                        f" · {skill.description}" if skill.description else "")))
                delete = Gtk.Button(icon_name="user-trash-symbolic", tooltip_text="Delete Skill",
                                    valign=Gtk.Align.CENTER)
                delete.add_css_class("flat")
                delete.connect("clicked", lambda _b, slug=skill.slug: (store.delete_skill(slug),
                                                                      self._notes_changed(), fill()))
                row.add_suffix(delete)
                rows.append(row)
            for r in rows:
                group.add(r)

        fill()
        return page

    # -- chat -------------------------------------------------------------
    def new_chat(self):
        if self.busy:
            self.stop()
        self.agent.reset()
        self.chat_id = store.new_chat_id()
        self.attachments.clear()
        self._show_attachments()
        self.chat_list.unselect_all()
        child = self.messages.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.messages.remove(child)
            child = nxt
        self.greeting.set_label(greeting())
        self.split.set_show_content(True)
        self.stack.set_visible_child_name("empty")
        self.entry.grab_focus()

    def ask(self, text: str):
        if self.busy:
            self.entry.set_text(text)
            return
        self.entry.set_text(text)
        self.send()

    def ask_from_shell(self, text: str):
        """A question from GNOME search: answer in a notification unless the window is open."""
        stale = self.last_activity and time.time() - self.last_activity > STALE_MINUTES * 60
        if stale and not self.busy and self.agent.messages:
            self.new_chat()
        if self.get_visible():
            self.present()
            self.ask(text)
            return
        if self.busy:
            self.app.notify(f"{APP_NAME} is busy", "Still working on the last question.", self.chat_id)
            return
        self.app.notify(f"Asking {APP_NAME}…", text, self.chat_id)
        self.ask(text)

    def send_or_stop(self):
        self.stop() if self.busy else self.send()

    def send(self):
        text = self.entry.get_text().strip()
        if self.busy or not (text or self.attachments):
            return
        prompt = text or "What's in this picture?"
        if text.startswith("/"):
            name, _, details = text[1:].partition(" ")
            skill = store.find_skill(name)
            if skill is None:
                names = ", ".join("/" + s.slug for s in store.skills())
                self.toast(f"No skill called /{name}" + (f". Skills: {names}" if names else ""))
                return
            prompt = store.skill_prompt(skill, details)
        images = [url for _thumb, url in self.attachments]
        thumbs = [thumb for thumb, _url in self.attachments]
        self.entry.set_text("")
        self.attachments.clear()
        self._show_attachments()
        self.stack.set_visible_child_name("chat")
        self._add_user(text, thumbs=thumbs)
        self._cancel_idle()
        self._set_busy(True)
        self.reply = Reply(self)
        try:
            self.core.ask(self.chat_id, prompt, images, self.reply)
        except CoreError as e:
            self.reply.on_finished(str(e))

    def stop(self):
        if self.busy:
            self.core.stop()

    def turn_finished(self, reply: "Reply"):
        """The service finished a turn (it has saved the chat)."""
        data = store.load_chat(self.chat_id)
        if data:
            self.agent.messages = data["messages"]
        self.last_activity = time.time()
        self.reply = None
        self._refresh_chats()
        self._set_busy(False)
        self._schedule_idle()

    def _set_busy(self, busy: bool):
        self.busy = busy
        self.send_btn.set_icon_name("media-playback-stop-symbolic" if busy else "go-up-symbolic")
        self.send_btn.set_tooltip_text("Stop" if busy else "Send")
        for cls in ("suggested-action", "destructive-action"):
            self.send_btn.remove_css_class(cls)
        self.send_btn.add_css_class("destructive-action" if busy else "suggested-action")
        self.model_action.set_enabled(not busy)
        self.unload_action.set_enabled(self.state == "Loaded" and not busy)
        if not busy:
            self.entry.grab_focus()
        return GLib.SOURCE_REMOVE

    def _add_user(self, text: str, thumbs=(), animate: bool = True):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, halign=Gtk.Align.END, margin_start=48)
        if thumbs:
            pics = Gtk.Box(spacing=6, halign=Gtk.Align.END)
            for thumb in thumbs:
                pic = Gtk.Picture(paintable=thumb, content_fit=Gtk.ContentFit.COVER, width_request=120,
                                  height_request=90, overflow=Gtk.Overflow.HIDDEN)
                pic.add_css_class("attachment")
                pics.append(pic)
            box.append(pics)
        shown = text.replace(IMAGE_MARKER, "").strip()
        if shown or not thumbs:
            label = Gtk.Label(label=shown or text, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, xalign=0,
                              selectable=True, halign=Gtk.Align.END, max_width_chars=52)
            label.add_css_class("bubble-user")
            box.append(label)
        if IMAGE_MARKER in text and not thumbs:
            note = Gtk.Label(label="Image not saved", halign=Gtk.Align.END)
            note.add_css_class("dim-label")
            note.add_css_class("caption")
            box.append(note)
        if animate:
            self.append_animated(box)
        else:
            self.messages.append(box)

    def scroll_down(self):
        def go():
            adj = self.scroller.get_vadjustment()
            adj.set_value(adj.get_upper() - adj.get_page_size())
            return GLib.SOURCE_REMOVE
        GLib.timeout_add(30, go)

    def in_view(self) -> bool:
        """Shown and focused: the user is looking at this window."""
        return self.get_visible() and self.is_active()

    def _step_aside(self):
        """Minimize so the window behind becomes the focused one (screenshots, read_window)."""
        self._stepped_aside = self.in_view()
        if self._stepped_aside:
            self.minimize()

    def _step_back(self):
        if getattr(self, "_stepped_aside", False):
            self._stepped_aside = False
            self.present()

    def copy_text(self, text: str):
        Gdk.Display.get_default().get_clipboard().set(text)

    def show_shortcuts(self):
        sections = [("General", [("New Chat", "<Ctrl>n"), ("Focus Message Field", "<Ctrl>l"),
                                 ("Search Chats", "<Ctrl>f"), ("Bigger Text", "<Ctrl>plus"),
                                 ("Smaller Text", "<Ctrl>minus"), ("Normal Text Size", "<Ctrl>0"),
                                 ("Attach Image", "<Ctrl><Shift>a"),
                                 ("Stop Answer", "Escape"), ("Preferences", "<Ctrl>comma"),
                                 ("Keyboard Shortcuts", "<Ctrl>question"),
                                 ("Close Window", "<Ctrl>w"), ("Quit", "<Ctrl>q")])]
        if hasattr(Adw, "ShortcutsDialog"):  # libadwaita 1.8+
            dialog = Adw.ShortcutsDialog()
            for title, items in sections:
                section = Adw.ShortcutsSection(title=title)
                for label, accel in items:
                    section.add(Adw.ShortcutsItem(title=label, accelerator=accel))
                dialog.add(section)
            dialog.present(self)
            return
        win = Gtk.ShortcutsWindow(transient_for=self, modal=True)
        section = Gtk.ShortcutsSection(visible=True)
        for title, items in sections:
            group = Gtk.ShortcutsGroup(title=title)
            for label, accel in items:
                group.append(Gtk.ShortcutsShortcut(title=label, accelerator=accel))
            section.append(group)
        win.set_child(section)
        win.present()

    def _on_close(self, *_):
        self.cfg["maximized"] = self.is_maximized()
        if not self.is_maximized():
            self.cfg["width"], self.cfg["height"] = self.get_default_size()
        save_config(self.cfg)
        self.stop()
        return False


class Reply:
    """One assistant turn in the window; its methods run on the GTK thread."""

    def __init__(self, win: Window, live: bool = True, model: str | None = None):
        self.win = win
        self.summaries: dict[str, str] = {}
        # Avatar on the left, name and the turn's content on the right.
        row = Gtk.Box(spacing=12)
        row.append(Gtk.Image(icon_name=APP_ID, pixel_size=32, valign=Gtk.Align.START))
        self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, hexpand=True)
        name = Gtk.Box(spacing=8)
        title = Gtk.Label(label=APP_NAME, xalign=0)
        title.add_css_class("assistant-name")
        self.model_label = Gtk.Label(label=model or win.agent.model, xalign=0)
        self.model_label.add_css_class("dim-label")
        self.model_label.add_css_class("caption")
        name.append(title)
        name.append(self.model_label)
        self.box.append(name)
        row.append(self.box)
        self.thought = Gtk.Label(wrap=True, xalign=0, selectable=True)
        self.thought.add_css_class("dim-label")
        self.thought.add_css_class("thought")
        self.thinking = Gtk.Expander(label="Thinking…", child=self.thought, visible=False)
        # The "working" pill: spinner + label, removed when the turn ends.
        self.spin = Gtk.Box(spacing=8, halign=Gtk.Align.START)
        self.spin.add_css_class("pill")
        self.spin.add_css_class("quiet")
        self.spin.append(spinner())
        self.spin_label = Gtk.Label(label="Thinking…")
        self.spin.append(self.spin_label)
        self.failure = ""
        self.text = ""          # text of the current message label
        self.answer = ""        # everything the model said this turn
        self.label: Gtk.Label | None = None
        self.box.append(self.thinking)
        if live:
            self.box.append(self.spin)
            win.append_animated(row)
        else:
            win.messages.append(row)

    # -- events from the service (see core_client.Turn), on the GTK thread --
    def on_text(self, s: str):
        self._text(s)

    def on_reasoning(self, s: str):
        self._reasoning(s)

    def on_tool_started(self, name: str, summary: str):
        self._tool_started(name, summary)

    def on_tool_finished(self, name: str, result: str, ok: bool):
        self._action(name, result, ok)

    def on_notice(self, text: str):
        self.win.toast(text)

    def on_handoff(self, model: str):
        self._handoff(model)

    def on_compacting(self):
        self.spin_label.set_label("Summarizing older messages…")

    def on_confirm(self, request: int, name: str, summary: str, detail: str, can_always: bool):
        self._confirm_card(request, name, summary, detail, can_always)

    def on_finished(self, error: str):
        if error:
            self.error(error)
        self.finish()
        self.win.turn_finished(self)

    def _before_spinner(self, widget: Gtk.Widget):
        if self.spin.get_parent():
            self.box.insert_child_after(widget, self.spin.get_prev_sibling())
        else:
            self.box.append(widget)

    def _tool_started(self, name: str, summary: str):
        self.summaries[name] = summary
        self.spin_label.set_label(summary + "…")
        return GLib.SOURCE_REMOVE

    def _text(self, s: str):
        self.spin_label.set_label("Writing…")
        if self.label is None:
            self.label = Gtk.Label(wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, xalign=0, selectable=True)
            self.label.add_css_class("assistant-text")
            self._before_spinner(self.label)
            self.text = ""
        self.text += s
        self.answer += s
        try:
            self.label.set_markup(to_pango(self.text.lstrip()))
        except GLib.Error:
            self.label.set_text(self.text.lstrip())
        if self.spin.get_parent():
            self.win.scroll_down()
        return GLib.SOURCE_REMOVE

    def _reasoning(self, s: str):
        self.thinking.set_visible(True)
        self.spin_label.set_label("Thinking…")
        self.thought.set_text(self.thought.get_text() + s)
        return GLib.SOURCE_REMOVE

    def _pill(self, icon: str, text: str, failed: bool = False, tooltip: str | None = None) -> Gtk.Widget:
        pill = Gtk.Box(spacing=8, halign=Gtk.Align.START, tooltip_text=tooltip)
        pill.add_css_class("pill")
        if failed:
            pill.add_css_class("failed")
        pill.append(Gtk.Image(icon_name=icon))
        pill.append(Gtk.Label(label=text, wrap=True, xalign=0))
        return pill

    def _action(self, name: str, result: str, ok: bool):
        short = len(result) <= 90 and "\n" not in result
        if ok:
            # Short confirmations ("Dark mode on.") read best; long results
            # (search hits, page text) go in the tooltip, the model relays them.
            text = result if short else self.summaries.get(name, name)
            icon = TOOL_ICONS.get(name, "network-server-symbolic" if "__" in name else "emblem-system-symbolic")
            pill = self._pill(icon, text,
                              tooltip=None if short else result[:600])
        else:
            pill = self._pill("dialog-warning-symbolic", result if short else result[:200], failed=True)
        self._before_spinner(pill)
        self.label = None  # text after an action starts a new message
        self.answer += "\n"
        self.win.scroll_down()
        return GLib.SOURCE_REMOVE

    def _handoff(self, model: str):
        self._before_spinner(self._pill("go-next-symbolic", f"Handed to {model}"))
        self.model_label.set_label(model)
        self.spin_label.set_label(f"Loading {model}…")
        self.label = None
        self.answer = ""  # the bigger model's reply is the answer
        self.win.scroll_down()
        return GLib.SOURCE_REMOVE

    def _confirm_card(self, request: int, name: str, summary: str, detail: str, can_always: bool):
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        card.add_css_class("card")
        card.add_css_class("confirm-card")
        head = Gtk.Box(spacing=10, margin_top=12, margin_start=12, margin_end=12)
        head.append(Gtk.Image(icon_name="dialog-question-symbolic", pixel_size=20))
        title = Gtk.Label(label=f"{summary}?", xalign=0, wrap=True, hexpand=True)
        title.add_css_class("heading")
        head.append(title)
        buttons = Gtk.Box(spacing=6, halign=Gtk.Align.END, margin_bottom=12, margin_end=12)
        cancel = Gtk.Button(label="_Cancel", use_underline=True)
        allow = Gtk.Button(label="_Allow", use_underline=True)
        allow.add_css_class("suggested-action")
        buttons.append(cancel)
        if can_always:
            always = Gtk.Button(label="Always A_llow", use_underline=True,
                                tooltip_text="Don't ask again for this kind of action")
            always.connect("clicked", lambda *_: (self.win.agent.always_allow.add(name), pick("always")))
            buttons.append(always)
        buttons.append(allow)
        card.append(head)
        if detail:
            code = Gtk.Label(label=detail, xalign=0, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR,
                             selectable=True, margin_start=12, margin_end=12)
            code.add_css_class("monospace")
            code.add_css_class("confirm-detail")
            card.append(code)
        card.append(buttons)
        self._before_spinner(card)
        self.label = None
        self.spin_label.set_label("Waiting for you…")
        self.win.scroll_down()
        if not self.win.in_view():
            self.win.app.notify(f"{APP_NAME} needs your OK", f"{summary}?", self.win.chat_id,
                                button="Review")

        def pick(answer: str):
            self.spin_label.set_label("Working…")
            card.remove(buttons)
            title.remove_css_class("heading")
            title.set_label(f"{summary}: {'cancelled' if answer == 'deny' else 'allowed'}")
            self.win.core.confirm(request, answer)

        allow.connect("clicked", lambda *_: pick("allow"))
        cancel.connect("clicked", lambda *_: pick("deny"))
        allow.grab_focus()

    def error(self, text: str):
        self.failure = text
        label = Gtk.Label(label=text, wrap=True, xalign=0, selectable=True)
        label.add_css_class("error")
        self._before_spinner(label)
        return GLib.SOURCE_REMOVE

    def finish(self, notify: bool = True):
        if self.spin.get_parent():
            self.box.remove(self.spin)
        if self.thinking.get_visible():
            self.thinking.set_label("Thoughts")
        answer = self.answer.strip()
        if answer:
            copy = Gtk.Button(icon_name="edit-copy-symbolic", tooltip_text="Copy", halign=Gtk.Align.START)
            copy.add_css_class("flat")
            copy.add_css_class("circular")
            copy.connect("clicked", lambda *_: (self.win.copy_text(answer), self.win.toast("Copied")))
            self.box.append(copy)
        if notify and not self.win.in_view():
            if answer:
                self.win.app.notify(APP_NAME, plain(answer), self.win.chat_id)
            elif self.failure:
                self.win.app.notify(f"{APP_NAME} couldn't answer", self.failure, self.win.chat_id)
        return GLib.SOURCE_REMOVE


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.add_main_option("ask", 0, GLib.OptionFlags.NONE, GLib.OptionArg.STRING, "Ask a question", "TEXT")
        self.add_main_option("open-chat", 0, GLib.OptionFlags.NONE, GLib.OptionArg.STRING, "Open a saved chat", "ID")
        self.add_main_option("new-chat", 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, "Start a new chat", None)
        self.add_main_option("unload", 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, "Unload the model", None)
        self.add_main_option("version", 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, "Show the version", None)
        self.window: Window | None = None
        self.connect("shutdown", self._on_shutdown)

    def do_startup(self):
        Adw.Application.do_startup(self)
        css = Gtk.CssProvider()
        if hasattr(css, "load_from_string"):
            css.load_from_string(CSS)
        else:
            css.load_from_data(CSS.encode())
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css,
                                                  Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        dev_icons = Path(__file__).resolve().parents[2] / "data"  # running from a checkout
        if (dev_icons / f"{APP_ID}.svg").exists():
            Gtk.IconTheme.get_for_display(Gdk.Display.get_default()).add_search_path(str(dev_icons))
        for name, fn in (("quit", self.quit), ("about", self.show_about), ("activate", self.do_activate)):
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", lambda *_a, f=fn: f())
            self.add_action(action)
        # Used by the GNOME search provider through org.freedesktop.Application.
        ask = Gio.SimpleAction.new("ask", GLib.VariantType.new("s"))
        ask.connect("activate", lambda _a, v: self._window().ask_from_shell(v.unpack()))
        self.add_action(ask)
        open_chat = Gio.SimpleAction.new("open-chat", GLib.VariantType.new("s"))
        open_chat.connect("activate", lambda _a, v: self._present().open_chat(v.unpack()))
        self.add_action(open_chat)
        # Right-click actions of the app icon (see the desktop file).
        for name, fn in (("new-chat", lambda: self._present().new_chat()),
                         ("look-at-screen", lambda: self._present().ask("What's on my screen?")),
                         ("unload-model", self._unload)):
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", lambda *_a, f=fn: f())
            self.add_action(action)
        for action, accels in (("app.quit", ["<Ctrl>q"]), ("window.close", ["<Ctrl>w"]),
                               ("win.new-chat", ["<Ctrl>n"]), ("win.focus-entry", ["<Ctrl>l"]),
                               ("win.stop", ["Escape"]), ("win.show-help-overlay", ["<Ctrl>question"]),
                               ("win.preferences", ["<Ctrl>comma"]), ("win.zoom-in", ["<Ctrl>plus", "<Ctrl>equal"]),
                               ("win.zoom-out", ["<Ctrl>minus"]), ("win.zoom-reset", ["<Ctrl>0"]),
                               ("win.search-chats", ["<Ctrl>f"]), ("win.attach", ["<Ctrl><Shift>a"])):
            self.set_accels_for_action(action, accels)

    def _unload(self):
        if self.window:
            self.window.unload(manual=True)

    def _window(self) -> Window:
        """The window, created hidden if there is none yet."""
        if self.window is None:
            self.window = Window(self)
            self.window.connect("destroy", lambda *_: setattr(self, "window", None))
        return self.window

    def notify(self, title: str, body: str, chat_id: str, button: str = "Open Chat"):
        """One notification at a time; clicking it opens the chat."""
        note = Gio.Notification.new(title)
        note.set_body(body)
        note.set_default_action_and_target_value("app.open-chat", GLib.Variant("s", chat_id))
        note.add_button_with_target_value(button, "app.open-chat", GLib.Variant("s", chat_id))
        self.send_notification("panda", note)

    def _present(self) -> Window:
        win = self._window()
        win.present()
        self.withdraw_notification("panda")
        return win

    def do_activate(self):
        self._present()

    def do_command_line(self, cmdline):
        opts = cmdline.get_options_dict().end().unpack()
        if opts.get("version"):
            cmdline.print_literal(f"{APP_NAME} {VERSION}\n")
            return 0
        if opts.get("unload"):
            self._unload()  # without opening a window
            return 0
        win = self._present()
        if opts.get("ask"):
            win.ask(opts["ask"])
        elif opts.get("open-chat"):
            win.open_chat(opts["open-chat"])
        elif opts.get("new-chat"):
            win.new_chat()
        return 0

    def show_about(self):
        kwargs = dict(application_name=APP_NAME, application_icon=APP_ID, version=VERSION,
                      developer_name="Wiertless", website=REPO_URL, license_type=Gtk.License.MIT_X11, issue_url=REPO_URL + "/issues",
                      comments="A local assistant that runs on your NPU with FastFlowLM "
                               "and can act on your desktop.")
        if hasattr(Adw, "AboutDialog"):  # libadwaita 1.5+
            Adw.AboutDialog(**kwargs).present(self.window)
        else:
            Adw.AboutWindow(transient_for=self.window, **kwargs).present()

    def _on_shutdown(self, *_):
        # The service keeps running (and keeps or frees the model per its
        # settings); just stop a turn this window started.
        if self.window and self.window.busy:
            self.window.core.stop()


def main(argv) -> int:
    return App().run(argv)
