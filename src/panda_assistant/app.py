"""GTK4 / libadwaita chat window, laid out after the GNOME HIG."""

from __future__ import annotations

import json
import os
import threading
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

from . import APP_ID, APP_NAME, DEFAULT_MODEL, VERSION  # noqa: E402
from . import tools as T  # noqa: E402
from .agent import Agent, Callbacks  # noqa: E402
from .flm import DEFAULT_CTX_LEN, FlmClient, FlmError, Model, pick_smarter_model  # noqa: E402
from .markup import to_pango  # noqa: E402

CONFIG = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "panda-assistant" / "config.json"
REPO_URL = "https://github.com/Wiertless/panda-assistant"
CTX_CHOICES = [(4096, "4K tokens"), (8192, "8K tokens"), (16384, "16K tokens"), (32768, "32K tokens")]
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
"""

SUGGESTIONS = ["How much memory is free?", "Turn on Night Light", "Switch to power saver",
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
}


def greeting() -> str:
    from datetime import datetime
    hour = datetime.now().hour
    return ("Good morning" if 5 <= hour < 12 else "Good afternoon" if hour < 18
            else "Good evening" if hour < 23 else "Hello, night owl")


def load_config() -> dict:
    try:
        return json.loads(CONFIG.read_text())
    except (OSError, ValueError):
        return {}


def save_config(cfg: dict) -> None:
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
                         default_width=self.cfg.get("width", 600), default_height=self.cfg.get("height", 760))
        if self.cfg.get("maximized"):
            self.maximize()
        self.app = app
        self.client = FlmClient()
        self.models: list[Model] = []
        self.busy = False
        self.agent = Agent(self.client, self.cfg.get("model", DEFAULT_MODEL),
                           ctx=T.Context(copy_to_clipboard=self.copy_text))
        self.agent.web_enabled = bool(self.cfg.get("web_search", True))
        self.smarter_choice = self.cfg.get("smarter_model", "auto")  # "auto", "off" or a model
        self._apply_smarter()  # "auto" is resolved again once the model list has loaded
        self.ctx_len = int(self.cfg.get("ctx_len", DEFAULT_CTX_LEN))
        self.unload_after = int(self.cfg.get("unload_after_minutes", 5))
        self.state = "Not loaded"
        self._loading = False
        self._idle_id = 0

        # Header bar: new chat at the start, primary menu at the end, model as subtitle.
        self.title = Adw.WindowTitle(title=APP_NAME, subtitle=self.agent.model)
        header = Adw.HeaderBar(title_widget=self.title)
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

        # Composer: a rounded floating bar.
        self.entry = Gtk.Entry(hexpand=True, placeholder_text="Message Panda…")
        self.entry.connect("activate", lambda *_: self.send())
        # Start loading the model while the question is being typed.
        self.entry.connect("changed", lambda e: e.get_text().strip() and self.preload())
        self.send_btn = Gtk.Button(icon_name="go-up-symbolic", tooltip_text="Send", valign=Gtk.Align.CENTER)
        self.send_btn.add_css_class("suggested-action")
        self.send_btn.add_css_class("circular")
        self.send_btn.connect("clicked", lambda *_: self.send_or_stop())
        composer = Gtk.Box(spacing=6)
        composer.add_css_class("composer")
        composer.append(self.entry)
        composer.append(self.send_btn)
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
        self.toasts = Adw.ToastOverlay(child=view)
        self.set_content(self.toasts)

        self._add_action("new-chat", self.new_chat)
        self._add_action("focus-entry", lambda: self.entry.grab_focus())
        self._add_action("stop", self.stop)
        self._add_action("show-help-overlay", self.show_shortcuts)
        self._add_action("preferences", self.show_preferences)
        self.unload_action = self._add_action("unload", lambda: self.unload(manual=True))
        self.unload_action.set_enabled(False)
        self.model_action = Gio.SimpleAction.new_stateful("model", GLib.VariantType.new("s"),
                                                          GLib.Variant("s", self.agent.model))
        self.model_action.connect("activate", self._on_model)
        self.add_action(self.model_action)

        self.connect("close-request", self._on_close)
        self.entry.grab_focus()
        threading.Thread(target=self._load_models, daemon=True).start()

    def _build_start_page(self) -> Gtk.Widget:
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, valign=Gtk.Align.CENTER,
                      margin_start=24, margin_end=24, margin_top=24, margin_bottom=24)
        box.append(Gtk.Image(icon_name=APP_ID, pixel_size=96, margin_bottom=12))
        self.greeting = Gtk.Label(label=greeting(), wrap=True, justify=Gtk.Justification.CENTER)
        self.greeting.add_css_class("greeting")
        box.append(self.greeting)
        sub = Gtk.Label(label="Ask anything. Panda runs privately on your NPU and can search the web, "
                              "change settings and work with your apps and files.",
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

    # -- models -----------------------------------------------------------
    def _load_models(self):
        GLib.idle_add(self._set_state, "Loaded" if self.client.is_up() else "Not loaded")
        try:
            models = self.client.list_models()
        except Exception as e:  # noqa: BLE001
            GLib.idle_add(self.toast, f"Couldn't list models: {e}")
            models = []
        GLib.idle_add(self._fill_models, models)

    def _fill_models(self, models: list[Model]):
        installed = [m for m in models if m.installed]
        if not installed and not self.client.is_up() and not models:
            self.banner.set_title("FastFlowLM isn't installed")
            self.banner.set_revealed(True)
        self.models = installed or models
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
        self.new_chat()
        self._apply_model()
        self.toast(f"Switched to {name}")

    def _apply_model(self):
        self._show_state()
        self._apply_smarter()
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

    # -- memory: load on demand, unload when idle ------------------------
    def _set_state(self, state: str):
        self.state = state
        self._show_state()
        self.unload_action.set_enabled(self.client.owns_server and not self.busy)
        return GLib.SOURCE_REMOVE

    def _show_state(self):
        self.title.set_subtitle(f"{self.agent.model} · {self.state}")

    def preload(self):
        """Load the model in the background if it isn't loaded yet."""
        if self._loading or self.state == "Loaded":
            return
        self._loading = True
        self._cancel_idle()
        self._set_state("Loading…")

        def work():
            try:
                self.client.ensure_server(self.agent.model, self.ctx_len)
                GLib.idle_add(self._set_state, "Loaded")
            except FlmError as e:
                GLib.idle_add(self._set_state, "Not loaded")
                GLib.idle_add(self.toast, str(e))
            finally:
                self._loading = False
                GLib.idle_add(self._schedule_idle)
        threading.Thread(target=work, daemon=True).start()

    def _cancel_idle(self):
        if self._idle_id:
            GLib.source_remove(self._idle_id)
            self._idle_id = 0

    def _schedule_idle(self):
        self._cancel_idle()
        if self.unload_after > 0 and self.client.owns_server and not self.busy:
            self._idle_id = GLib.timeout_add_seconds(self.unload_after * 60, self._on_idle)
        return GLib.SOURCE_REMOVE

    def _on_idle(self):
        self._idle_id = 0
        self.unload(manual=False)
        return GLib.SOURCE_REMOVE

    def unload(self, manual: bool):
        """Stop the server Panda started, which frees the model's memory."""
        if self.busy or not self.client.owns_server:
            return
        self._cancel_idle()

        def work():
            stopped = self.client.stop_server()
            GLib.idle_add(self._set_state, "Not loaded")
            if stopped and manual:
                GLib.idle_add(self.toast, "Model unloaded")
        threading.Thread(target=work, daemon=True).start()

    def show_preferences(self):
        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Memory", description="The model only uses memory while it's loaded. "
                                     "It loads when you start typing and unloads when idle.")
        ctx_row = Adw.ComboRow(title="Context Length", subtitle="Longer conversations need more memory",
                               model=Gtk.StringList.new([label for _, label in CTX_CHOICES]))
        ctx_values = [v for v, _ in CTX_CHOICES]
        ctx_row.set_selected(ctx_values.index(self.ctx_len) if self.ctx_len in ctx_values else 1)
        idle_row = Adw.ComboRow(title="Unload Model", subtitle="Frees memory when Panda isn't used",
                                model=Gtk.StringList.new([label for _, label in IDLE_CHOICES]))
        idle_values = [v for v, _ in IDLE_CHOICES]
        idle_row.set_selected(idle_values.index(self.unload_after) if self.unload_after in idle_values else 1)

        def on_ctx(row, _p):
            self.ctx_len = ctx_values[row.get_selected()]
            self.cfg["ctx_len"] = self.ctx_len
            save_config(self.cfg)
            if self.client.owns_server and not self.busy:
                self.unload(manual=False)  # reloads with the new length on next use

        def on_idle(row, _p):
            self.unload_after = idle_values[row.get_selected()]
            self.cfg["unload_after_minutes"] = self.unload_after
            save_config(self.cfg)
            self._schedule_idle()

        abilities = Adw.PreferencesGroup(title="Abilities")
        web_row = Adw.SwitchRow(title="Search the Web", active=self.agent.web_enabled,
                                subtitle="Lets Panda look things up with DuckDuckGo and read web pages")

        def on_web(row, _p):
            self.agent.web_enabled = row.get_active()
            self.cfg["web_search"] = self.agent.web_enabled
            save_config(self.cfg)

        web_row.connect("notify::active", on_web)
        abilities.add(web_row)

        smart_group = Adw.PreferencesGroup(
            title="Smarter Model",
            description="When a request is too hard, the model can hand it to a bigger one. "
                        "Only one model is loaded at a time, so the hand-off takes a moment.")
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

        smart_row.connect("notify::selected", on_smart)
        smart_group.add(smart_row)

        ctx_row.connect("notify::selected", on_ctx)
        idle_row.connect("notify::selected", on_idle)
        group.add(ctx_row)
        group.add(idle_row)
        page.add(abilities)
        page.add(smart_group)
        page.add(group)
        if hasattr(Adw, "PreferencesDialog"):  # libadwaita 1.5+
            dialog = Adw.PreferencesDialog()
            dialog.add(page)
            dialog.present(self)
        else:
            win = Adw.PreferencesWindow(transient_for=self, modal=True)
            win.add(page)
            win.present()

    # -- chat -------------------------------------------------------------
    def new_chat(self):
        if self.busy:
            self.stop()
        self.agent.reset()
        child = self.messages.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.messages.remove(child)
            child = nxt
        self.greeting.set_label(greeting())
        self.stack.set_visible_child_name("empty")
        self.entry.grab_focus()

    def ask(self, text: str):
        if self.busy:
            self.entry.set_text(text)
            return
        self.entry.set_text(text)
        self.send()

    def send_or_stop(self):
        self.stop() if self.busy else self.send()

    def send(self):
        text = self.entry.get_text().strip()
        if not text or self.busy:
            return
        self.entry.set_text("")
        self.stack.set_visible_child_name("chat")
        self._add_user(text)
        self._cancel_idle()
        self._set_busy(True)
        reply = Reply(self)
        threading.Thread(target=self._worker, args=(text, reply), daemon=True).start()

    def stop(self):
        if self.busy:
            self.agent.cancel.set()

    def _worker(self, text: str, reply: "Reply"):
        try:
            if self.state != "Loaded":
                GLib.idle_add(self._set_state, "Loading…")
            self.client.ensure_server(self.agent.model, self.ctx_len)
            GLib.idle_add(self._set_state, "Loaded")
            self.agent.ask(text, reply.callbacks())
        except FlmError as e:
            GLib.idle_add(reply.error, str(e))
            if not self.client.is_up():
                GLib.idle_add(self._set_state, "Not loaded")
        except Exception as e:  # noqa: BLE001
            GLib.idle_add(reply.error, f"{type(e).__name__}: {e}")
        finally:
            GLib.idle_add(reply.finish)
            GLib.idle_add(self._set_busy, False)
            GLib.idle_add(self._schedule_idle)

    def _set_busy(self, busy: bool):
        self.busy = busy
        self.send_btn.set_icon_name("media-playback-stop-symbolic" if busy else "go-up-symbolic")
        self.send_btn.set_tooltip_text("Stop" if busy else "Send")
        for cls in ("suggested-action", "destructive-action"):
            self.send_btn.remove_css_class(cls)
        self.send_btn.add_css_class("destructive-action" if busy else "suggested-action")
        self.model_action.set_enabled(not busy)
        self.unload_action.set_enabled(self.client.owns_server and not busy)
        if not busy:
            self.entry.grab_focus()
        return GLib.SOURCE_REMOVE

    def _add_user(self, text: str):
        label = Gtk.Label(label=text, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, xalign=0,
                          selectable=True, halign=Gtk.Align.END, max_width_chars=52, margin_start=48)
        label.add_css_class("bubble-user")
        self.append_animated(label)

    def scroll_down(self):
        def go():
            adj = self.scroller.get_vadjustment()
            adj.set_value(adj.get_upper() - adj.get_page_size())
            return GLib.SOURCE_REMOVE
        GLib.timeout_add(30, go)

    def copy_text(self, text: str):
        on_main(lambda: Gdk.Display.get_default().get_clipboard().set(text))

    def show_shortcuts(self):
        sections = [("General", [("New Chat", "<Ctrl>n"), ("Focus Message Field", "<Ctrl>l"),
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

    def __init__(self, win: Window):
        self.win = win
        self.summaries: dict[str, str] = {}
        # Avatar on the left, name and the turn's content on the right.
        row = Gtk.Box(spacing=12)
        row.append(Gtk.Image(icon_name=APP_ID, pixel_size=32, valign=Gtk.Align.START))
        self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, hexpand=True)
        name = Gtk.Box(spacing=8)
        title = Gtk.Label(label=APP_NAME, xalign=0)
        title.add_css_class("assistant-name")
        self.model_label = Gtk.Label(label=win.agent.model, xalign=0)
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
        self.text = ""          # text of the current message label
        self.answer = ""        # everything the model said this turn
        self.label: Gtk.Label | None = None
        self.box.append(self.thinking)
        self.box.append(self.spin)
        win.append_animated(row)

    def callbacks(self) -> Callbacks:
        idle = GLib.idle_add
        return Callbacks(
            text=lambda s: idle(self._text, s),
            reasoning=lambda s: idle(self._reasoning, s),
            tool_started=lambda n, s: idle(self._tool_started, n, s),
            tool_finished=lambda n, r, ok: idle(self._action, n, r, ok),
            notice=lambda s: idle(self.win.toast, s),
            handoff=lambda model: idle(self._handoff, model),
            confirm=self._confirm,
        )

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
            pill = self._pill(TOOL_ICONS.get(name, "emblem-system-symbolic"), text,
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

    def _confirm(self, name: str, summary: str) -> bool:
        """Worker thread: show the action inline and wait for Allow or Cancel."""
        answered = threading.Event()
        answer = {"v": False}
        on_main(self._confirm_card, summary, answer, answered)
        while not answered.wait(0.2):
            if self.win.agent.cancel.is_set():
                return False
        return answer["v"]

    def _confirm_card(self, summary: str, answer: dict, answered: threading.Event):
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        card.add_css_class("card")
        card.add_css_class("confirm-card")
        head = Gtk.Box(spacing=10, margin_top=12, margin_start=12, margin_end=12)
        head.append(Gtk.Image(icon_name="dialog-question-symbolic", pixel_size=20))
        title = Gtk.Label(label=f"{summary}?", xalign=0, wrap=True, hexpand=True)
        title.add_css_class("heading")
        head.append(title)
        buttons = Gtk.Box(spacing=6, halign=Gtk.Align.END, margin_bottom=12, margin_end=12, homogeneous=True)
        cancel = Gtk.Button(label="_Cancel", use_underline=True)
        allow = Gtk.Button(label="_Allow", use_underline=True)
        allow.add_css_class("suggested-action")
        buttons.append(cancel)
        buttons.append(allow)
        card.append(head)
        card.append(buttons)
        self._before_spinner(card)
        self.label = None
        self.spin_label.set_label("Waiting for you…")
        self.win.scroll_down()

        def pick(value: bool):
            self.spin_label.set_label("Working…")
            answer["v"] = value
            card.remove(buttons)
            title.remove_css_class("heading")
            title.set_label(f"{summary}: {'allowed' if value else 'cancelled'}")
            answered.set()

        allow.connect("clicked", lambda *_: pick(True))
        cancel.connect("clicked", lambda *_: pick(False))
        allow.grab_focus()

    def error(self, text: str):
        label = Gtk.Label(label=text, wrap=True, xalign=0, selectable=True)
        label.add_css_class("error")
        self._before_spinner(label)
        return GLib.SOURCE_REMOVE

    def finish(self):
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
            if not self.win.is_active():
                note = Gio.Notification.new(f"{APP_NAME} answered")
                note.set_body(answer[:200])
                note.set_default_action("app.activate")
                self.win.app.send_notification("answer", note)
        return GLib.SOURCE_REMOVE


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.add_main_option("ask", 0, GLib.OptionFlags.NONE, GLib.OptionArg.STRING, "Ask a question", "TEXT")
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
        ask.connect("activate", lambda _a, v: self._present().ask(v.unpack()))
        self.add_action(ask)
        for action, accels in (("app.quit", ["<Ctrl>q"]), ("window.close", ["<Ctrl>w"]),
                               ("win.new-chat", ["<Ctrl>n"]), ("win.focus-entry", ["<Ctrl>l"]),
                               ("win.stop", ["Escape"]), ("win.show-help-overlay", ["<Ctrl>question"]),
                               ("win.preferences", ["<Ctrl>comma"])):
            self.set_accels_for_action(action, accels)

    def _present(self) -> Window:
        if self.window is None:
            self.window = Window(self)
        self.window.present()
        return self.window

    def do_activate(self):
        self._present()

    def do_command_line(self, cmdline):
        opts = cmdline.get_options_dict().end().unpack()
        if opts.get("version"):
            cmdline.print_literal(f"{APP_NAME} {VERSION}\n")
            return 0
        win = self._present()
        if opts.get("ask"):
            win.ask(opts["ask"])
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
        if self.window:
            self.window.client.stop_server()


def main(argv) -> int:
    return App().run(argv)
