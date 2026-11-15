//! The window's link to assistantd, which runs the model, the tools, memory,
//! skills and chats. D-Bus starts the service on the first call. A turn
//! streams back as signals tagged with its chat id, which the window chooses,
//! so it can route them before the call returns.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gio::prelude::*;
use glib::Variant;
use serde_json::Value;

const BUS_NAME: &str = "io.github.wiertless.PandaAssistant.Core";
const OBJECT_PATH: &str = "/io/github/wiertless/PandaAssistant/Core";
const INTERFACE: &str = "io.github.wiertless.PandaAssistant.Core1";

/// What the window does with a turn's events (on the GTK thread).
pub trait Turn {
    fn on_text(&self, text: &str);
    fn on_reasoning(&self, text: &str);
    fn on_tool_started(&self, name: &str, summary: &str);
    fn on_tool_finished(&self, name: &str, result: &str, ok: bool);
    fn on_notice(&self, text: &str);
    fn on_handoff(&self, model: &str);
    fn on_compacting(&self);
    fn on_confirm(&self, request: u32, name: &str, summary: &str, detail: &str, can_always: bool);
    fn on_finished(&self, error: &str);
}

#[derive(Default)]
pub struct Hooks {
    pub state: Option<Box<dyn Fn(&str, &str)>>,
    pub clipboard: Option<Box<dyn Fn(&str)>>,
    pub step_aside: Option<Box<dyn Fn()>>,
    pub step_back: Option<Box<dyn Fn()>>,
}

pub struct Core {
    bus: gio::DBusConnection,
    turns: RefCell<HashMap<String, Rc<dyn Turn>>>,
    pub hooks: RefCell<Hooks>,
}

fn friendly(e: &glib::Error) -> String {
    let remote = gio::DBusError::remote_error(e).map(|s| s.to_string()).unwrap_or_default();
    if remote.contains("ServiceUnknown") || remote.contains("NameHasNoOwner") {
        return "The assistant service (assistantd) isn't installed.".into();
    }
    if remote.contains("Error.Busy") {
        return "Still busy with the last question.".into();
    }
    let mut e = e.clone();
    gio::DBusError::strip_remote_error(&mut e);
    e.message().to_string()
}

