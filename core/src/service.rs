//! `assistantd`: the agent as a D-Bus service on the session bus.
//!
//! Clients (the window, GNOME search) call methods; a turn runs on a worker
//! thread and streams back as signals tagged with its chat id. When an action
//! needs the user's OK, the service emits ConfirmRequest and waits for a
//! client to call Confirm.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gio::prelude::*;
use glib::{ControlFlow, Variant};
use serde_json::{Value, json};

use crate::agent::{Agent, Callbacks};
use crate::flm::{self, FlmClient, Model};
use crate::mcp::Hub;
use crate::search;
use crate::tool::Context;
use crate::{activity, paths, store, tools};

pub const BUS_NAME: &str = "io.github.wiertless.PandaAssistant.Core";
pub const OBJECT_PATH: &str = "/io/github/wiertless/PandaAssistant/Core";
pub const INTERFACE: &str = "io.github.wiertless.PandaAssistant.Core1";
/// A bigger model (hand-off, pictures) is swapped out this soon after use.
const BIG_MODEL_IDLE: Duration = Duration::from_secs(60);
/// With no model loaded and nothing to do, the service exits after this long
/// (D-Bus starts it again on the next call).
const EXIT_WHEN_IDLE: Duration = Duration::from_secs(600);
/// A question from GNOME search after this long starts a new chat.
const STALE_CHAT: Duration = Duration::from_secs(30 * 60);

pub const INTROSPECTION: &str = r#"
<node>
  <interface name="io.github.wiertless.PandaAssistant.Core1">
    <method name="Ask">
      <arg type="s" name="chat_id" direction="in"/>
      <arg type="s" name="text" direction="in"/>
      <arg type="as" name="images" direction="in"/>
      <arg type="s" name="chat_id" direction="out"/>
    </method>
    <method name="Stop"/>
    <method name="Confirm">
      <arg type="u" name="request" direction="in"/>
      <arg type="s" name="answer" direction="in"/>
    </method>
    <method name="Status"><arg type="s" name="json" direction="out"/></method>
    <method name="Models"><arg type="s" name="json" direction="out"/></method>
    <method name="Reload"/>
    <method name="Unload"/>
    <method name="Load"/>
    <method name="Restart"/>
    <method name="Undo"><arg type="s" name="result" direction="out"/></method>
    <method name="SnoozeReminder"><arg type="s" name="id" direction="in"/></method>
    <method name="DismissReminder"><arg type="s" name="id" direction="in"/></method>
    <method name="Connections"><arg type="s" name="json" direction="out"/></method>
    <method name="Pending">
      <arg type="s" name="chat_id" direction="in"/>
      <arg type="s" name="json" direction="out"/>
    </method>
    <signal name="Text"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="Reasoning"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="ToolStarted"><arg type="s" name="chat_id"/><arg type="s" name="name"/><arg type="s" name="summary"/></signal>
    <signal name="ToolFinished"><arg type="s" name="chat_id"/><arg type="s" name="name"/><arg type="s" name="result"/><arg type="b" name="ok"/></signal>
    <signal name="Notice"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="Handoff"><arg type="s" name="chat_id"/><arg type="s" name="model"/></signal>
    <signal name="Compacting"><arg type="s" name="chat_id"/></signal>
    <signal name="Retract"><arg type="s" name="chat_id"/></signal>
    <signal name="ConfirmRequest"><arg type="s" name="chat_id"/><arg type="u" name="request"/><arg type="s" name="name"/><arg type="s" name="summary"/><arg type="s" name="detail"/><arg type="b" name="can_always"/></signal>
    <signal name="Finished"><arg type="s" name="chat_id"/><arg type="s" name="error"/></signal>
    <signal name="StateChanged"><arg type="s" name="model"/><arg type="s" name="state"/></signal>
    <signal name="Clipboard"><arg type="s" name="text"/></signal>
    <signal name="StepAside"/>
    <signal name="StepBack"/>
  </interface>
</node>
"#;

/// Settings shared with the window (~/.config/panda-assistant/config.json).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub model: String,
    /// The user picked the model (else the default for what's installed).
    pub model_chosen: bool,
    pub ctx_len: u32,
    pub web_search: bool,
    pub memory: bool,
    pub smarter_model: String, // "auto", "off" or a model
    pub unload_after_minutes: u32,
    pub always_allow: HashSet<String>,
}

impl Config {
    pub fn load() -> Self {
        let raw: Value = std::fs::read_to_string(paths::config_dir().join("config.json"))
            .ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
        Self::from_json(&raw)
    }

