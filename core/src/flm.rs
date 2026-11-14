//! FastFlowLM (FLM) client: server lifecycle, model list, streaming chat.
//!
//! FLM serves an OpenAI-compatible API on 127.0.0.1:52625. It keeps one model
//! loaded and swaps to whichever model a request names.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Map, Value, json};

pub const DEFAULT_URL: &str = "http://127.0.0.1:52625";
pub const DEFAULT_CTX_LEN: u32 = 8192;
/// Qwen 3.5/3.6 keep earlier prompts in a host-memory prefix cache (FLM's
/// default limit is 4 GiB); the assistant's prompts are short.
const PREFIX_CACHE_MB: u32 = 1024;
const PREFIX_CACHE_FAMILIES: [&str; 2] = ["qwen3.5", "qwen3.6"];

#[derive(Debug, Clone, PartialEq)]
pub struct FlmError {
    pub message: String,
    pub status: Option<u16>,
}

impl FlmError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), status: None }
    }
}

impl std::fmt::Display for FlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FlmError {}

pub fn serve_command(flm: &str, model: &str, ctx_len: Option<u32>) -> Vec<String> {
    let mut cmd = vec![flm.to_string(), "serve".into(), model.to_string()];
    if let Some(ctx) = ctx_len {
        cmd.extend(["--ctx-len".into(), ctx.to_string()]);
    }
    let family = model.split(':').next().unwrap_or("").split('-').next().unwrap_or("");
    if PREFIX_CACHE_FAMILIES.contains(&family) {
        cmd.extend(["--prefix-cache-mb".into(), PREFIX_CACHE_MB.to_string()]);
    }
    cmd
}

// -- models -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub name: String,
    pub installed: bool,
    pub labels: Vec<String>,
}

impl Model {
    pub fn new(name: &str, installed: bool, labels: &[&str]) -> Self {
        Self { name: name.into(), installed, labels: labels.iter().map(|s| s.to_string()).collect() }
    }
    fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
    pub fn can_use_tools(&self) -> bool {
        self.has("tool-calling")
    }
    pub fn single_turn(&self) -> bool {
        self.has("single-turn")
    }
    pub fn can_see(&self) -> bool {
        self.has("vision")
    }
    /// Rough parameter count in billions from the tag ("e4b" -> 4, "35b-a3b" -> 35).
    pub fn size_b(&self) -> f64 {
        static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^e?(\d+(?:\.\d+)?)([bm])").unwrap());
        let tag = self.name.split_once(':').map(|(_, t)| t).unwrap_or("");
        match SIZE.captures(tag) {
            Some(c) => {
                let n: f64 = c[1].parse().unwrap_or(0.0);
                if &c[2] == "m" { n / 1000.0 } else { n }
            }
            None => 0.0,
        }
    }
}

pub fn total_ram_gib() -> f64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map(|kib| kib / 1024.0 / 1024.0)
        .unwrap_or(0.0)
}

fn ram_limit(ram_gib: Option<f64>) -> f64 {
    let ram = ram_gib.unwrap_or_else(total_ram_gib);
    if ram > 0.0 { ram * 0.75 } else { 12.0 }
}

/// The biggest installed tool-calling model that should fit in RAM (4-bit
/// weights take about 0.6 GiB per billion parameters).
pub fn pick_smarter_model(models: &[Model], current: &str, ram_gib: Option<f64>) -> Option<String> {
    let limit = ram_limit(ram_gib);
    let cur = models.iter().find(|m| m.name == current);
    models
        .iter()
        .filter(|m| m.installed && m.can_use_tools() && !m.single_turn() && m.name != current)
        .filter(|m| m.size_b() <= limit && cur.is_none_or(|c| m.size_b() > c.size_b()))
        .max_by(|a, b| a.size_b().total_cmp(&b.size_b()))
        .map(|m| m.name.clone())
}