impl Core {
    pub fn new() -> Rc<Self> {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).expect("session bus");
        let core = Rc::new(Self { bus: bus.clone(), turns: RefCell::default(), hooks: RefCell::default() });
        let weak = Rc::downgrade(&core);
        #[allow(deprecated)] // the callback form is the simplest for a whole interface
        bus.signal_subscribe(Some(BUS_NAME), Some(INTERFACE), None, Some(OBJECT_PATH), None, gio::DBusSignalFlags::NONE,
                             move |_, _, _, _, name, params| {
                                 if let Some(core) = weak.upgrade() {
                                     core.on_signal(name, params);
                                 }
                             });
        core
    }

    fn call(&self, method: &str, args: Option<&Variant>, reply: Option<&str>) -> Result<Option<Variant>, String> {
        self.bus.call_sync(Some(BUS_NAME), OBJECT_PATH, INTERFACE, method, args,
                           reply.map(|r| glib::VariantTy::new(r).unwrap()), gio::DBusCallFlags::NONE, 25000,
                           None::<&gio::Cancellable>)
            .map(Some).map_err(|e| friendly(&e))
    }

    fn call_async(&self, method: &str, args: Option<&Variant>) {
        self.bus.call(Some(BUS_NAME), OBJECT_PATH, INTERFACE, method, args, None, gio::DBusCallFlags::NONE, 60000,
                      None::<&gio::Cancellable>, |_| {});
    }

    fn json(&self, method: &str, args: Option<&Variant>) -> Result<Value, String> {
        let v = self.call(method, args, Some("(s)"))?.unwrap();
        Ok(serde_json::from_str(&v.child_value(0).get::<String>().unwrap_or_default()).unwrap_or(Value::Null))
    }

    pub fn ask(&self, chat_id: &str, text: &str, images: &[String], turn: Rc<dyn Turn>) -> Result<(), String> {
        self.turns.borrow_mut().insert(chat_id.into(), turn);
        let args = (chat_id, text, images.to_vec()).to_variant();
        self.call("Ask", Some(&args), Some("(s)")).map(|_| ()).inspect_err(|_| {
            self.turns.borrow_mut().remove(chat_id);
        })
    }

    /// Follow a turn the service is already running (e.g. one started from search).
    pub fn watch(&self, chat_id: &str, turn: Rc<dyn Turn>) {
        self.turns.borrow_mut().insert(chat_id.into(), turn);
    }

    pub fn confirm(&self, request: u32, answer: &str) {
        self.call_async("Confirm", Some(&(request, answer).to_variant()));
    }
    pub fn stop(&self) {
        self.call_async("Stop", None);
    }
    pub fn load(&self) {
        self.call_async("Load", None);
    }
    pub fn unload(&self) {
        self.call_async("Unload", None);
    }
    pub fn restart(&self) {
        self.call_async("Restart", None);
    }
    /// End turns whose service went away without finishing them.
    pub fn finish_all(&self, error: &str) {
        let turns: Vec<Rc<dyn Turn>> = self.turns.borrow_mut().drain().map(|(_, t)| t).collect();
        for t in turns {
            t.on_finished(error);
        }
    }
    /// The service process currently answering (its unique bus name), if any.
    pub fn owner(&self) -> Option<String> {
        self.bus.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner",
                           Some(&(BUS_NAME,).to_variant()), Some(glib::VariantTy::new("(s)").unwrap()),
                           gio::DBusCallFlags::NONE, 2000, None::<&gio::Cancellable>)
            .ok().and_then(|v| v.child_value(0).get::<String>())
    }
    pub fn reload(&self) {
        self.call_async("Reload", None);
    }
    pub fn reload_now(&self) -> Result<(), String> {
        self.call("Reload", None, None).map(|_| ())
    }
    pub fn status(&self) -> Result<Value, String> {
        self.json("Status", None)
    }
    pub fn pending(&self, chat_id: &str) -> Result<Value, String> {
        self.json("Pending", Some(&(chat_id,).to_variant()))
    }
    pub fn connections(&self) -> Result<Value, String> {
        self.json("Connections", None)
    }

    /// The installed models (the service may need a moment to list them).
    pub fn models(&self, done: impl FnOnce(Result<Value, String>) + 'static) {
        self.bus.call(Some(BUS_NAME), OBJECT_PATH, INTERFACE, "Models", None, Some(glib::VariantTy::new("(s)").unwrap()),
                      gio::DBusCallFlags::NONE, 60000, None::<&gio::Cancellable>, move |res| {
                          done(res.map_err(|e| friendly(&e)).map(|v| {
                              serde_json::from_str(&v.child_value(0).get::<String>().unwrap_or_default()).unwrap_or(Value::Null)
                          }))
                      });
    }

    fn on_signal(&self, name: &str, params: &Variant) {
        let s = |i: usize| params.try_child_value(i).and_then(|v| v.get::<String>()).unwrap_or_default();
        let hooks = self.hooks.borrow();
        match name {
            "StateChanged" => return if let Some(f) = &hooks.state { f(&s(0), &s(1)) },
            "Clipboard" => return if let Some(f) = &hooks.clipboard { f(&s(0)) },
            "StepAside" => return if let Some(f) = &hooks.step_aside { f() },
            "StepBack" => return if let Some(f) = &hooks.step_back { f() },
            _ => {}
        }
        drop(hooks);
        let chat = s(0);
        let Some(turn) = self.turns.borrow().get(&chat).cloned() else { return };
        match name {
            "Text" => turn.on_text(&s(1)),
            "Reasoning" => turn.on_reasoning(&s(1)),
            "ToolStarted" => turn.on_tool_started(&s(1), &s(2)),
            "ToolFinished" => turn.on_tool_finished(&s(1), &s(2), params.child_value(3).get::<bool>().unwrap_or(false)),
            "Notice" => turn.on_notice(&s(1)),
            "Handoff" => turn.on_handoff(&s(1)),
            "Compacting" => turn.on_compacting(),
            "ConfirmRequest" => turn.on_confirm(params.child_value(1).get::<u32>().unwrap_or(0), &s(2), &s(3), &s(4),
                                                params.child_value(5).get::<bool>().unwrap_or(false)),
            "Finished" => {
                self.turns.borrow_mut().remove(&chat);
                turn.on_finished(&s(1));
            }
            _ => {}
        }
    }
}
