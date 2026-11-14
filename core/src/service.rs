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
use crate::tool::Context;
use crate::{paths, store, tools};

pub const BUS_NAME: &str = "io.github.wiertless.PandaAssistant.Core";
pub const OBJECT_PATH: &str = "/io/github/wiertless/PandaAssistant/Core";
pub const INTERFACE: &str = "io.github.wiertless.PandaAssistant.Core1";
/// A bigger model (hand-off, pictures) is swapped out this soon after use.
const BIG_MODEL_IDLE: Duration = Duration::from_secs(60);

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
    <signal name="Text"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="Reasoning"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="ToolStarted"><arg type="s" name="chat_id"/><arg type="s" name="name"/><arg type="s" name="summary"/></signal>
    <signal name="ToolFinished"><arg type="s" name="chat_id"/><arg type="s" name="name"/><arg type="s" name="result"/><arg type="b" name="ok"/></signal>
    <signal name="Notice"><arg type="s" name="chat_id"/><arg type="s" name="text"/></signal>
    <signal name="Handoff"><arg type="s" name="chat_id"/><arg type="s" name="model"/></signal>
    <signal name="Compacting"><arg type="s" name="chat_id"/></signal>
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
            model: raw["model"].as_str().unwrap_or("gemma4-it:e2b").to_string(),
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
        self.emit("ToolStarted", (self.chat_id.as_str(), n, s).to_variant());
    }
    fn tool_finished(&self, n: &str, r: &str, ok: bool) {
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
    fn confirm(&self, name: &str, summary: &str, detail: &str) -> bool {
        let (id, rx) = self.service.new_request();
        let can_always = !tools::ALWAYS_ASK.contains(&name);
        self.emit("ConfirmRequest", (self.chat_id.as_str(), id, name, summary, detail, can_always).to_variant());
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
        answer == "allow" || answer == "always"
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
    pending: Mutex<HashMap<u32, Sender<String>>>,
    pub state: Mutex<String>,
    pub mcp: Arc<Hub>,
    /// What tools may ask of the window: clipboard, stepping aside.
    pub tool_ctx: Mutex<Context>,
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
            mcp: Hub::new(),
            tool_ctx: Mutex::new(Context::default()),
        })
    }

    fn new_request(&self) -> (u32, Receiver<String>) {
        let id = self.next_request.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);
        (id, rx)
    }

    pub fn answer(&self, id: u32, answer: &str) -> bool {
        match self.pending.lock().unwrap().get(&id) {
            Some(tx) => tx.send(answer.to_string()).is_ok(),
            None => false,
        }
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
        (store::new_chat_id(), agent)
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
        Arc::new(Self { shared, conn, idle: Mutex::new(None) })
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

    pub fn load_models(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            if let Ok(models) = me.shared.client.list_models() {
                *me.shared.models.lock().unwrap() = models.into_iter().filter(|m| m.installed).collect();
            }
        });
    }

    pub fn status(&self) -> Value {
        let cfg = self.shared.config.lock().unwrap();
        json!({"model": cfg.model, "state": *self.shared.state.lock().unwrap(),
               "busy": self.shared.busy.load(Ordering::SeqCst), "chat_id": *self.shared.chat_id.lock().unwrap(),
               "version": env!("CARGO_PKG_VERSION")})
    }

    /// Start a turn on a worker thread; returns its chat id at once.
    pub fn ask(self: &Arc<Self>, chat_id: &str, text: &str, images: Vec<String>) -> Result<String, String> {
        if self.shared.busy.swap(true, Ordering::SeqCst) {
            return Err("busy with another question".into());
        }
        self.cancel_idle();
        let (chat_id, mut agent) = self.shared.agent_for(chat_id);
        *self.shared.chat_id.lock().unwrap() = chat_id.clone();
        self.shared.cancel.store(false, Ordering::SeqCst);
        let me = self.clone();
        let text = text.to_string();
        let id = chat_id.clone();
        std::thread::spawn(move || {
            let shared = me.shared.clone();
            let cfg = shared.config.lock().unwrap().clone();
            let cb = SignalCallbacks { conn: me.conn.clone(), chat_id: id.clone(), service: shared.clone() };
            if *shared.state.lock().unwrap() != "Loaded" {
                me.set_state("Loading…");
            }
            let result = shared.client.ensure_server(&agent.model, Some(cfg.ctx_len), Duration::from_secs(300))
                .and_then(|_| {
                    me.set_state("Loaded");
                    if agent.use_tools {
                        for problem in shared.mcp.start_all() {
                            cb.notice(&format!("Couldn't connect {problem}"));
                        }
                    }
                    agent.ask(&text, &cb, &images)
                });
            let error = result.err().map(|e| e.message).unwrap_or_default();
            if !error.is_empty() && !shared.client.is_up() {
                me.set_state("Not loaded");
            }
            let _ = store::save_chat(&id, &agent.messages, &agent.model, &agent.summary, agent.compacted);
            *shared.agent.lock().unwrap() = Some(agent);
            shared.busy.store(false, Ordering::SeqCst);
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
        let reply = |v: Option<Variant>| invocation.clone().return_value(v.as_ref());
        match method {
            "Ask" => {
                let (chat_id, text, images): (String, String, Vec<String>) = params.get().unwrap_or_default();
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
                let models: Vec<Value> = self.shared.models.lock().unwrap().iter()
                    .map(|m| json!({"name": m.name, "labels": m.labels})).collect();
                reply(Some((Value::Array(models).to_string(),).to_variant()))
            }
            "Reload" => {
                self.reload();
                reply(None)
            }
            "Unload" => {
                self.unload();
                reply(None)
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
    if let Err(e) = service.register() {
        eprintln!("assistantd: can't export the service: {e}");
        return ControlFlow::Break;
    }
    service.load_models();
    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    let _owner = gio::bus_own_name_on_connection(&conn, BUS_NAME, gio::BusNameOwnerFlags::NONE, |_, _| {},
                                                 move |_, _| quit.quit());
    main_loop.run();
    service.shared().mcp.stop_all();
    service.shared().client.stop_server();
    ControlFlow::Continue
}