/// A model to hand pictures to: tool-calling and multi-turn preferred, then small.
pub fn pick_vision_model(models: &[Model], current: &str, ram_gib: Option<f64>) -> Option<String> {
    let limit = ram_limit(ram_gib);
    models
        .iter()
        .filter(|m| m.installed && m.can_see() && m.name != current && m.size_b() <= limit)
        .min_by(|a, b| {
            (!a.can_use_tools(), a.single_turn())
                .cmp(&(!b.can_use_tools(), b.single_turn()))
                .then(a.size_b().total_cmp(&b.size_b()))
        })
        .map(|m| m.name.clone())
}

fn find_model_list() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(flm) = which("flm")
        && let Ok(real) = fs::canonicalize(&flm)
            && let Some(dir) = real.parent() {
                candidates.push(dir.join("model_list.json"));
            }
    for dir in ["/usr/share/flm", "/usr/share/fastflowlm", "/usr/lib64/flm", "/usr/lib/flm", "/usr/lib64/fastflowlm",
                "/usr/lib/fastflowlm", "/opt/fastflowlm", "/opt/flm"] {
        candidates.push(Path::new(dir).join("model_list.json"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Labels ("tool-calling", "single-turn", ...) from FLM's model_list.json.
pub fn model_labels() -> BTreeMap<String, Vec<String>> {
    find_model_list()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| parse_model_labels(&s))
        .unwrap_or_default()
}

pub fn parse_model_labels(json_text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Ok(config) = serde_json::from_str::<Value>(json_text) else { return out };
    if let Some(models) = config.get("models").and_then(Value::as_object) {
        for (family, sizes) in models {
            for (size, info) in sizes.as_object().into_iter().flatten() {
                let labels = info
                    .get("label")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                out.insert(format!("{family}:{size}"), labels);
            }
        }
    }
    out
}

/// `flm list` output -> {model: installed}, in listed order.
pub fn parse_flm_list(output: &str) -> Vec<(String, bool)> {
    static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-\s*(\S+)\s*(.*)$").unwrap());
    output
        .lines()
        .filter_map(|l| LINE.captures(l).map(|c| (c[1].to_string(), c[2].contains('✅'))))
        .collect()
}

/// Drop inline <think> blocks some models leave in the answer text.
pub fn strip_think(text: &str) -> String {
    static THINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<think>.*?(</think>|$)").unwrap());
    THINK.replace_all(text, "").trim_start().to_string()
}

fn log_tail(path: &Path, lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else { return String::new() };
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &all[all.len().saturating_sub(lines)..];
    if tail.is_empty() {
        String::new()
    } else {
        format!("\n\nLast lines of {}:\n{}", path.display(), tail.join("\n"))
    }
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| p.is_file())
}

// -- streaming ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    pub fn parsed_arguments(&self) -> Result<Map<String, Value>, String> {
        if self.arguments.trim().is_empty() {
            return Ok(Map::new());
        }
        match serde_json::from_str::<Value>(&self.arguments) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err("tool arguments are not a JSON object".into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
    Done { tool_calls: Vec<ToolCall>, finish_reason: Option<String> },
}

/// Turns OpenAI chat-completion SSE lines into events. Tool call deltas are
/// merged by index (only a call's first delta has its id and name) and
/// reported once, in the final `Done`.
#[derive(Default)]
pub struct SseParser {
    calls: BTreeMap<i64, ToolCall>,
    finish_reason: Option<String>,
    pub finished: bool,
}

impl SseParser {
    pub fn feed(&mut self, raw: &str) -> Result<Vec<StreamEvent>, FlmError> {
        let mut out = Vec::new();
        let line = raw.trim();
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { return Ok(out) };
        if data == "[DONE]" {
            self.finished = true;
            return Ok(out);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else { return Ok(out) };
        let choices = chunk.get("choices").and_then(Value::as_array);
        if let (Some(err), true) = (chunk.get("error"), choices.is_none_or(|c| c.is_empty())) {
            let msg = err.get("message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| err.to_string());
            return Err(FlmError::new(msg));
        }
        for choice in choices.into_iter().flatten() {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
            let Some(delta) = choice.get("delta") else { continue };
            if let Some(r) = delta.get("reasoning_content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.push(StreamEvent::Reasoning(r.to_string()));
            }
            if let Some(t) = delta.get("content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.push(StreamEvent::Text(t.to_string()));
            }
            for tc in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let idx = tc.get("index").and_then(Value::as_i64).unwrap_or(0);
                let func = tc.get("function").cloned().unwrap_or(Value::Null);
                let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                let call = self.calls.entry(idx).or_insert_with(|| ToolCall {
                    id: tc.get("id").and_then(Value::as_str).map(String::from).unwrap_or(format!("call_{idx}")),
                    name: String::new(),
                    arguments: String::new(),
                });
                if !name.is_empty() {
                    call.name = name.to_string();
                }
                call.arguments.push_str(func.get("arguments").and_then(Value::as_str).unwrap_or(""));
            }
        }
        Ok(out)
    }

    pub fn finish(&mut self) -> StreamEvent {
        self.finished = true;
        StreamEvent::Done {
            tool_calls: std::mem::take(&mut self.calls).into_values().collect(),
            finish_reason: self.finish_reason.take(),
        }
    }
}

/// A chat completion being streamed; yields events until `Done`.
pub struct ChatStream {
    reader: Option<Box<dyn BufRead + Send>>,
    parser: SseParser,
    pending: std::collections::VecDeque<StreamEvent>,
    done: bool,
}

impl ChatStream {
    pub fn from_reader(reader: impl Read + Send + 'static) -> Self {
        Self { reader: Some(Box::new(BufReader::new(reader))), parser: SseParser::default(),
               pending: Default::default(), done: false }
    }
}

impl Iterator for ChatStream {
    type Item = Result<StreamEvent, FlmError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Some(Ok(ev));
            }
            if self.done {
                return None;
            }
            if self.parser.finished {
                self.done = true;
                self.reader = None;
                return Some(Ok(self.parser.finish()));
            }
            let reader = self.reader.as_mut()?;
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => self.parser.finished = true,
                Ok(_) => match self.parser.feed(&line) {
                    Ok(evs) => self.pending.extend(evs),
                    Err(e) => {
                        self.done = true;
                        return Some(Err(e));
                    }
                },
                Err(e) => {
                    self.done = true;
                    return Some(Err(FlmError::new(format!("Lost the connection to FastFlowLM: {e}"))));
                }
            }
        }
    }
}