    pub fn from_json(raw: &Value) -> Self {
        Self {
            model: raw["model"].as_str().unwrap_or(flm::DEFAULT_MODEL).to_string(),
            model_chosen: raw["model"].as_str().is_some(),
            ctx_len: raw["ctx_len"].as_u64().unwrap_or(flm::DEFAULT_CTX_LEN as u64) as u32,
            web_search: raw["web_search"].as_bool().unwrap_or(true),
            memory: raw["memory"].as_bool().unwrap_or(true),
            smarter_model: raw["smarter_model"].as_str().unwrap_or("auto").to_string(),
            unload_after_minutes: raw["unload_after_minutes"].as_u64().unwrap_or(0) as u32,
            always_allow: raw["always_allow"].as_array().into_iter().flatten()
                .filter_map(|v| v.as_str().map(String::from)).collect(),
        }
    }
}

/// Emits a turn's events as signals; blocks on ConfirmRequest until answered.
struct SignalCallbacks {
    conn: gio::DBusConnection,
    chat_id: String,
    service: Arc<Shared>,
    /// Asked from GNOME search with no window: tell the user by notification.
    background: bool,
    /// For the activity log: each tool's summary, and the last Allow/Cancel answer.
    summaries: Mutex<HashMap<String, String>>,
    decision: Mutex<Option<&'static str>>,
}

impl SignalCallbacks {
    fn emit(&self, name: &str, args: Variant) {
        let _ = self.conn.emit_signal(None, OBJECT_PATH, INTERFACE, name, Some(&args));
    }
}

impl Callbacks for SignalCallbacks {
    fn text(&self, t: &str) {
        self.emit("Text", (self.chat_id.as_str(), t).to_variant());
    }
    fn reasoning(&self, t: &str) {
        self.emit("Reasoning", (self.chat_id.as_str(), t).to_variant());
    }
    fn tool_started(&self, n: &str, s: &str) {
        self.summaries.lock().unwrap().insert(n.into(), s.into());
        self.emit("ToolStarted", (self.chat_id.as_str(), n, s).to_variant());
    }
    fn tool_finished(&self, n: &str, r: &str, ok: bool) {
        if n != crate::tools::groups::MORE_TOOLS {
            let summary = self.summaries.lock().unwrap().get(n).cloned().unwrap_or_else(|| n.replace('_', " "));
            activity::record(&activity::Entry { chat_id: &self.chat_id, tool: n, summary: &summary, result: r, ok,
                                                from: if self.background { "search" } else { "window" },
                                                decision: self.decision.lock().unwrap().take() });
        }
        self.emit("ToolFinished", (self.chat_id.as_str(), n, r, ok).to_variant());
    }
    fn notice(&self, t: &str) {
        self.emit("Notice", (self.chat_id.as_str(), t).to_variant());
    }
    fn handoff(&self, m: &str) {
        self.emit("Handoff", (self.chat_id.as_str(), m).to_variant());
    }
    fn compacting(&self) {
        self.emit("Compacting", (self.chat_id.as_str(),).to_variant());
    }
    fn retract(&self) {
        self.emit("Retract", (self.chat_id.as_str(),).to_variant());
    }
    fn confirm(&self, name: &str, summary: &str, detail: &str) -> bool {
        let can_always = !tools::ALWAYS_ASK.contains(&name);
        let (id, rx) = self.service.new_request(json!({"chat_id": self.chat_id, "name": name, "summary": summary,
                                                       "detail": detail, "can_always": can_always}));
        self.emit("ConfirmRequest", (self.chat_id.as_str(), id, name, summary, detail, can_always).to_variant());
        if self.background && !search::app_running(&self.conn) {
            search::notify(&self.conn, "Assistant needs your OK", &format!("{summary}?"), &self.chat_id, "Review");
        }
        let answer = loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(a) => break a,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !self.service.cancel.load(Ordering::SeqCst) => continue,
                Err(_) => break "deny".into(),
            }
        };
        self.service.pending.lock().unwrap().remove(&id);
        if answer == "always" && can_always {
            self.service.allow_always(name);
        }
        let allowed = answer == "allow" || answer == "always";
        *self.decision.lock().unwrap() = Some(if answer == "always" && can_always { "always" } else if allowed { "allowed" } else { "declined" });
        allowed
    }
}

