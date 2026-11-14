//! The chat loop: send messages, run the tools the model asks for, repeat.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::flm::{FlmClient, FlmError, StreamEvent, ToolCall, strip_think};
use crate::store;
use crate::tool::{Context, Risk, Tool};

pub const MAX_ROUNDS: usize = 6;

const SYSTEM_PROMPT: &str = "You are the user's assistant, running locally on the user's laptop \
(Fedora Silverblue with GNOME) on its NPU. Be brief and direct. When the user asks you \
to do something on the computer, or asks about its state, call a tool instead of \
describing how to do it. Only call tools the request needs. Never invent tool results; \
if a tool fails, say what failed. Answer in the user's language.";
const WEB_PROMPT: &str = " For anything recent or that you aren't sure of, use web_search and read the \
most relevant result with read_web_page, then answer briefly and name the source.";
const MEMORY_PROMPT: &str = " When the user tells you a lasting preference or fact about themselves, or asks you \
to remember something, call remember. Use what you remember without mentioning it.";
const SKILL_PROMPT: &str = " The user's saved skills are listed below; when a request matches one, call use_skill \
and follow its steps. When the user asks to save a workflow, call save_skill.";
const SCREEN_PROMPT: &str = " When the user says this, here, or asks about what they're looking at, call \
read_window first to read the window they mean.";
const VISION_PROMPT: &str = " You can see images. Use look_at_screen when the user asks about what's on their \
screen, and look_at_image for image files.";
const ESCALATE_PROMPT: &str = " If a request needs more than you can do well (careful reasoning, \
longer writing, code, several steps, or you're unsure), call ask_smarter_model instead \
of guessing. It hands the task to a bigger local model, which answers the user directly.";

pub const ESCALATE: &str = "ask_smarter_model";
pub const REPLY_RESERVE: usize = 1024;
const COMPACT_AT: f64 = 0.75;
const COMPACT_TO: f64 = 0.4;
const COMPACT_PROMPT: &str = "Summarize the conversation above in under 150 words for your own later use: facts, \
names, numbers, file paths, decisions and anything still to do. Plain text, no preamble.";
const IMAGE_TOKENS: usize = 300;
const TOOL_IMAGE_NOTE: &str = "(Image from the tool.)";
pub const DECLINED: &str = "The user declined this action.";

pub const WEB_TOOLS: [&str; 2] = ["web_search", "read_web_page"];
pub const MEMORY_TOOLS: [&str; 2] = ["remember", "forget"];
pub const VISION_TOOLS: [&str; 2] = ["look_at_screen", "look_at_image"];

/// How the agent talks to whoever runs it. Called from the agent's thread.
pub trait Callbacks: Send + Sync {
    fn text(&self, _text: &str) {}
    fn reasoning(&self, _text: &str) {}
    fn tool_started(&self, _name: &str, _summary: &str) {}
    fn tool_finished(&self, _name: &str, _result: &str, _ok: bool) {}
    fn notice(&self, _text: &str) {}
    fn handoff(&self, _model: &str) {}
    fn compacting(&self) {}
    /// Blocks until the user answers; true means run it.
    fn confirm(&self, _name: &str, _summary: &str, _detail: &str) -> bool {
        false
    }
}

/// Message content: plain text, or text plus pictures (OpenAI format).
pub fn with_images(text: &str, images: &[String]) -> Value {
    if images.is_empty() {
        return Value::String(text.into());
    }
    let mut parts = vec![json!({"type": "text", "text": text})];
    parts.extend(images.iter().map(|u| json!({"type": "image_url", "image_url": {"url": u}})));
    Value::Array(parts)
}

/// Rough token count: about 3.5 characters per token, pictures counted flat.
pub fn estimate_tokens(obj: &Value) -> usize {
    if let Some(parts) = obj.get("content").and_then(Value::as_array) {
        let images = parts.iter().filter(|p| p["type"] == "image_url").count();
        let text: usize = parts.iter().filter(|p| p["type"] == "text")
            .map(|p| p["text"].as_str().unwrap_or("").chars().count()).sum();
        return (text as f64 / 3.5) as usize + images * IMAGE_TOKENS + 8;
    }
    (obj.to_string().chars().count() as f64 / 3.5) as usize + 4
}