// -- client -----------------------------------------------------------------------

struct Server {
    child: Child,
    ctx: Option<u32>,
}

pub struct FlmClient {
    pub base_url: String,
    agent: ureq::Agent,
    server: Mutex<Option<Server>>,
    /// The model FLM most likely has loaded (it swaps per request).
    pub last_model: Mutex<Option<String>>,
}

impl FlmClient {
    pub fn new(base_url: &str) -> Self {
        // FLM is local: never go through an http_proxy from the environment.
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .build();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            agent: config.into(),
            server: Mutex::new(None),
            last_model: Mutex::new(None),
        }
    }

    pub fn from_env() -> Self {
        Self::new(&std::env::var("PANDA_FLM_URL").unwrap_or_else(|_| DEFAULT_URL.into()))
    }

    /// True once anything answers HTTP on the port (older FLM releases answer
    /// /api/version with an error status even when ready).
    pub fn is_up(&self) -> bool {
        ["/api/version", "/v1/models"].iter().any(|p| {
            self.agent
                .get(&format!("{}{}", self.base_url, p))
                .config()
                .timeout_global(Some(Duration::from_secs(2)))
                .build()
                .call()
                .is_ok()
        })
    }

    pub fn owns_server(&self) -> bool {
        let mut guard = self.server.lock().unwrap();
        match guard.as_mut() {
            Some(s) => matches!(s.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn log_path() -> PathBuf {
        crate::paths::state_dir().join("flm-serve.log")
    }

    /// Start `flm serve` unless one is listening. True if this call started it.
    /// A server this service started with another context length is restarted.
    pub fn ensure_server(&self, model: &str, ctx_len: Option<u32>, wait: Duration) -> Result<bool, FlmError> {
        let mut guard = self.server.lock().unwrap();
        if let Some(s) = guard.as_mut()
            && s.ctx != ctx_len && matches!(s.child.try_wait(), Ok(None)) {
                stop_child(guard.take().unwrap().child);
            }
        if self.is_up() {
            return Ok(false);
        }
        let flm = which("flm").ok_or_else(|| FlmError::new("FastFlowLM (flm) is not installed."))?;
        let log_path = Self::log_path();
        if let Some(dir) = log_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let log = OpenOptions::new().create(true).append(true).open(&log_path)
            .map_err(|e| FlmError::new(format!("Can't write {}: {e}", log_path.display())))?;
        let cmd = serve_command(&flm.to_string_lossy(), model, ctx_len);
        let mut command = Command::new(&cmd[0]);
        command.args(&cmd[1..]).stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| FlmError::new(e.to_string()))?)
            .stderr(log);
        new_session(&mut command);
        let child = command.spawn().map_err(|e| FlmError::new(format!("Couldn't start FastFlowLM: {e}")))?;
        *guard = Some(Server { child, ctx: ctx_len });
        *self.last_model.lock().unwrap() = Some(model.to_string());
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = guard.as_mut().unwrap().child.try_wait() {
                *guard = None;
                return Err(FlmError::new(format!(
                    "FastFlowLM stopped while starting (exit code {}).{}",
                    status.code().unwrap_or(-1), log_tail(&log_path, 6))));
            }
            if self.is_up() {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        stop_child(guard.take().unwrap().child);
        Err(FlmError::new(format!("FastFlowLM didn't start within {} minutes.{}", wait.as_secs() / 60,
                                  log_tail(&log_path, 6))))
    }

    /// Stop the server only if this service started it. True if one was stopped.
    pub fn stop_server(&self) -> bool {
        *self.last_model.lock().unwrap() = None;
        let Some(mut s) = self.server.lock().unwrap().take() else { return false };
        if matches!(s.child.try_wait(), Ok(None)) {
            stop_child(s.child);
            true
        } else {
            false
        }
    }

    pub fn list_models(&self) -> Result<Vec<Model>, FlmError> {
        let flm = which("flm").ok_or_else(|| FlmError::new("FastFlowLM (flm) is not installed."))?;
        let out = Command::new(flm).arg("list").stdin(Stdio::null()).output()
            .map_err(|e| FlmError::new(format!("flm list failed: {e}")))?;
        let installed = parse_flm_list(&String::from_utf8_lossy(&out.stdout));
        let labels = model_labels();
        let names: Vec<(String, bool)> = if installed.is_empty() {
            labels.keys().map(|k| (k.clone(), false)).collect()
        } else {
            installed
        };
        Ok(names
            .into_iter()
            .map(|(name, inst)| Model { labels: labels.get(&name).cloned().unwrap_or_default(), name, installed: inst })
            .collect())
    }

    pub fn chat_stream(&self, model: &str, messages: &[Value], tools: Option<&[Value]>) -> Result<ChatStream, FlmError> {
        *self.last_model.lock().unwrap() = Some(model.to_string());
        let mut body = json!({"model": model, "messages": messages, "stream": true});
        if let Some(t) = tools.filter(|t| !t.is_empty()) {
            body["tools"] = Value::Array(t.to_vec());
        }
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", "Bearer flm")
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .send(&serde_json::to_vec(&body).unwrap_or_default()[..])
            .map_err(|e| FlmError::new(format!("Can't reach FastFlowLM at {}: {e}", self.base_url)))?;
        let status = resp.status().as_u16();
        let reader = resp.into_body().into_reader();
        if status >= 400 {
            let mut detail = String::new();
            let _ = reader.take(500).read_to_string(&mut detail);
            return Err(FlmError { message: format!("FLM returned {status}: {detail}"), status: Some(status) });
        }
        Ok(ChatStream::from_reader(reader))
    }
}

impl Drop for FlmClient {
    fn drop(&mut self) {
        self.stop_server();
    }
}

fn stop_child(mut child: Child) {
    // SIGTERM first so FLM can release the NPU cleanly.
    unsafe {
        libc_kill(child.id() as i32, 15);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
    fn setsid() -> i32;
}

fn new_session(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and only detaches the child from our terminal.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_command_sizes_context_and_prefix_cache() {
        assert_eq!(serve_command("flm", "gemma4-it:e2b", Some(8192)), ["flm", "serve", "gemma4-it:e2b", "--ctx-len", "8192"]);
        assert_eq!(serve_command("flm", "qwen3.5:9b", None), ["flm", "serve", "qwen3.5:9b", "--prefix-cache-mb", "1024"]);
    }

    #[test]
    fn model_sizes_and_picks() {
        assert_eq!(Model::new("gemma4-it:e4b", true, &[]).size_b(), 4.0);
        assert_eq!(Model::new("qwen3.6-moe:35b-a3b", true, &[]).size_b(), 35.0);
        assert_eq!(Model::new("embed-gemma:300m", true, &[]).size_b(), 0.3);
        let models = vec![
            Model::new("gemma4-it:e2b", true, &["vision", "tool-calling"]),
            Model::new("qwen3.5:9b", true, &["vision", "reasoning", "tool-calling"]),
            Model::new("qwen3.6-moe:35b-a3b", true, &["tool-calling"]),
            Model::new("gemma4e-flash:e4b", true, &["vision", "single-turn"]),
            Model::new("gemma3:1b", true, &[]),
        ];
        assert_eq!(pick_smarter_model(&models, "gemma4-it:e2b", Some(16.0)).as_deref(), Some("qwen3.5:9b"));
        assert_eq!(pick_smarter_model(&models, "qwen3.5:9b", Some(16.0)), None);
        assert_eq!(pick_vision_model(&models, "gemma3:1b", Some(16.0)).as_deref(), Some("gemma4-it:e2b"));
    }

    #[test]
    fn sse_merges_tool_call_deltas() {
        let mut p = SseParser::default();
        let lines = [
            r#"data: {"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"get_battery","arguments":"{\"a\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":": 1}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ];
        let mut events = Vec::new();
        for l in lines {
            events.extend(p.feed(l).unwrap());
        }
        assert!(p.finished);
        events.push(p.finish());
        assert_eq!(events[0], StreamEvent::Reasoning("hmm".into()));
        assert_eq!(events[1], StreamEvent::Text("Hi".into()));
        let StreamEvent::Done { tool_calls, finish_reason } = &events[2] else { panic!() };
        assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(tool_calls[0].name, "get_battery");
        assert_eq!(tool_calls[0].parsed_arguments().unwrap()["a"], 1);
    }

    #[test]
    fn sse_error_and_stream() {
        let mut p = SseParser::default();
        assert_eq!(p.feed(r#"data: {"error": {"message": "model not found"}}"#).unwrap_err().message, "model not found");
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\ndata: [DONE]\n\n";
        let events: Vec<_> = ChatStream::from_reader(std::io::Cursor::new(body.to_string())).map(Result::unwrap).collect();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[2], StreamEvent::Done { tool_calls, .. } if tool_calls.is_empty()));
    }

    #[test]
    fn list_labels_and_think() {
        let list = "Models:\n  - gemma4-it:e2b ✅\n  - qwen3.5:9b ⏬\n";
        assert_eq!(parse_flm_list(list), vec![("gemma4-it:e2b".into(), true), ("qwen3.5:9b".into(), false)]);
        let labels = parse_model_labels(r#"{"models": {"qwen3.5": {"9b": {"label": ["vision", "tool-calling"]}}}}"#);
        assert_eq!(labels["qwen3.5:9b"], ["vision", "tool-calling"]);
        assert_eq!(strip_think("<think>plan</think>  Answer"), "Answer");
        assert_eq!(strip_think("Hi <think>unfinished"), "Hi ");
        let call = ToolCall { arguments: "[1]".into(), ..Default::default() };
        assert!(call.parsed_arguments().is_err());
    }
}