/// State shared between the D-Bus handlers (main thread) and turns (workers).
pub struct Shared {
    pub client: Arc<FlmClient>,
    pub config: Mutex<Config>,
    pub models: Mutex<Vec<Model>>,
    pub agent: Mutex<Option<Agent>>,
    pub chat_id: Mutex<String>,
    pub busy: AtomicBool,
    pub cancel: Arc<AtomicBool>,
    pub always_allow: Arc<Mutex<HashSet<String>>>,
    next_request: AtomicU32,
    pending: Mutex<HashMap<u32, (Sender<String>, Value)>>,
    pub state: Mutex<String>,
    /// The question being answered right now (a window following the turn shows it).
    pub question: Mutex<String>,
    pub mcp: Arc<Hub>,
    /// What tools may ask of the window: clipboard, stepping aside.
    pub tool_ctx: Mutex<Context>,
    /// Routing by meaning, once the embedding model is known to be installed.
    pub embedder: Mutex<Option<Arc<tools::groups::Embedder>>>,
}

impl Shared {
    pub fn new(client: Arc<FlmClient>) -> Arc<Self> {
        let config = Config::load();
        Arc::new(Self {
            client,
            always_allow: Arc::new(Mutex::new(config.always_allow.clone())),
            config: Mutex::new(config),
            models: Mutex::new(vec![]),
            agent: Mutex::new(None),
            chat_id: Mutex::new(String::new()),
            busy: AtomicBool::new(false),
            cancel: Arc::new(AtomicBool::new(false)),
            next_request: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
            state: Mutex::new("Not loaded".into()),
            question: Mutex::new(String::new()),
            mcp: Hub::new(),
            tool_ctx: Mutex::new(Context::default()),
            embedder: Mutex::new(None),
        })
    }

    fn new_request(&self, info: Value) -> (u32, Receiver<String>) {
        let id = self.next_request.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, (tx, info));
        (id, rx)
    }

    pub fn answer(&self, id: u32, answer: &str) -> bool {
        match self.pending.lock().unwrap().get(&id) {
            Some((tx, _)) => tx.send(answer.to_string()).is_ok(),
            None => false,
        }
    }

    /// Questions waiting for the user in this chat (a window opening it shows them).
    pub fn pending_for(&self, chat_id: &str) -> Value {
        let list: Vec<Value> = self.pending.lock().unwrap().iter()
            .filter(|(_, (_, info))| info["chat_id"] == chat_id)
            .map(|(id, (_, info))| { let mut i = info.clone(); i["request"] = json!(id); i })
            .collect();
        Value::Array(list)
    }

    fn allow_always(&self, name: &str) {
        self.always_allow.lock().unwrap().insert(name.to_string());
        // Keep the window's settings file in step.
        let path = paths::config_dir().join("config.json");
        let mut raw: Value = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(json!({}));
        let mut names: Vec<String> = self.always_allow.lock().unwrap().iter().cloned().collect();
        names.sort();
        raw["always_allow"] = json!(names);
        let _ = store::write_atomic(&path, &serde_json::to_string_pretty(&raw).unwrap_or_default());
    }

    fn model_sees(&self, name: &str) -> bool {
        self.models.lock().unwrap().iter().any(|m| m.name == name && m.can_see())
    }

    /// A fresh agent with the current settings.
    fn make_agent(self: &Arc<Self>) -> Agent {
        let cfg = self.config.lock().unwrap().clone();
        let models = self.models.lock().unwrap().clone();
        let hub = self.mcp.clone();
        let mut agent = Agent::new(self.client.clone(), &cfg.model, Arc::new(move || {
            let mut all = tools::all();
            all.extend(hub.tools());
            all
        }));
        agent.ctx = self.tool_ctx.lock().unwrap().clone();
        agent.cancel = self.cancel.clone();
        agent.web_enabled = cfg.web_search;
        agent.memory_enabled = cfg.memory;
        agent.ctx_tokens = cfg.ctx_len as usize;
        agent.always_allow = self.always_allow.clone();
        agent.embedder = self.embedder.lock().unwrap().clone();
        agent.machine_source = Arc::new(crate::context::system_context);
        let me = Arc::downgrade(self);
        agent.sees = Arc::new(move |m| me.upgrade().is_some_and(|s| s.model_sees(m)));
        agent.vision_model = flm::pick_vision_model(&models, &cfg.model, None);
        agent.smarter_model = match cfg.smarter_model.as_str() {
            "off" => None,
            "auto" => flm::pick_smarter_model(&models, &cfg.model, None),
            other => Some(other.to_string()),
        };
        if let Some(m) = models.iter().find(|m| m.name == cfg.model).filter(|m| !m.labels.is_empty()) {
            agent.use_tools = m.can_use_tools();
        }
        agent
    }

    /// The agent for this chat: the current one, a saved chat, or a new chat.
    fn agent_for(self: &Arc<Self>, chat_id: &str) -> (String, Agent) {
        let current = self.chat_id.lock().unwrap().clone();
        if !chat_id.is_empty() && chat_id == current
            && let Some(a) = self.agent.lock().unwrap().take() {
                return (current, a);
            }
        let mut agent = self.make_agent();
        if !chat_id.is_empty()
            && let Some(data) = store::load_chat(chat_id) {
                agent.messages = data["messages"].as_array().cloned().unwrap_or_default();
                agent.summary = data["summary"].as_str().unwrap_or("").to_string();
                agent.compacted = (data["compacted"].as_u64().unwrap_or(0) as usize).min(agent.messages.len());
                return (chat_id.to_string(), agent);
            }
        // A new chat; a client may choose its id (so it can route the signals).
        let id = if !chat_id.is_empty() && chat_id.len() <= 64
            && chat_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            chat_id.to_string()
        } else {
            store::new_chat_id()
        };
        (id, agent)
    }

    fn big_model_loaded(&self) -> bool {
        let model = self.config.lock().unwrap().model.clone();
        self.client.owns_server() && self.client.last_model.lock().unwrap().as_deref().is_some_and(|m| m != model)
    }
}