fn clip(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Messages as plain text for summarizing, newest kept if it's too long.
pub fn render_transcript(messages: &[Value], limit: usize) -> String {
    let mut lines = Vec::new();
    for m in messages {
        let content = store::plain_text(&m["content"]);
        match m["role"].as_str() {
            Some("user") => lines.push(format!("User: {}", clip(&content, 1500))),
            Some("assistant") => {
                for call in m["tool_calls"].as_array().into_iter().flatten() {
                    let f = &call["function"];
                    lines.push(format!("(Action {} {})", f["name"].as_str().unwrap_or(""),
                                       clip(f["arguments"].as_str().unwrap_or(""), 200)));
                }
                if !content.is_empty() {
                    lines.push(format!("Assistant: {}", clip(&content, 1500)));
                }
            }
            Some("tool") => lines.push(format!("(Result: {})", clip(&content, 400))),
            _ => {}
        }
    }
    let text = lines.join("\n");
    let n = text.chars().count();
    if n > limit { text.chars().skip(n - limit).collect() } else { text }
}

fn escalate_schema(model: &str) -> Value {
    json!({"type": "function", "function": {
        "name": ESCALATE,
        "description": format!("Hand this request to {model}, a bigger and smarter local model that can also \
use the tools. Use it for hard requests; it answers the user itself."),
        "parameters": {"type": "object", "properties": {
            "task": {"type": "string", "description": "What the user wants, and anything you found out"}},
            "required": ["task"]}}})
}

pub type ToolSource = Arc<dyn Fn() -> Vec<Tool> + Send + Sync>;

#[derive(Clone)]
pub struct Agent {
    pub client: Arc<FlmClient>,
    pub model: String,
    pub use_tools: bool,
    pub ctx: Context,
    pub messages: Vec<Value>,
    pub cancel: Arc<AtomicBool>,
    pub smarter_model: Option<String>,
    pub web_enabled: bool,
    /// The computer's description, taken once per chat.
    pub machine: Option<String>,
    pub machine_source: Arc<dyn Fn() -> String + Send + Sync>,
    pub memory_enabled: bool,
    pub sees: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub vision_model: Option<String>,
    /// Memory and skills, taken once per chat.
    pub notes: Option<String>,
    /// All tools: built-in and other apps' (MCP), looked up at each request.
    pub tools: ToolSource,
    pub always_allow: Arc<Mutex<HashSet<String>>>,
    /// Asked every time, never "Always Allow".
    pub always_ask: Arc<HashSet<String>>,
    pub ctx_tokens: usize,
    /// Compaction: messages before `compacted` are replaced by `summary`.
    pub summary: String,
    pub compacted: usize,
}

impl Agent {
    pub fn new(client: Arc<FlmClient>, model: &str, tools: ToolSource) -> Self {
        Self {
            client,
            model: model.into(),
            use_tools: true,
            ctx: Context::default(),
            messages: vec![],
            cancel: Arc::new(AtomicBool::new(false)),
            smarter_model: None,
            web_enabled: true,
            machine: None,
            machine_source: Arc::new(String::new),
            memory_enabled: true,
            sees: Arc::new(|_| false),
            vision_model: None,
            notes: None,
            tools,
            always_allow: Arc::new(Mutex::new(HashSet::new())),
            always_ask: Arc::new(["run_command".to_string()].into()),
            ctx_tokens: 8192,
            summary: String::new(),
            compacted: 0,
        }
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.machine = None;
        self.notes = None;
        self.summary.clear();
        self.compacted = 0;
    }

    fn sees_now(&self) -> bool {
        (self.sees)(&self.model)
    }

    fn can_escalate(&self) -> bool {
        self.use_tools && self.smarter_model.as_deref().is_some_and(|m| m != self.model)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn make_notes(&self) -> String {
        let mut parts = vec![];
        if self.memory_enabled {
            let facts = store::facts();
            if !facts.is_empty() {
                let list: Vec<String> = facts.iter().map(|f| format!("- {f}")).collect();
                parts.push(format!("What you remember about the user:\n{}", list.join("\n")));
            }
        }
        if self.use_tools {
            let skills = store::skills();
            if !skills.is_empty() {
                let list: Vec<String> = skills.iter().take(20).map(|s| {
                    if s.description.is_empty() { format!("- {}", s.title) } else { format!("- {}: {}", s.title, s.description) }
                }).collect();
                parts.push(format!("Saved skills:\n{}", list.join("\n")));
            }
        }
        parts.join("\n\n")
    }

    fn system_message(&mut self) -> Value {
        if self.machine.is_none() {
            self.machine = Some((self.machine_source)());
        }
        if self.notes.is_none() {
            self.notes = Some(self.make_notes());
        }
        let mut system = SYSTEM_PROMPT.to_string();
        if self.use_tools && self.web_enabled {
            system += WEB_PROMPT;
        }
        if self.use_tools && self.memory_enabled {
            system += MEMORY_PROMPT;
        }
        if self.use_tools {
            system += SKILL_PROMPT;
            system += SCREEN_PROMPT;
        }
        if self.use_tools && self.sees_now() {
            system += VISION_PROMPT;
        }
        if self.can_escalate() {
            system += ESCALATE_PROMPT;
        }
        // The machine description and notes go last and stay the same for the
        // whole chat, so the server can reuse the cached prompt between turns.
        let earlier = if self.summary.is_empty() { String::new() } else {
            format!("Earlier in this chat (summary):\n{}", self.summary)
        };
        let tail: Vec<&str> = [self.machine.as_deref().unwrap_or(""), self.notes.as_deref().unwrap_or(""), &earlier]
            .into_iter().filter(|s| !s.is_empty()).collect();
        json!({"role": "system", "content": format!("{system}\n\n{}", tail.join("\n\n"))})
    }

    fn available_tools(&self) -> Vec<Tool> {
        if !self.use_tools {
            return vec![];
        }
        let sees = self.sees_now();
        (self.tools)()
            .into_iter()
            .filter(|t| self.web_enabled || !WEB_TOOLS.contains(&t.name.as_str()))
            .filter(|t| self.memory_enabled || !MEMORY_TOOLS.contains(&t.name.as_str()))
            .filter(|t| sees || !VISION_TOOLS.contains(&t.name.as_str()))
            .collect()
    }

    fn tool_schemas(&self) -> Option<Vec<Value>> {
        if !self.use_tools {
            return None;
        }
        let mut schemas: Vec<Value> = self.available_tools().iter().map(Tool::schema).collect();
        if self.can_escalate() {
            schemas.push(escalate_schema(self.smarter_model.as_deref().unwrap()));
        }
        Some(schemas)
    }

    fn budget(&self, head: &Value) -> usize {
        let tools = self.tool_schemas().map(|t| estimate_tokens(&Value::Array(t))).unwrap_or(0);
        self.ctx_tokens.saturating_sub(estimate_tokens(head) + tools + REPLY_RESERVE)
    }

    fn turn_starts(&self) -> Vec<usize> {
        self.messages.iter().enumerate()
            .filter(|(i, m)| *i >= self.compacted && m["role"] == "user")
            .map(|(i, _)| i).collect()
    }

    fn tokens(messages: &[Value]) -> usize {
        messages.iter().map(estimate_tokens).sum()
    }

    /// The latest turns that fit; whole turns are dropped from the start.
    fn recent(&self, budget: usize) -> Vec<Value> {
        let starts = self.turn_starts();
        for &s in &starts {
            if Self::tokens(&self.messages[s..]) <= budget {
                return self.messages[s..].to_vec();
            }
        }
        match starts.last() {
            Some(&s) => self.messages[s..].to_vec(),
            None => self.messages[self.compacted.min(self.messages.len())..].to_vec(),
        }
    }

    pub fn conversation(&mut self) -> Vec<Value> {
        let head = self.system_message();
        let budget = self.budget(&head);
        let mut out = vec![head];
        out.extend(self.recent(budget));
        out
    }

    /// Summarize the oldest turns once the chat fills most of the context.
    fn maybe_compact(&mut self, cb: &dyn Callbacks) {
        let head = self.system_message();
        let budget = self.budget(&head) as f64;
        let live = Self::tokens(&self.messages[self.compacted.min(self.messages.len())..]) as f64;
        let starts = self.turn_starts();
        if live <= budget * COMPACT_AT || starts.len() < 2 {
            return;
        }
        let cut = starts[1..].iter().copied()
            .find(|&s| Self::tokens(&self.messages[s..]) as f64 <= budget * COMPACT_TO)
            .unwrap_or(*starts.last().unwrap());
        let limit = self.ctx_tokens.saturating_sub(REPLY_RESERVE + 400) * 3;
        let transcript = render_transcript(&self.messages[self.compacted..cut], limit);
        let previous = if self.summary.is_empty() { String::new() } else { format!("Summary so far:\n{}\n\n", self.summary) };
        let prompt = vec![
            json!({"role": "system", "content": "You write short, factual summaries of conversations."}),
            json!({"role": "user", "content": format!("{previous}Conversation:\n{transcript}\n\n{COMPACT_PROMPT}")}),
        ];
        cb.compacting();
        let Ok(stream) = self.client.chat_stream(&self.model, &prompt, None) else { return };
        let mut text = String::new();
        for ev in stream {
            match ev {
                Ok(StreamEvent::Text(t)) => text.push_str(&t),
                Ok(_) => {}
                Err(_) => return, // turns get dropped instead
            }
        }
        let summary = strip_think(&text).trim().to_string();
        if !summary.is_empty() {
            self.summary = clip(&summary, 2000).to_string();
            self.compacted = cut;
        }
    }

    /// Earlier pictures cost hundreds of tokens each; keep a marker only.
    fn drop_old_images(&mut self) {
        for m in &mut self.messages {
            if m["content"].is_array() {
                m["content"] = Value::String(store::plain_text(&m["content"]));
            }
        }
    }

    fn helper(&self, model: &str) -> Agent {
        let mut h = self.clone();
        h.model = model.into();
        h.use_tools = true;
        h.messages = vec![];
        h.compacted = 0;
        h.smarter_model = None;
        h
    }

    /// Run one user turn to completion. Blocking.
    pub fn ask(&mut self, text: &str, cb: &dyn Callbacks, images: &[String]) -> Result<(), FlmError> {
        self.cancel.store(false, Ordering::SeqCst);
        self.drop_old_images();
        if self.use_tools || !self.messages.is_empty() {
            self.maybe_compact(cb);
        }
        let mut images = images.to_vec();
        if !images.is_empty() && !self.sees_now() {
            if let Some(vm) = self.vision_model.clone().filter(|v| *v != self.model) {
                return self.hand_images(&vm, text, &images, cb);
            }
            cb.notice(&format!("{} can't see images, so they were left out.", self.model));
            images.clear();
        }
        self.messages.push(json!({"role": "user", "content": with_images(text, &images)}));
        for _ in 0..MAX_ROUNDS {
            if self.cancelled() {
                return Ok(());
            }
            let schemas = self.tool_schemas();
            let (content, calls) = match self.stream(schemas.as_deref(), cb) {
                Err(e) if schemas.is_some() && matches!(e.status, Some(400 | 422)) => {
                    self.use_tools = false;
                    cb.notice(&format!("{} can't use tools, so it can only chat.", self.model));
                    self.stream(None, cb)?
                }
                other => other?,
            };
            let mut assistant = json!({"role": "assistant", "content": content});
            if !calls.is_empty() {
                assistant["tool_calls"] = calls.iter().map(|c| json!({"id": c.id, "type": "function",
                    "function": {"name": c.name, "arguments": if c.arguments.is_empty() { "{}" } else { &c.arguments }}}))
                    .collect();
            }
            self.messages.push(assistant);
            if calls.is_empty() || self.cancelled() {
                return Ok(());
            }
            if self.can_escalate()
                && let Some(call) = calls.iter().find(|c| c.name == ESCALATE) {
                    return self.escalate(call, text, cb);
                }
            for call in &calls {
                let result = self.run_call(call, cb);
                self.messages.push(json!({"role": "tool", "tool_call_id": call.id, "content": result}));
            }
            let shots: Vec<String> = std::mem::take(&mut *self.ctx.images.lock().unwrap());
            if !shots.is_empty() && self.sees_now() {
                // Tool results are text only, so pictures follow as a message of their own.
                self.messages.push(json!({"role": "user", "content": with_images(TOOL_IMAGE_NOTE, &shots)}));
            }
        }
        cb.notice("Stopped after too many tool calls in a row.");
        Ok(())
    }

    /// The smarter model finishes this turn; its answer is the reply.
    fn escalate(&mut self, call: &ToolCall, request: &str, cb: &dyn Callbacks) -> Result<(), FlmError> {
        let task = call.parsed_arguments().ok()
            .and_then(|a| a.get("task").and_then(Value::as_str).map(|s| s.trim().to_string()))
            .unwrap_or_default();
        let model = self.smarter_model.clone().unwrap();
        cb.handoff(&model);
        let mut helper = self.helper(&model);
        let end = self.messages.len().saturating_sub(2);
        helper.messages = self.messages[self.compacted.min(end)..end].iter()
            .filter(|m| matches!(m["role"].as_str(), Some("user" | "assistant")))
            .map(|m| json!({"role": m["role"], "content": store::plain_text(&m["content"])}))
            .filter(|m| m["content"].as_str().is_some_and(|s| !s.is_empty()))
            .collect();
        let note = if task.is_empty() { String::new() } else {
            format!("\n\n(A smaller assistant handed this to you. Its notes: {task})")
        };
        helper.ask(&format!("{request}{note}"), cb, &[])?;
        let answer = last_answer(&helper.messages);
        self.messages.push(json!({"role": "tool", "tool_call_id": call.id,
                                  "content": format!("{model} answered the user directly:\n{answer}")}));
        self.messages.push(json!({"role": "assistant", "content": answer}));
        Ok(())
    }

    /// This model is blind: the vision model answers this turn.
    fn hand_images(&mut self, model: &str, text: &str, images: &[String], cb: &dyn Callbacks) -> Result<(), FlmError> {
        cb.handoff(model);
        let mut helper = self.helper(model);
        helper.use_tools = self.use_tools;
        helper.sees = Arc::new(|_| true);
        helper.messages = self.messages[self.compacted.min(self.messages.len())..].iter()
            .filter(|m| matches!(m["role"].as_str(), Some("user" | "assistant")))
            .filter(|m| m["content"].as_str().is_some_and(|s| !s.is_empty()))
            .map(|m| json!({"role": m["role"], "content": m["content"]}))
            .collect();
        helper.ask(text, cb, images)?;
        let answer = last_answer(&helper.messages);
        self.messages.push(json!({"role": "user",
                                  "content": format!("{text}\n{}", store::IMAGE_MARKER.repeat(images.len())).trim()}));
        self.messages.push(json!({"role": "assistant", "content": answer}));
        Ok(())
    }

    fn stream(&mut self, schemas: Option<&[Value]>, cb: &dyn Callbacks) -> Result<(String, Vec<ToolCall>), FlmError> {
        let conversation = self.conversation();
        let stream = self.client.chat_stream(&self.model, &conversation, schemas)?;
        let mut content = String::new();
        let mut calls = vec![];
        for ev in stream {
            if self.cancelled() {
                return Ok((strip_think(&content), vec![]));
            }
            match ev? {
                StreamEvent::Text(t) => {
                    content.push_str(&t);
                    cb.text(&t);
                }
                StreamEvent::Reasoning(r) => cb.reasoning(&r),
                StreamEvent::Done { tool_calls, finish_reason } => {
                    calls = tool_calls;
                    if finish_reason.as_deref() == Some("length") && !calls.is_empty() {
                        cb.notice("The answer was cut off before the action was complete.");
                        calls.clear();
                    }
                }
            }
        }
        Ok((strip_think(&content), calls))
    }

    fn run_call(&self, call: &ToolCall, cb: &dyn Callbacks) -> String {
        let args = match call.parsed_arguments() {
            Ok(a) => a,
            Err(_) => {
                let r = format!("Error: arguments for {} were not valid JSON.", call.name);
                cb.tool_finished(&call.name, &r, false);
                return r;
            }
        };
        let Some(tool) = self.available_tools().into_iter().find(|t| t.name == call.name) else {
            let r = format!("Error: there is no tool named {}.", call.name);
            cb.tool_finished(&call.name, &r, false);
            return r;
        };
        let summary = (tool.summary)(&args);
        let detail = tool.detail.as_ref().map(|d| d(&args)).unwrap_or_default();
        let allowed = self.always_allow.lock().unwrap().contains(&call.name);
        let needs_ok = tool.risk == Risk::Confirm && (self.always_ask.contains(&call.name) || !allowed);
        if needs_ok && !cb.confirm(&call.name, &summary, &detail) {
            cb.tool_finished(&call.name, "Cancelled", false);
            return DECLINED.into();
        }
        cb.tool_started(&call.name, &summary);
        let run = tool.run.clone();
        let ctx = self.ctx.clone();
        // A panicking tool is reported like a failed one instead of ending the turn.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&ctx, &args)));
        match outcome {
            Ok(Ok(result)) => {
                cb.tool_finished(&call.name, &result, true);
                result
            }
            Ok(Err(e)) => {
                cb.tool_finished(&call.name, &e.0, false);
                format!("Error: {}", e.0)
            }
            Err(_) => {
                cb.tool_finished(&call.name, "The tool crashed", false);
                "Error: the tool crashed".into()
            }
        }
    }
}

fn last_answer(messages: &[Value]) -> String {
    messages.iter().rev()
        .find(|m| m["role"] == "assistant" && m["content"].as_str().is_some_and(|s| !s.is_empty()))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_flm::{FakeFlm, text_reply, tool_reply};
    use crate::store::testenv::temp_home;
    use crate::tool::{ToolError, obj};

    #[derive(Default)]
    struct Rec {
        log: Mutex<Vec<String>>,
        allow: bool,
    }

    impl Callbacks for Rec {
        fn text(&self, t: &str) {
            self.log.lock().unwrap().push(format!("text:{t}"));
        }
        fn tool_finished(&self, n: &str, r: &str, ok: bool) {
            self.log.lock().unwrap().push(format!("tool:{n}:{ok}:{r}"));
        }
        fn notice(&self, t: &str) {
            self.log.lock().unwrap().push(format!("notice:{t}"));
        }
        fn handoff(&self, m: &str) {
            self.log.lock().unwrap().push(format!("handoff:{m}"));
        }
        fn compacting(&self) {
            self.log.lock().unwrap().push("compacting".into());
        }
        fn confirm(&self, n: &str, s: &str, _d: &str) -> bool {
            self.log.lock().unwrap().push(format!("confirm:{n}:{s}"));
            self.allow
        }
    }

    fn test_tools() -> ToolSource {
        Arc::new(|| vec![
            Tool::new("get_datetime", "Time", obj(json!({}), &[]), Risk::Safe, |_, _| Ok("It's 22:00.".into()),
                      |_| "Read the time".into()),
            Tool::new("send_file", "Send", obj(json!({"path": {"type": "string"}}), &["path"]), Risk::Confirm,
                      |_, _| Ok("Sent.".into()), |a| format!("Send {}", crate::tool::arg_str(a, "path"))),
            Tool::new("run_command", "Run", obj(json!({}), &[]), Risk::Confirm, |_, _| Ok("ran".into()),
                      |_| "Run".into()),
            Tool::new("set_volume", "Vol", obj(json!({}), &[]), Risk::Safe,
                      |_, a| crate::tool::arg_percent(a, "percent", 0).map(|p| format!("{p}%")), |_| "Vol".into()),
            Tool::new("look_at_screen", "Look", obj(json!({}), &[]), Risk::Safe, |ctx, _| {
                ctx.images.lock().unwrap().push("data:image/jpeg;base64,AAAA".into());
                Ok("Took a screenshot.".into())
            }, |_| "Look".into()),
            Tool::new("broken", "x", obj(json!({}), &[]), Risk::Safe, |_, _| Err(ToolError("nope".into())),
                      |_| "x".into()),
        ])
    }

    fn agent(flm: &FakeFlm) -> Agent {
        let mut a = Agent::new(Arc::new(FlmClient::new(&flm.url)), "test-model", test_tools());
        a.machine = Some("About this computer: test".into());
        a
    }

    #[test]
    fn safe_tool_runs_and_result_goes_back() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![(200, tool_reply("get_datetime", "{}", "call_1")), (200, text_reply("It's late."))]);
        let mut a = agent(&flm);
        let rec = Rec::default();
        a.ask("what time is it", &rec, &[]).unwrap();
        assert!(rec.log.lock().unwrap().contains(&"text:It's late.".to_string()));
        let reqs = flm.requests();
        let msgs = reqs[1]["messages"].as_array().unwrap();
        assert_eq!(msgs[msgs.len() - 2]["tool_calls"][0]["function"]["name"], "get_datetime");
        assert_eq!(msgs[msgs.len() - 1]["content"], "It's 22:00.");
        assert_eq!(msgs[msgs.len() - 1]["tool_call_id"], "call_1");
        assert!(reqs[0]["tools"].as_array().unwrap().iter().all(|t| t["function"]["name"] != "look_at_screen"));
    }

    #[test]
    fn risky_tool_asks_and_always_allow_skips_except_commands() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![
            (200, tool_reply("send_file", r#"{"path": "a.txt"}"#, "c1")), (200, text_reply("OK.")),
            (200, tool_reply("send_file", r#"{"path": "b.txt"}"#, "c2")),
            (200, tool_reply("run_command", "{}", "c3")), (200, text_reply("Done.")),
        ]);
        let mut a = agent(&flm);
        let rec = Rec::default();
        a.ask("send a.txt", &rec, &[]).unwrap();
        assert!(rec.log.lock().unwrap().contains(&"confirm:send_file:Send a.txt".to_string()));
        assert_eq!(flm.requests()[1]["messages"].as_array().unwrap().last().unwrap()["content"], DECLINED);
        a.always_allow.lock().unwrap().extend(["send_file".to_string(), "run_command".to_string()]);
        let rec = Rec::default();
        a.ask("send b, then run", &rec, &[]).unwrap();
        let confirms: Vec<String> = rec.log.lock().unwrap().iter().filter(|l| l.starts_with("confirm")).cloned().collect();
        assert_eq!(confirms, ["confirm:run_command:Run"]);
    }

    #[test]
    fn bad_and_unknown_tools_are_reported() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![
            (200, tool_reply("set_volume", r#"{"percent": 500}"#, "c1")), (200, tool_reply("rm_rf", "{}", "c2")),
            (200, tool_reply("broken", "{}", "c3")), (200, text_reply("Sorry.")),
        ]);
        let mut a = agent(&flm);
        let rec = Rec::default();
        a.ask("max volume", &rec, &[]).unwrap();
        let log = rec.log.lock().unwrap().join("\n");
        assert!(log.contains("tool:set_volume:false:'percent' must be from 0 to 100"));
        assert!(log.contains("tool:rm_rf:false:Error: there is no tool named rm_rf."));
        assert!(log.contains("tool:broken:false:nope"));
    }

    #[test]
    fn tools_refused_means_chat_only() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![(400, "tools not supported".into()), (200, text_reply("Hi."))]);
        let mut a = agent(&flm);
        let rec = Rec::default();
        a.ask("hi", &rec, &[]).unwrap();
        assert!(!a.use_tools);
        assert!(flm.requests()[1].get("tools").is_none());
        assert!(rec.log.lock().unwrap().iter().any(|l| l.contains("can only chat")));
    }

    #[test]
    fn escalation_hands_the_turn_to_the_smarter_model() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![
            (200, tool_reply(ESCALATE, r#"{"task": "write a poem"}"#, "c1")), (200, text_reply("A poem.")),
        ]);
        let mut a = agent(&flm);
        a.smarter_model = Some("big:9b".into());
        let rec = Rec::default();
        a.ask("poem please", &rec, &[]).unwrap();
        let reqs = flm.requests();
        assert_eq!(reqs[1]["model"], "big:9b");
        assert!(reqs[1]["tools"].as_array().unwrap().iter().all(|t| t["function"]["name"] != ESCALATE));
        assert!(reqs[1]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("write a poem"));
        assert_eq!(a.messages.last().unwrap()["content"], "A poem.");
        assert!(rec.log.lock().unwrap().contains(&"handoff:big:9b".to_string()));
    }

    #[test]
    fn images_screenshots_and_blind_models() {
        let _h = temp_home();
        let img = "data:image/jpeg;base64,AAAA".to_string();
        // A seeing model gets the picture; a later turn keeps only a marker.
        let flm = FakeFlm::start(vec![(200, text_reply("A panda.")), (200, text_reply("Yes.")),
                                      (200, tool_reply("look_at_screen", "{}", "c1")), (200, text_reply("A terminal."))]);
        let mut a = agent(&flm);
        a.sees = Arc::new(|_| true);
        a.ask("what is it", &Rec::default(), std::slice::from_ref(&img)).unwrap();
        a.ask("sure?", &Rec::default(), &[]).unwrap();
        a.ask("what's on screen", &Rec::default(), &[]).unwrap();
        let reqs = flm.requests();
        assert_eq!(reqs[0]["messages"].as_array().unwrap().last().unwrap()["content"][1]["image_url"]["url"], img.as_str());
        assert_eq!(reqs[1]["messages"][1]["content"], "what is it\n[image]");
        let last = reqs[3]["messages"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["content"][1]["image_url"]["url"], img.as_str());
        // A blind model hands the turn to the vision model.
        let flm = FakeFlm::start(vec![(200, text_reply("A cat."))]);
        let mut a = agent(&flm);
        a.vision_model = Some("seer".into());
        let rec = Rec::default();
        a.ask("what is it", &rec, std::slice::from_ref(&img)).unwrap();
        assert_eq!(flm.requests()[0]["model"], "seer");
        assert_eq!(a.messages[0]["content"], "what is it\n[image]");
        assert_eq!(a.messages[1]["content"], "A cat.");
    }

    #[test]
    fn memory_and_skills_reach_the_prompt() {
        let _h = temp_home();
        store::remember("Prefers metric units").unwrap();
        store::save_skill("Morning", "1. a", "Start the day").unwrap();
        let flm = FakeFlm::start(vec![(200, text_reply("Hi"))]);
        let mut a = agent(&flm);
        a.ask("hi", &Rec::default(), &[]).unwrap();
        let system = flm.requests()[0]["messages"][0]["content"].as_str().unwrap().to_string();
        assert!(system.contains("- Prefers metric units"));
        assert!(system.contains("- Morning: Start the day"));
    }

    #[test]
    fn compaction() {
        let _h = temp_home();
        let flm = FakeFlm::start(vec![(200, text_reply("User asked six questions.")), (200, text_reply("Sure."))]);
        let mut a = agent(&flm);
        a.use_tools = false;
        a.ctx_tokens = 3000;
        for i in 0..6 {
            a.messages.push(json!({"role": "user", "content": format!("question {i} {}", "x".repeat(900))}));
            a.messages.push(json!({"role": "assistant", "content": format!("answer {i} {}", "y".repeat(900))}));
        }
        let rec = Rec::default();
        a.ask("next", &rec, &[]).unwrap();
        assert!(rec.log.lock().unwrap().contains(&"compacting".to_string()));
        let reqs = flm.requests();
        assert!(reqs[0].get("tools").is_none());
        let msgs = reqs[1]["messages"].as_array().unwrap();
        assert!(msgs[0]["content"].as_str().unwrap().contains("Earlier in this chat (summary):\nUser asked six questions."));
        assert_eq!(msgs.last().unwrap()["content"], "next");
        assert!(a.compacted > 0);
        assert_eq!(a.messages.len(), 14);
        assert!(Agent::tokens(msgs) < 3000 - REPLY_RESERVE);
    }

    #[test]
    fn transcript_and_tokens() {
        let msgs = vec![json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "get_battery", "arguments": "{}"}}]}),
            json!({"role": "tool", "content": "81%"}), json!({"role": "assistant", "content": "81 percent"})];
        assert_eq!(render_transcript(&msgs, 1000), "User: hi\n(Action get_battery {})\n(Result: 81%)\nAssistant: 81 percent");
        assert_eq!(render_transcript(&[json!({"role": "user", "content": "z".repeat(500)})], 100).len(), 100);
        assert_eq!(estimate_tokens(&json!({"content": [{"type": "image_url"}]})), IMAGE_TOKENS + 8);
    }
}