pub struct Service {
    shared: Arc<Shared>,
    conn: gio::DBusConnection,
    idle: Mutex<Option<glib::SourceId>>,
    last_activity: Mutex<std::time::Instant>,
    loading: AtomicBool,
    /// When the last answer finished (a question from search after a long
    /// pause starts a new chat).
    last_turn: Mutex<Option<std::time::Instant>>,
    reminder_timer: Mutex<Option<glib::SourceId>>,
}

impl Service {
    pub fn new(conn: gio::DBusConnection, client: Arc<FlmClient>) -> Arc<Self> {
        let shared = Shared::new(client);
        let emit = |conn: &gio::DBusConnection, name: &'static str, args: Option<Variant>| {
            let c = conn.clone();
            move || { let _ = c.emit_signal(None, OBJECT_PATH, INTERFACE, name, args.as_ref()); }
        };
        {
            let mut ctx = shared.tool_ctx.lock().unwrap();
            let c = conn.clone();
            ctx.copy_to_clipboard = Some(Arc::new(move |text: &str| {
                let _ = c.emit_signal(None, OBJECT_PATH, INTERFACE, "Clipboard", Some(&(text,).to_variant()));
            }));
            ctx.hide_window = Some(Arc::new(emit(&conn, "StepAside", None)));
            ctx.show_window = Some(Arc::new(emit(&conn, "StepBack", None)));
        }
        Arc::new(Self { shared, conn, idle: Mutex::new(None), last_activity: Mutex::new(std::time::Instant::now()),
                        loading: AtomicBool::new(false), last_turn: Mutex::new(None), reminder_timer: Mutex::new(None) })
    }

    /// Fire what's due and wake up for the next one. At most a minute
    /// between checks: the monotonic clock stops during suspend, so a long
    /// timeout would fire late after waking up.
    pub fn schedule_reminders(self: &Arc<Self>) {
        if let Some(id) = self.reminder_timer.lock().unwrap().take() {
            id.remove();
        }
        for r in crate::reminders::take_due() {
            search::notify_reminder(&self.conn, &r);
            self.touch();
        }
        let Some(next) = crate::reminders::pending().first().map(|r| r.due) else { return };
        let wait = (next - crate::reminders::now()).clamp(1, 60) as u64;
        let me = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_secs(wait), move || {
            me.reminder_timer.lock().unwrap().take();
            me.schedule_reminders();
        });
        *self.reminder_timer.lock().unwrap() = Some(id);
    }

    /// Tools that add or remove reminders tell the main loop to reschedule.
    pub fn watch_reminders(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.shared.tool_ctx.lock().unwrap().reminders_changed = Some(Arc::new(move || {
            let w = weak.clone();
            glib::MainContext::default().invoke(move || if let Some(s) = w.upgrade() { s.schedule_reminders() });
        }));
        self.schedule_reminders();
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    fn set_state(&self, state: &str) {
        *self.shared.state.lock().unwrap() = state.to_string();
        let model = self.shared.config.lock().unwrap().model.clone();
        let _ = self.conn.emit_signal(None, OBJECT_PATH, INTERFACE, "StateChanged",
                                      Some(&(model.as_str(), state).to_variant()));
    }

    /// Load the default model ahead of a question (e.g. while it's typed).
    pub fn preload(self: &Arc<Self>) {
        if self.shared.busy.load(Ordering::SeqCst) || self.loading.swap(true, Ordering::SeqCst) {
            return;
        }
        self.cancel_idle();
        let me = self.clone();
        std::thread::spawn(move || {
            let cfg = me.shared.config.lock().unwrap().clone();
            if *me.shared.state.lock().unwrap() != "Loaded" {
                me.set_state("Loading…");
            }
            let ok = me.shared.client.ensure_server(&cfg.model, Some(cfg.ctx_len), Duration::from_secs(300)).is_ok();
            me.set_state(if ok { "Loaded" } else { "Not loaded" });
            me.loading.store(false, Ordering::SeqCst);
            let me2 = me.clone();
            glib::MainContext::default().invoke(move || me2.schedule_idle());
        });
    }

    /// The MCP servers and how they're doing, for the window's Connections page.
    pub fn connections(&self) -> Value {
        let servers: Vec<Value> = self.shared.mcp.servers.lock().unwrap().values().map(|s| {
            let s = s.lock().unwrap();
            let target = if s.cfg.url.is_empty() { std::iter::once(s.cfg.command.clone()).chain(s.cfg.args.clone()).collect::<Vec<_>>().join(" ") } else { s.cfg.url.clone() };
            json!({"name": s.cfg.name, "label": s.label, "running": s.running(), "tools": s.tools.len(), "error": s.error,
                   "disabled": s.cfg.disabled, "trust": s.cfg.trust, "target": target})
        }).collect();
        json!({"servers": servers, "config_error": *self.shared.mcp.config_error.lock().unwrap(), "max_tools": crate::mcp::MAX_TOOLS})
    }

    pub fn touch(&self) {
        *self.last_activity.lock().unwrap() = std::time::Instant::now();
    }

    /// True when the service has nothing loaded and nothing to do for a while.
    fn should_exit(&self) -> bool {
        !self.shared.busy.load(Ordering::SeqCst) && !self.loading.load(Ordering::SeqCst) && !self.shared.client.owns_server()
            && self.last_activity.lock().unwrap().elapsed() > EXIT_WHEN_IDLE
            && crate::reminders::pending().is_empty()
    }

    pub fn load_models(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            if let Ok(models) = me.shared.client.list_models() {
                let installed: Vec<Model> = models.into_iter().filter(|m| m.installed).collect();
                // The embedding model, when installed, runs next to the chat model for routing.
                if installed.iter().any(|m| m.name == flm::EMBED_MODEL) {
                    *me.shared.client.embed_model.lock().unwrap() = Some(flm::EMBED_MODEL.into());
                    me.shared.embedder.lock().unwrap().get_or_insert_with(|| {
                        Arc::new(tools::groups::Embedder::new(me.shared.client.clone(), flm::EMBED_MODEL))
                    });
                }
                // No choice yet: the best preferred model that's installed.
                let mut cfg = me.shared.config.lock().unwrap();
                if !cfg.model_chosen {
                    cfg.model = flm::default_model(&installed).to_string();
                }
                drop(cfg);
                *me.shared.models.lock().unwrap() = installed;
            }
        });
    }

    pub fn status(&self) -> Value {
        let cfg = self.shared.config.lock().unwrap();
        json!({"model": cfg.model, "state": *self.shared.state.lock().unwrap(),
               "busy": self.shared.busy.load(Ordering::SeqCst), "chat_id": *self.shared.chat_id.lock().unwrap(),
               "question": *self.shared.question.lock().unwrap(),
               "version": env!("CARGO_PKG_VERSION")})
    }

    /// A question from GNOME search with no window open: answer it here and
    /// say so by notification. After 30 quiet minutes it starts a new chat.
    pub fn ask_in_background(self: &Arc<Self>, text: &str) {
        if self.shared.busy.load(Ordering::SeqCst) {
            let id = self.shared.chat_id.lock().unwrap().clone();
            search::notify(&self.conn, "Assistant is busy", "Still working on the last question.", &id, "Open Chat");
            return;
        }
        let fresh = self.last_turn.lock().unwrap().is_some_and(|t| t.elapsed() < STALE_CHAT);
        let current = self.shared.chat_id.lock().unwrap().clone();
        let chat_id = if fresh && !current.is_empty() { current } else { store::new_chat_id() };
        // "/name details" runs a saved skill, as in the window.
        let prompt = match text.strip_prefix('/') {
            Some(rest) => {
                let (name, details) = rest.split_once(' ').unwrap_or((rest, ""));
                store::find_skill(name).map(|s| store::skill_prompt(&s, details)).unwrap_or_else(|| text.to_string())
            }
            None => text.to_string(),
        };
        search::notify(&self.conn, "Asking Assistant…", text, &chat_id, "Open Chat");
        if let Err(e) = self.start_turn(&chat_id, &prompt, vec![], true) {
            search::notify(&self.conn, "Assistant couldn't answer", &e, &chat_id, "Open Chat");
        }
    }

    /// Start a turn on a worker thread; returns its chat id at once.
    pub fn ask(self: &Arc<Self>, chat_id: &str, text: &str, images: Vec<String>) -> Result<String, String> {
        self.start_turn(chat_id, text, images, false)
    }

    fn start_turn(self: &Arc<Self>, chat_id: &str, text: &str, images: Vec<String>, background: bool) -> Result<String, String> {
        if self.shared.busy.swap(true, Ordering::SeqCst) {
            return Err("busy with another question".into());
        }
        self.cancel_idle();
        let (chat_id, mut agent) = self.shared.agent_for(chat_id);
        *self.shared.chat_id.lock().unwrap() = chat_id.clone();
        self.shared.cancel.store(false, Ordering::SeqCst);
        *self.shared.question.lock().unwrap() = text.to_string();
        let me = self.clone();
        let text = text.to_string();
        let id = chat_id.clone();
        std::thread::spawn(move || {
            let shared = me.shared.clone();
            let cfg = shared.config.lock().unwrap().clone();
            let cb = SignalCallbacks { conn: me.conn.clone(), chat_id: id.clone(), service: shared.clone(), background,
                                       summaries: Mutex::default(), decision: Mutex::default() };
            if *shared.state.lock().unwrap() != "Loaded" {
                me.set_state("Loading…");
            }
            let before = (agent.messages.len(), agent.summary.clone(), agent.compacted);
            let attempt = |agent: &mut Agent| shared.client.ensure_server(&agent.model, Some(cfg.ctx_len), Duration::from_secs(300))
                .and_then(|_| {
                    me.set_state("Loaded");
                    if agent.use_tools {
                        for problem in shared.mcp.start_all() {
                            cb.notice(&format!("Couldn't connect {problem}"));
                        }
                    }
                    agent.ask(&text, &cb, &images)
                });
            let mut result = attempt(&mut agent);
            // FastFlowLM itself broke (not the request): stop every flm serve, start it
            // again and ask once more, from where the turn began.
            if let Err(e) = &result && flm::server_trouble(e) && !shared.cancel.load(Ordering::SeqCst) {
                cb.retract();
                cb.notice("FastFlowLM stopped working, so it's being restarted…");
                me.set_state("Loading…");
                agent.messages.truncate(before.0);
                (agent.summary, agent.compacted) = (before.1.clone(), before.2);
                result = shared.client.restart_server(&agent.model, Some(cfg.ctx_len)).and_then(|_| attempt(&mut agent));
            }
            let result = result.map_err(|mut e| {
                if flm::server_trouble(&e) {
                    e.message += &shared.client.diagnose();
                }
                e
            });
            let error = result.err().map(|e| e.message).unwrap_or_default();
            if !error.is_empty() && !shared.client.is_up() {
                me.set_state("Not loaded");
            }
            let _ = store::save_chat(&id, &agent.messages, &agent.model, &agent.summary, agent.compacted);
            let answer = agent.messages.last().filter(|m| m["role"] == "assistant")
                .and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
            *shared.agent.lock().unwrap() = Some(agent);
            shared.busy.store(false, Ordering::SeqCst);
            *me.last_turn.lock().unwrap() = Some(std::time::Instant::now());
            me.touch();
            if background && !search::app_running(&me.conn) {
                if !error.is_empty() {
                    search::notify(&me.conn, "Assistant couldn't answer", &error, &id, "Open Chat");
                } else if !answer.trim().is_empty() {
                    search::notify(&me.conn, "Assistant", &search::plain(&answer, 400), &id, "Open Chat");
                }
            }
            cb.emit("Finished", (id.as_str(), error.as_str()).to_variant());
            let me2 = me.clone();
            glib::MainContext::default().invoke(move || me2.schedule_idle());
        });
        Ok(chat_id)
    }

    pub fn stop(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
    }

    pub fn reload(self: &Arc<Self>) {
        self.shared.mcp.reload();
        let cfg = Config::load();
        *self.shared.always_allow.lock().unwrap() = cfg.always_allow.clone();
        let changed_ctx = self.shared.config.lock().unwrap().ctx_len != cfg.ctx_len;
        *self.shared.config.lock().unwrap() = cfg;
        // The next question starts from the new settings (chat history is kept on disk).
        if !self.shared.busy.load(Ordering::SeqCst) {
            *self.shared.agent.lock().unwrap() = None;
            if changed_ctx {
                self.unload();
            }
        }
        self.load_models();
    }

    /// Start over: stop the turn, the model server and connected apps, then
    /// exit; D-Bus starts a fresh service on the next call.
    pub fn restart(self: &Arc<Self>) {
        self.stop();
        self.cancel_idle();
        let me = self.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if !me.shared.busy.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            me.shared.mcp.stop_all();
            // Every flm serve, not just ours: a stale one is usually why a restart was needed.
            me.shared.client.stop_all_servers();
            me.set_state("Not loaded");
            let _ = me.conn.flush_sync(None::<&gio::Cancellable>);
            std::process::exit(0);
        });
    }

    pub fn unload(self: &Arc<Self>) {
        if self.shared.busy.load(Ordering::SeqCst) {
            return;
        }
        self.cancel_idle();
        let me = self.clone();
        std::thread::spawn(move || {
            me.shared.mcp.stop_all();
            me.shared.client.stop_server();
            me.set_state("Not loaded");
        });
    }

    fn cancel_idle(&self) {
        if let Some(id) = self.idle.lock().unwrap().take() {
            id.remove();
        }
    }

    /// Big models go a minute after use; the default one after its timer, if any.
    fn schedule_idle(self: &Arc<Self>) {
        self.cancel_idle();
        if self.shared.busy.load(Ordering::SeqCst) {
            return;
        }
        let minutes = self.shared.config.lock().unwrap().unload_after_minutes;
        let delay = if self.shared.big_model_loaded() {
            BIG_MODEL_IDLE
        } else if minutes > 0 && self.shared.client.owns_server() {
            Duration::from_secs(minutes as u64 * 60)
        } else {
            return;
        };
        let me = self.clone();
        let id = glib::timeout_add_local_once(delay, move || {
            me.idle.lock().unwrap().take();
            me.on_idle();
        });
        *self.idle.lock().unwrap() = Some(id);
    }

    fn on_idle(self: &Arc<Self>) {
        if self.shared.busy.load(Ordering::SeqCst) {
            return;
        }
        if self.shared.big_model_loaded() {
            // Restart with just the default model: frees the bigger one and its context.
            let me = self.clone();
            std::thread::spawn(move || {
                me.set_state("Loading…");
                me.shared.client.stop_server();
                let cfg = me.shared.config.lock().unwrap().clone();
                let ok = me.shared.client.ensure_server(&cfg.model, Some(cfg.ctx_len), Duration::from_secs(300)).is_ok();
                me.set_state(if ok { "Loaded" } else { "Not loaded" });
                let me2 = me.clone();
                glib::MainContext::default().invoke(move || me2.schedule_idle());
            });
        } else {
            self.unload();
        }
    }

    fn handle(self: &Arc<Self>, method: &str, params: &Variant, invocation: gio::DBusMethodInvocation) {
        self.touch();
        let reply = |v: Option<Variant>| invocation.clone().return_value(v.as_ref());
        match method {
            "Ask" => {
                let (chat_id, text, images): (String, String, Vec<String>) = params.get().unwrap_or_default();
                // Remember who asked: reading "the window" skips the asking window.
                if let Some(sender) = invocation.sender() {
                    let pid = self.conn.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus",
                        "GetConnectionUnixProcessID", Some(&(sender.as_str(),).to_variant()),
                        Some(glib::VariantTy::new("(u)").unwrap()), gio::DBusCallFlags::NONE, 1000, None::<&gio::Cancellable>)
                        .ok().and_then(|v| v.child_value(0).get::<u32>()).unwrap_or(0);
                    self.shared.tool_ctx.lock().unwrap().client_pid.store(pid, Ordering::SeqCst);
                }
                match self.ask(&chat_id, &text, images) {
                    Ok(id) => reply(Some((id,).to_variant())),
                    Err(e) => invocation.return_dbus_error("io.github.wiertless.PandaAssistant.Error.Busy", &e),
                }
            }
            "Stop" => {
                self.stop();
                reply(None)
            }
            "Confirm" => {
                let (id, answer): (u32, String) = params.get().unwrap_or_default();
                self.shared.answer(id, &answer);
                reply(None)
            }
            "Status" => reply(Some((self.status().to_string(),).to_variant())),
            "Models" => {
                // The list loads in the background at start; wait for it if needed.
                let shared = self.shared.clone();
                glib::MainContext::default().spawn_local(async move {
                    let list = gio::spawn_blocking(move || {
                        if shared.models.lock().unwrap().is_empty()
                            && let Ok(models) = shared.client.list_models() {
                                *shared.models.lock().unwrap() = models.into_iter().filter(|m| m.installed).collect();
                            }
                        let models: Vec<Value> = shared.models.lock().unwrap().iter()
                            .map(|m| json!({"name": m.name, "labels": m.labels})).collect();
                        Value::Array(models).to_string()
                    }).await.unwrap_or_else(|_| "[]".into());
                    invocation.return_value(Some(&(list,).to_variant()));
                });
            }
            "Load" => {
                self.preload();
                reply(None)
            }
            "Connections" => reply(Some((self.connections().to_string(),).to_variant())),
            "Pending" => {
                let (chat_id,): (String,) = params.get().unwrap_or_default();
                reply(Some((self.shared.pending_for(&chat_id).to_string(),).to_variant()))
            }
            "Reload" => {
                self.reload();
                reply(None)
            }
            "Unload" => {
                self.unload();
                reply(None)
            }
            "Undo" => {
                let ctx = self.shared.tool_ctx.lock().unwrap().clone();
                let out = ctx.undo_last().unwrap_or_else(|e| e.0);
                reply(Some((out,).to_variant()))
            }
            "SnoozeReminder" | "DismissReminder" => {
                let (id,): (String,) = params.get().unwrap_or_default();
                if method == "SnoozeReminder" {
                    crate::reminders::snooze(&id, crate::reminders::SNOOZE_MINUTES);
                } else {
                    crate::reminders::remove(&id);
                }
                self.schedule_reminders();
                reply(None)
            }
            "Restart" => {
                reply(None);
                self.restart();
            }
            _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        }
    }

    /// Export the object; call once on the main thread.
    pub fn register(self: &Arc<Self>) -> Result<gio::RegistrationId, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(INTROSPECTION)?;
        let iface = node.lookup_interface(INTERFACE).expect("interface in XML");
        let me = self.clone();
        self.conn
            .register_object(OBJECT_PATH, &iface)
            .method_call(move |_conn, _sender, _path, _iface, method, params, invocation| {
                me.handle(method, &params, invocation);
            })
            .build()
    }
}

/// Run the service until it loses its bus name.
pub fn run() -> ControlFlow {
    let conn = match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("assistantd: no session bus: {e}");
            return ControlFlow::Break;
        }
    };
    let service = Service::new(conn.clone(), Arc::new(FlmClient::from_env()));
    // Export the objects first, then take the names: a client D-Bus started
    // us for may call the moment the name is ours.
    if let Err(e) = service.register() {
        eprintln!("assistantd: can't export the service: {e}");
        return ControlFlow::Break;
    }
    service.watch_reminders();
    let search = search::register(&service, &conn);
    if let Err(e) = &search {
        eprintln!("assistantd: no GNOME search: {e}");
    }
    // One service per session: if another one has the name, leave.
    if request_name(&conn, BUS_NAME) != Some(1) {
        return ControlFlow::Continue;
    }
    if search.is_ok() {
        request_name(&conn, search::BUS_NAME);
    }
    service.load_models();
    let main_loop = glib::MainLoop::new(None, false);
    let (s, l) = (service.clone(), main_loop.clone());
    glib::timeout_add_seconds_local(60, move || {
        if s.should_exit() {
            l.quit();
            return ControlFlow::Break;
        }
        ControlFlow::Continue
    });
    // Losing the bus closes the connection, which ends the loop below.
    let quit = main_loop.clone();
    conn.connect_closed(move |_, _, _| quit.quit());
    main_loop.run();
    service.shared().mcp.stop_all();
    service.shared().client.stop_server();
    ControlFlow::Continue
}

/// Ask the bus for a name without queueing: 1 means we have it.
fn request_name(conn: &gio::DBusConnection, name: &str) -> Option<u32> {
    const DO_NOT_QUEUE: u32 = 4;
    conn.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
                   Some(&(name, DO_NOT_QUEUE).to_variant()), Some(glib::VariantTy::new("(u)").unwrap()),
                   gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>)
        .ok().and_then(|v| v.child_value(0).get::<u32>())
}
