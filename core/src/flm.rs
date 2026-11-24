//! FastFlowLM (FLM) client: server lifecycle, model list, streaming chat.
//!
//! FLM serves an OpenAI-compatible API on 127.0.0.1:52625. It keeps one model
//! loaded and swaps to whichever model a request names.
//!
//! When Lemonade Server is installed (its `lemond` user service), the same
//! client talks to it instead: Lemonade runs FLM on the NPU and llama.cpp on
//! the graphics chip, so a bigger model can run there next to the small one
//! instead of replacing it. Models keep FLM's names ("gemma4-it:e2b"); the
//! graphics-chip ones are "gpu:<Lemonade's name>".

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Map, Value, json};

pub const DEFAULT_URL: &str = "http://127.0.0.1:52625";
pub const LEMONADE_URL: &str = "http://127.0.0.1:13305";
const LEMOND_UNIT: &str = "lemond.service";
/// Models Lemonade runs with llama.cpp on the graphics chip: "gpu:Qwen3.5-9B-GGUF".
pub const GPU_PREFIX: &str = "gpu:";

/// What serves the models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `flm serve`, which this service starts and stops.
    Flm,
    /// Lemonade Server (FLM on the NPU, llama.cpp on the graphics chip).
    Lemonade,
}

/// An FLM tag in plain words: "gemma4-it:e2b" -> "Gemma 4 E2B", "qwen3.5:9b" ->
/// "Qwen 3.5 9B"; anything else as it is.
pub fn pretty_model(tag: &str) -> String {
    if tag.starts_with(GPU_PREFIX) {
        return tag.trim_start_matches(GPU_PREFIX).to_string();
    }
    let Some((family, size)) = tag.split_once(':') else {
        return tag.to_string();
    };
    let family = family.trim_end_matches("-it").replace('-', " ");
    let split = family
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(family.len());
    let (word, version) = family.split_at(split);
    let mut name: String = word
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    name.push_str(&word[word.chars().next().map_or(0, char::len_utf8)..]);
    if !version.is_empty() {
        name.push(' ');
        name.push_str(version.trim());
    }
    format!("{} {}", name.trim(), size.to_uppercase())
}

/// One line on what the assistant runs on, for questions about it ("what API
/// does Lemonade advertise?").
pub fn self_info(backend: Backend, url: &str) -> String {
    let url = url.replace("127.0.0.1", "localhost");
    match backend {
        Backend::Lemonade => format!(
            "You run locally through Lemonade Server (OpenAI-compatible API at {url}/api/v1); \
             FastFlowLM runs the models on the NPU."
        ),
        Backend::Flm => format!(
            "You run locally on FastFlowLM, which runs the models on the NPU \
             (OpenAI-compatible API at {url}/v1)."
        ),
    }
}

/// Lemonade's name for an FLM model: "gemma4-it:e2b" -> "gemma4-it-e2b-FLM".
pub fn lemonade_flm_name(tag: &str) -> String {
    format!("{}-FLM", tag.replace(':', "-"))
}

/// Our model list from Lemonade's /v1/models: FLM models by their FLM tag,
/// downloaded llama.cpp chat models as "gpu:<name>". Also returns Lemonade's
/// name for each.
pub fn parse_lemonade_models(
    v: &Value,
    flm_labels: &BTreeMap<String, Vec<String>>,
) -> Vec<(Model, String)> {
    let mut out = Vec::new();
    for m in v["data"].as_array().into_iter().flatten() {
        let id = m["id"].as_str().unwrap_or_default();
        let downloaded = m["downloaded"].as_bool().unwrap_or(false);
        let labels: Vec<String> = m["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().map(String::from))
            .collect();
        let main = m["checkpoints"]["main"]
            .as_str()
            .or(m["checkpoint"].as_str())
            .unwrap_or_default();
        match m["recipe"].as_str().unwrap_or_default() {
            "flm" if !main.is_empty() => out.push((
                Model {
                    name: main.to_string(),
                    installed: downloaded,
                    labels: flm_labels.get(main).cloned().unwrap_or(labels),
                },
                id.to_string(),
            )),
            // Chat models only, and only downloaded ones: the catalog is long.
            "llamacpp"
                if downloaded
                    && !labels.iter().any(|l| {
                        ["embeddings", "embedding", "reranking"].contains(&l.as_str())
                    }) =>
            {
                out.push((
                    Model {
                        name: format!("{GPU_PREFIX}{id}"),
                        installed: true,
                        labels,
                    },
                    id.to_string(),
                ))
            }
            _ => {}
        }
    }
    out
}
pub const DEFAULT_CTX_LEN: u32 = 8192;
/// The chat model's context until the user picks one: room for every tool.
pub const DEFAULT_CHAT_CTX_LEN: u32 = 16384;
/// From this context on, every tool goes with every request, in the same order:
/// FLM then reuses the instructions and tools it already read (0.65 s to the
/// first token instead of 4.3 s on the laptop), with the same accuracy as
/// routing (eval 44920114036: 91.3% vs 91.1%). Below it, tools are routed and
/// the embedding model helps choose them.
pub const ALL_TOOLS_CTX_LEN: u32 = 16384;
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
        Self {
            message: message.into(),
            status: None,
        }
    }
}

impl std::fmt::Display for FlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FlmError {}

/// The model used until the user picks one: the fastest small model that
/// does well in the eval (Gemma 4 E2B reads prompts ~3.5x faster than Qwen 3.5
/// 4B on the NPU and reuses what it read), then the others in order, whichever
/// is installed first.
pub const DEFAULT_MODEL: &str = "gemma4-it:e2b";
pub const PREFERRED_MODELS: [&str; 4] =
    [DEFAULT_MODEL, "gemma4-it:e4b", "qwen3.5:4b", "qwen3.5:2b"];

/// The default for this machine: the first preferred model that's installed,
/// else any installed one that can call tools, else Gemma 4 E2B (the small default).
pub fn default_model(installed: &[Model]) -> String {
    PREFERRED_MODELS
        .iter()
        .find(|p| installed.iter().any(|m| m.name == **p && m.installed))
        .map(|s| s.to_string())
        .or_else(|| {
            installed
                .iter()
                .find(|m| m.installed && m.can_use_tools() && !m.single_turn() && !m.on_gpu())
                .map(|m| m.name.clone())
        })
        .unwrap_or_else(|| "gemma4-it:e2b".into())
}

/// Sampling and constraints for one request.
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// "required" makes the server's constrained decoding produce a tool call
    /// (FastFlowLM 1.0.10+, llama.cpp); servers without it just ignore it.
    pub tool_choice: Option<Value>,
    /// FLM's per-request thinking switch: false answers (and calls tools) without
    /// reasoning first, the biggest cost between tool calls on Qwen 3.5.
    pub think: Option<bool>,
}

/// The embedding model FLM serves next to the chat model, when installed.
pub const EMBED_MODEL: &str = "embed-gemma:300m";

/// The `flm serve` command. `version` is FLM's (major, minor, patch) when
/// known: 1.0.11 and later cache prompts for most models by default (8
/// slots, 4 GiB), better than the 1 GiB override older ones needed for Qwen.
/// `extras` false leaves out the optional flags (for an FLM that rejects them).
pub fn serve_command(
    flm: &str,
    model: &str,
    ctx_len: Option<u32>,
    embed: Option<&str>,
    version: Option<(u32, u32, u32)>,
    extras: bool,
) -> Vec<String> {
    let mut cmd = vec![flm.to_string(), "serve".into(), model.to_string()];
    if let Some(ctx) = ctx_len {
        cmd.extend(["--ctx-len".into(), ctx.to_string()]);
    }
    if !extras {
        return cmd;
    }
    if let Some(e) = embed {
        cmd.extend(["--embed".into(), e.to_string()]);
    }
    let family = model
        .split(':')
        .next()
        .unwrap_or("")
        .split('-')
        .next()
        .unwrap_or("");
    let modern = version.is_some_and(|v| v >= (1, 0, 11));
    if PREFIX_CACHE_FAMILIES.contains(&family) && !modern {
        cmd.extend(["--prefix-cache-mb".into(), PREFIX_CACHE_MB.to_string()]);
    }
    cmd
}

/// "FLM v1.0.9" -> (1, 0, 9).
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let v = text.split_whitespace().find_map(|w| {
        w.trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| p.parse::<u32>().ok())
            .collect::<Option<Vec<u32>>>()
            .filter(|p| p.len() >= 2)
    })?;
    Some((v[0], v[1], v.get(2).copied().unwrap_or(0)))
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
        Self {
            name: name.into(),
            installed,
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
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
    /// The name to show: "Gemma 4 E2B", "Qwen3.5-9B-GGUF (graphics chip)" for a llama.cpp one.
    pub fn display_name(&self) -> String {
        match self.name.strip_prefix(GPU_PREFIX) {
            Some(id) => format!("{id} (graphics chip)"),
            None => pretty_model(&self.name),
        }
    }
    /// Runs on the graphics chip (through Lemonade), not the NPU.
    pub fn on_gpu(&self) -> bool {
        self.name.starts_with(GPU_PREFIX)
    }
    /// Rough parameter count in billions from the tag ("e4b" -> 4, "35b-a3b" -> 35).
    pub fn size_b(&self) -> f64 {
        static SIZE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^e?(\d+(?:\.\d+)?)([bm])").unwrap());
        // "gpu:Gemma-4-26B-A4B-it-GGUF" -> 26, "gpu:Gemma-4-E2B-it-GGUF" -> 2.
        static GPU_SIZE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)(?:^|[-_.])e?(\d+(?:\.\d+)?)b(?:[-_.]|$)").unwrap());
        if let Some(id) = self.name.strip_prefix(GPU_PREFIX) {
            return GPU_SIZE
                .captures(id)
                .and_then(|c| c[1].parse().ok())
                .unwrap_or(0.0);
        }
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
        // The graphics chip only when chosen: it's heavier on the laptop.
        .filter(|m| !m.on_gpu())
        .filter(|m| m.size_b() <= limit && cur.is_none_or(|c| m.size_b() > c.size_b()))
        .max_by(|a, b| a.size_b().total_cmp(&b.size_b()))
        .map(|m| m.name.clone())
}

/// A model to hand pictures to: tool-calling and multi-turn preferred, then small.
pub fn pick_vision_model(models: &[Model], current: &str, ram_gib: Option<f64>) -> Option<String> {
    let limit = ram_limit(ram_gib);
    models
        .iter()
        .filter(|m| m.installed && m.can_see() && !m.on_gpu())
        .filter(|m| m.name != current && m.size_b() <= limit)
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
        && let Some(dir) = real.parent()
    {
        candidates.push(dir.join("model_list.json"));
    }
    for dir in [
        "/usr/share/flm",
        "/usr/share/fastflowlm",
        "/usr/lib64/flm",
        "/usr/lib/flm",
        "/usr/lib64/fastflowlm",
        "/usr/lib/fastflowlm",
        "/opt/fastflowlm",
        "/opt/flm",
    ] {
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
    let Ok(config) = serde_json::from_str::<Value>(json_text) else {
        return out;
    };
    if let Some(models) = config.get("models").and_then(Value::as_object) {
        for (family, sizes) in models {
            for (size, info) in sizes.as_object().into_iter().flatten() {
                let labels = info
                    .get("label")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
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
        .filter_map(|l| {
            LINE.captures(l)
                .map(|c| (c[1].to_string(), c[2].contains('✅')))
        })
        .collect()
}

/// Drop inline <think> blocks some models leave in the answer text.
pub fn strip_think(text: &str) -> String {
    static THINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<think>.*?(</think>|$)").unwrap());
    THINK.replace_all(text, "").trim_start().to_string()
}

fn log_tail(path: &Path, lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
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
    std::env::split_paths(&path)
        .map(|d| d.join(cmd))
        .find(|p| p.is_file())
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
    Done {
        tool_calls: Vec<ToolCall>,
        finish_reason: Option<String>,
        usage: Option<Usage>,
    },
}

/// Token counts the server reports for one request (`stream_options.include_usage`).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    /// Of the prompt, the tokens the server reused from its prefix cache.
    pub cached_tokens: u64,
    pub completion_tokens: u64,
}

/// Turns OpenAI chat-completion SSE lines into events. Tool call deltas are
/// merged by index (only a call's first delta has its id and name) and
/// reported once, in the final `Done`.
#[derive(Default)]
pub struct SseParser {
    calls: BTreeMap<i64, ToolCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    pub finished: bool,
}

impl SseParser {
    pub fn feed(&mut self, raw: &str) -> Result<Vec<StreamEvent>, FlmError> {
        let mut out = Vec::new();
        let line = raw.trim();
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Ok(out);
        };
        if data == "[DONE]" {
            self.finished = true;
            return Ok(out);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return Ok(out);
        };
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            let n = |v: &Value| v.as_u64().unwrap_or(0);
            self.usage = Some(Usage {
                prompt_tokens: n(&u["prompt_tokens"]),
                cached_tokens: n(&u["prompt_tokens_details"]["cached_tokens"]),
                completion_tokens: n(&u["completion_tokens"]),
            });
        }
        let choices = chunk.get("choices").and_then(Value::as_array);
        if let (Some(err), true) = (chunk.get("error"), choices.is_none_or(|c| c.is_empty())) {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| err.to_string());
            return Err(FlmError::new(msg));
        }
        for choice in choices.into_iter().flatten() {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(r) = delta
                .get("reasoning_content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(StreamEvent::Reasoning(r.to_string()));
            }
            if let Some(t) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(StreamEvent::Text(t.to_string()));
            }
            for tc in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let idx = tc.get("index").and_then(Value::as_i64).unwrap_or(0);
                let func = tc.get("function").cloned().unwrap_or(Value::Null);
                let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                let call = self.calls.entry(idx).or_insert_with(|| ToolCall {
                    id: tc
                        .get("id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or(format!("call_{idx}")),
                    name: String::new(),
                    arguments: String::new(),
                });
                if !name.is_empty() {
                    call.name = name.to_string();
                }
                call.arguments
                    .push_str(func.get("arguments").and_then(Value::as_str).unwrap_or(""));
            }
        }
        Ok(out)
    }

    pub fn finish(&mut self) -> StreamEvent {
        self.finished = true;
        StreamEvent::Done {
            tool_calls: std::mem::take(&mut self.calls).into_values().collect(),
            finish_reason: self.finish_reason.take(),
            usage: self.usage.take(),
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
        Self {
            reader: Some(Box::new(BufReader::new(reader))),
            parser: SseParser::default(),
            pending: Default::default(),
            done: false,
        }
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
                    return Some(Err(FlmError::new(format!(
                        "Lost the connection to FastFlowLM: {e}"
                    ))));
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
    base_url: Mutex<String>,
    backend: Mutex<Backend>,
    /// Lemonade's names for our models.
    wire_names: Mutex<HashMap<String, String>>,
    /// This service started Lemonade, so it stops it again when idle.
    started_lemond: std::sync::atomic::AtomicBool,
    /// Models this service had Lemonade load, and their context length.
    loaded: Mutex<BTreeSet<String>>,
    ctx_len: Mutex<Option<u32>>,
    agent: ureq::Agent,
    server: Mutex<Option<Server>>,
    /// The model FLM most likely has loaded (it swaps per request).
    pub last_model: Mutex<Option<String>>,
    /// Serve this embedding model too (set when it's installed).
    pub embed_model: Mutex<Option<String>>,
    /// This FLM wouldn't start with the optional flags (--embed, prefix cache): leave them out.
    plain_serve: std::sync::atomic::AtomicBool,
    /// Request fields this FLM refused (tool_choice, think): left out from then on.
    unsupported: Mutex<std::collections::HashSet<&'static str>>,
    version: Mutex<Option<Option<(u32, u32, u32)>>>,
    /// Add EmbeddingGemma's task prompts to inputs ourselves (llama.cpp; FLM
    /// adds them from the request's `task`).
    pub embed_prompts: bool,
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
            base_url: Mutex::new(base_url.trim_end_matches('/').to_string()),
            backend: Mutex::new(Backend::Flm),
            wire_names: Mutex::new(HashMap::new()),
            started_lemond: std::sync::atomic::AtomicBool::new(false),
            loaded: Mutex::new(BTreeSet::new()),
            ctx_len: Mutex::new(None),
            agent: config.into(),
            server: Mutex::new(None),
            last_model: Mutex::new(None),
            embed_model: Mutex::new(None),
            plain_serve: std::sync::atomic::AtomicBool::new(false),
            unsupported: Mutex::new(Default::default()),
            version: Mutex::new(None),
            embed_prompts: false,
        }
    }

    /// A client for Lemonade Server at this address.
    pub fn lemonade(base_url: &str) -> Self {
        let c = Self::new(base_url);
        *c.backend.lock().unwrap() = Backend::Lemonade;
        c
    }

    /// FLM directly, unless Lemonade is installed (or PANDA_BACKEND says).
    /// PANDA_FLM_URL points at an FLM (or any OpenAI-compatible) server.
    pub fn from_env() -> Self {
        if let Ok(url) = std::env::var("PANDA_FLM_URL") {
            return Self::new(&url);
        }
        let lemonade_url =
            std::env::var("PANDA_LEMONADE_URL").unwrap_or_else(|_| LEMONADE_URL.into());
        match std::env::var("PANDA_BACKEND").as_deref() {
            Ok("flm") => Self::new(DEFAULT_URL),
            Ok("lemonade") => Self::lemonade(&lemonade_url),
            _ if lemonade_installed() => Self::lemonade(&lemonade_url),
            _ => Self::new(DEFAULT_URL),
        }
    }

    pub fn url(&self) -> String {
        self.base_url.lock().unwrap().clone()
    }

    pub fn backend(&self) -> Backend {
        *self.backend.lock().unwrap()
    }

    fn lemonade_mode(&self) -> bool {
        self.backend() == Backend::Lemonade
    }

    /// Lemonade wouldn't start: talk to FLM directly from now on.
    fn fall_back_to_flm(&self) {
        *self.backend.lock().unwrap() = Backend::Flm;
        *self.base_url.lock().unwrap() = DEFAULT_URL.into();
        self.loaded.lock().unwrap().clear();
    }

    /// True once anything answers HTTP on the port (older FLM releases answer
    /// /api/version with an error status even when ready).
    pub fn is_up(&self) -> bool {
        let paths: &[&str] = if self.lemonade_mode() {
            &["/live"]
        } else {
            &["/api/version", "/v1/models"]
        };
        paths.iter().any(|p| {
            self.agent
                .get(&format!("{}{}", self.url(), p))
                .config()
                .timeout_global(Some(Duration::from_secs(2)))
                .build()
                .call()
                .is_ok()
        })
    }

    pub fn owns_server(&self) -> bool {
        if self.lemonade_mode() {
            return !self.loaded.lock().unwrap().is_empty();
        }
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
    /// FLM's version (asked once).
    pub fn version(&self) -> Option<(u32, u32, u32)> {
        *self.version.lock().unwrap().get_or_insert_with(|| {
            let flm = which("flm")?;
            let out = Command::new(flm)
                .arg("--version")
                .stdin(Stdio::null())
                .output()
                .ok()?;
            parse_version(&format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ))
        })
    }

    pub fn ensure_server(
        &self,
        model: &str,
        ctx_len: Option<u32>,
        wait: Duration,
    ) -> Result<bool, FlmError> {
        if self.lemonade_mode() {
            match self.lemonade_load(model, ctx_len, wait) {
                Err(e) if !self.is_up() && !model.starts_with(GPU_PREFIX) => {
                    eprintln!(
                        "assistantd: Lemonade isn't running ({}); using FastFlowLM directly",
                        e.message
                    );
                    self.fall_back_to_flm();
                }
                other => return other,
            }
        }
        match self.start_server(
            model,
            ctx_len,
            wait,
            !self.plain_serve.load(std::sync::atomic::Ordering::SeqCst),
        ) {
            // It quit at once with the optional flags: an FLM that doesn't know them. Without.
            Err(e)
                if e.message.contains("stopped while starting")
                    && !self
                        .plain_serve
                        .swap(true, std::sync::atomic::Ordering::SeqCst) =>
            {
                eprintln!(
                    "assistantd: FastFlowLM failed with the optional flags; starting it without them"
                );
                self.start_server(model, ctx_len, wait, false)
            }
            other => other,
        }
    }

    fn start_server(
        &self,
        model: &str,
        ctx_len: Option<u32>,
        wait: Duration,
        extras: bool,
    ) -> Result<bool, FlmError> {
        let version = self.version();
        let mut guard = self.server.lock().unwrap();
        if let Some(s) = guard.as_mut()
            && s.ctx != ctx_len
            && matches!(s.child.try_wait(), Ok(None))
        {
            stop_child(guard.take().unwrap().child);
        }
        if self.is_up() {
            return Ok(false);
        }
        let flm =
            which("flm").ok_or_else(|| FlmError::new("FastFlowLM (flm) is not installed."))?;
        let log_path = Self::log_path();
        if let Some(dir) = log_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| FlmError::new(format!("Can't write {}: {e}", log_path.display())))?;
        let embed = self.embed_model.lock().unwrap().clone();
        let cmd = serve_command(
            &flm.to_string_lossy(),
            model,
            ctx_len,
            embed.as_deref(),
            version,
            extras,
        );
        let mut command = Command::new(&cmd[0]);
        command
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| FlmError::new(e.to_string()))?)
            .stderr(log);
        new_session(&mut command);
        let child = command
            .spawn()
            .map_err(|e| FlmError::new(format!("Couldn't start FastFlowLM: {e}")))?;
        *guard = Some(Server {
            child,
            ctx: ctx_len,
        });
        *self.last_model.lock().unwrap() = Some(model.to_string());
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = guard.as_mut().unwrap().child.try_wait() {
                *guard = None;
                return Err(FlmError::new(format!(
                    "FastFlowLM stopped while starting (exit code {}).{}",
                    status.code().unwrap_or(-1),
                    log_tail(&log_path, 6)
                )));
            }
            if self.is_up() {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        stop_child(guard.take().unwrap().child);
        Err(FlmError::new(format!(
            "FastFlowLM didn't start within {} minutes.{}",
            wait.as_secs() / 60,
            log_tail(&log_path, 6)
        )))
    }

    /// Stop the server only if this service started it. True if one was stopped.
    pub fn stop_server(&self) -> bool {
        *self.last_model.lock().unwrap() = None;
        if self.lemonade_mode() {
            let models: Vec<String> = std::mem::take(&mut *self.loaded.lock().unwrap())
                .into_iter()
                .collect();
            for m in &models {
                let _ = self.lemonade_post(
                    "/v1/unload",
                    &json!({"model_name": self.wire_name(m)}),
                    Duration::from_secs(30),
                );
            }
            let started = self
                .started_lemond
                .swap(false, std::sync::atomic::Ordering::SeqCst);
            if started {
                systemctl_user("stop");
            }
            return started || !models.is_empty();
        }
        let Some(mut s) = self.server.lock().unwrap().take() else {
            return false;
        };
        if matches!(s.child.try_wait(), Ok(None)) {
            stop_child(s.child);
            true
        } else {
            false
        }
    }

    /// Stop every `flm serve` this user runs, not just ours: one left over
    /// from a crashed service (it runs in its own session, so it outlives
    /// us) or started by hand keeps the NPU and the port. TERM first, KILL
    /// after 5 s, then wait for the port to close. Returns how many it stopped.
    pub fn stop_all_servers(&self) -> usize {
        if self.lemonade_mode() {
            let mine = self.stop_server();
            let ran = self.is_up();
            if ran {
                systemctl_user("stop");
            }
            // Its FLM children go with it; any left keep the NPU.
            let pids = flm_serve_pids(None);
            for pid in &pids {
                // SAFETY: plain kill(2) on a pid we just read as this user's flm serve.
                unsafe { libc_kill(*pid, 9) };
            }
            return usize::from(mine || ran) + pids.len();
        }
        let mine = self.stop_server();
        let port = self
            .url()
            .rsplit(':')
            .next()
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok());
        let pids = flm_serve_pids(port);
        for pid in &pids {
            // SAFETY: plain kill(2) on a pid we just read as this user's flm serve.
            unsafe { libc_kill(*pid, 15) };
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && pids.iter().any(|p| alive(*p)) {
            std::thread::sleep(Duration::from_millis(100));
        }
        for pid in pids.iter().filter(|p| alive(**p)) {
            // SAFETY: as above; it ignored SIGTERM.
            unsafe { libc_kill(*pid, 9) };
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && self.is_up() {
            std::thread::sleep(Duration::from_millis(200));
        }
        usize::from(mine) + pids.len()
    }

    /// Stop whatever FLM is running and start it again with this model.
    pub fn restart_server(&self, model: &str, ctx_len: Option<u32>) -> Result<(), FlmError> {
        self.stop_all_servers();
        self.ensure_server(model, ctx_len, Duration::from_secs(300))
            .map(|_| ())
    }

    /// The end of FLM's log, and a hint when it points at the NPU driver
    /// (a restart of FLM can't fix that; reloading the driver or a reboot can).
    pub fn diagnose(&self) -> String {
        let tail = if self.lemonade_mode() {
            Command::new("journalctl")
                .args([
                    "--user",
                    "-u",
                    LEMOND_UNIT,
                    "-n",
                    "8",
                    "-o",
                    "cat",
                    "--no-pager",
                ])
                .stdin(Stdio::null())
                .output()
                .map(|o| {
                    format!(
                        "\n\nLemonade's log:\n{}",
                        String::from_utf8_lossy(&o.stdout).trim_end()
                    )
                })
                .unwrap_or_default()
        } else {
            log_tail(&Self::log_path(), 6)
        };
        if npu_driver_trouble(&tail) {
            format!(
                "{tail}\n\nThe NPU driver looks stuck, which restarting FastFlowLM can't fix. Reboot, or reload it: \
sudo modprobe -r amdxdna && sudo modprobe amdxdna"
            )
        } else {
            tail
        }
    }

    pub fn list_models(&self) -> Result<Vec<Model>, FlmError> {
        if self.lemonade_mode() {
            match self.lemonade_models() {
                Ok(m) => return Ok(m),
                Err(e) => eprintln!("assistantd: Lemonade's model list: {}", e.message),
            }
        }
        let flm =
            which("flm").ok_or_else(|| FlmError::new("FastFlowLM (flm) is not installed."))?;
        let out = Command::new(flm)
            .arg("list")
            .stdin(Stdio::null())
            .output()
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
            .map(|(name, inst)| Model {
                labels: labels.get(&name).cloned().unwrap_or_default(),
                name,
                installed: inst,
            })
            .collect())
    }

    pub fn chat_stream(
        &self,
        model: &str,
        messages: &[Value],
        tools: Option<&[Value]>,
        opts: &ChatOptions,
    ) -> Result<ChatStream, FlmError> {
        match self.chat_stream_once(model, messages, tools, opts) {
            // An older FLM may refuse the newer fields: drop the ones sent, remember, ask again.
            Err(e)
                if (opts.tool_choice.is_some() || opts.think.is_some())
                    && (e.status.is_some_and(|s| s >= 500)
                        || e.message.contains("tool_choice")
                        || e.message.contains("think")) =>
            {
                let mut u = self.unsupported.lock().unwrap();
                if opts.tool_choice.is_some() {
                    u.insert("tool_choice");
                }
                if opts.think.is_some() {
                    u.insert("think");
                }
                drop(u);
                eprintln!(
                    "assistantd: FastFlowLM refused tool_choice/think ({}); asking without them",
                    e.message
                );
                self.chat_stream_once(
                    model,
                    messages,
                    tools,
                    &ChatOptions {
                        tool_choice: None,
                        think: None,
                        ..opts.clone()
                    },
                )
            }
            other => other,
        }
    }

    /// Whether this server took a request field (tool_choice, think) so far.
    pub fn supports(&self, field: &str) -> bool {
        !self.unsupported.lock().unwrap().contains(field)
    }

    fn chat_stream_once(
        &self,
        model: &str,
        messages: &[Value],
        tools: Option<&[Value]>,
        opts: &ChatOptions,
    ) -> Result<ChatStream, FlmError> {
        let wire = if self.lemonade_mode() {
            // Load it with our context length (Lemonade's automatic one can be huge).
            if !self.loaded.lock().unwrap().contains(model) {
                let ctx = *self.ctx_len.lock().unwrap();
                self.lemonade_load(
                    model,
                    ctx.or(Some(DEFAULT_CTX_LEN)),
                    Duration::from_secs(300),
                )?;
            }
            self.wire_name(model)
        } else {
            model.to_string()
        };
        *self.last_model.lock().unwrap() = Some(model.to_string());
        let mut body = json!({"model": wire, "messages": messages, "stream": true,
            "stream_options": {"include_usage": true}});
        if let Some(t) = tools.filter(|t| !t.is_empty()) {
            body["tools"] = Value::Array(t.to_vec());
        }
        if let Some(t) = opts.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(n) = opts.max_tokens {
            body["max_tokens"] = json!(n);
        }
        if let Some(c) = opts
            .tool_choice
            .as_ref()
            .filter(|_| self.supports("tool_choice"))
        {
            body["tool_choice"] = c.clone();
        }
        if let Some(t) = opts.think.filter(|_| self.supports("think")) {
            body["think"] = json!(t);
            // llama.cpp's name for it.
            if model.starts_with(GPU_PREFIX) {
                body["chat_template_kwargs"] = json!({"enable_thinking": t});
            }
        }
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.url()))
            .header("Authorization", "Bearer flm")
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .send(&serde_json::to_vec(&body).unwrap_or_default()[..])
            .map_err(|e| {
                FlmError::new(format!(
                    "Can't reach the model server at {}: {e}",
                    self.url()
                ))
            })?;
        let status = resp.status().as_u16();
        let reader = resp.into_body().into_reader();
        if status >= 400 {
            let mut detail = String::new();
            let _ = reader.take(500).read_to_string(&mut detail);
            return Err(FlmError {
                message: format!("FLM returned {status}: {detail}"),
                status: Some(status),
            });
        }
        Ok(ChatStream::from_reader(reader))
    }
}

impl FlmClient {
    /// Embedding vectors for `inputs` (one per string), from /v1/embeddings.
    /// `task` is FLM's EmbeddingGemma prompt: "query" or "document".
    /// Where Lemonade runs a loaded model, and the name its backend knows it by.
    fn lemonade_backend(&self, name: &str) -> Option<(String, String)> {
        let mut text = String::new();
        self.agent
            .get(&format!("{}/v1/health", self.url()))
            .call()
            .ok()?
            .into_body()
            .into_reader()
            .take(1 << 20)
            .read_to_string(&mut text)
            .ok()?;
        lemonade_backend_in(&serde_json::from_str(&text).ok()?, name)
    }

    pub fn embed(
        &self,
        model: &str,
        inputs: &[String],
        task: &str,
    ) -> Result<Vec<Vec<f32>>, FlmError> {
        let inputs: Vec<String> = if self.embed_prompts {
            inputs
                .iter()
                .map(|x| match task {
                    "query" => format!("task: search result | query: {x}"),
                    _ => format!("title: none | text: {x}"),
                })
                .collect()
        } else {
            inputs.to_vec()
        };
        let post = |url: &str, name: &str| -> Result<(u16, String), FlmError> {
            let body = json!({"model": name, "input": inputs, "task": task});
            let resp = self
                .agent
                .post(&format!("{url}/v1/embeddings"))
                .header("Authorization", "Bearer flm")
                .header("Content-Type", "application/json")
                .config()
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .send(&serde_json::to_vec(&body).unwrap_or_default()[..])
                .map_err(|e| FlmError::new(format!("embeddings: {e}")))?;
            let status = resp.status().as_u16();
            let mut text = String::new();
            let _ = resp
                .into_body()
                .into_reader()
                .take(4 << 20)
                .read_to_string(&mut text);
            Ok((status, text))
        };
        let wire = self.wire_name(model);
        let (mut status, mut text) = post(&self.url(), &wire)?;
        // Lemonade 2026.46 loads FLM's embedding model but forwards its own name for it,
        // which FLM rejects: send the request to that backend with FLM's name instead.
        if status == 400 && self.lemonade_mode() && text.contains("Unknown embedding model") {
            let backend = self.lemonade_backend(&wire);
            if let Some((url, checkpoint)) = backend {
                (status, text) = post(url.trim_end_matches("/v1"), &checkpoint)?;
            }
        }
        if status >= 400 {
            return Err(FlmError {
                message: format!("embeddings: {status}"),
                status: Some(status),
            });
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| FlmError::new(format!("embeddings: {e}")))?;
        let vecs: Vec<Vec<f32>> = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                d["embedding"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_f64().map(|f| f as f32))
                    .collect()
            })
            .collect();
        if vecs.len() != inputs.len() || vecs.iter().any(|v| v.is_empty()) {
            return Err(FlmError::new("embeddings: unexpected answer"));
        }
        Ok(vecs)
    }
}

fn lemonade_backend_in(health: &Value, name: &str) -> Option<(String, String)> {
    let m = health["all_models_loaded"]
        .as_array()?
        .iter()
        .find(|m| m["model_name"] == name)?;
    Some((
        m["backend_url"].as_str()?.to_string(),
        m["checkpoint"].as_str()?.to_string(),
    ))
}

/// Lemonade Server.
impl FlmClient {
    /// Lemonade's name for one of our models.
    fn wire_name(&self, model: &str) -> String {
        if !self.lemonade_mode() {
            return model.to_string();
        }
        if let Some(n) = self.wire_names.lock().unwrap().get(model) {
            return n.clone();
        }
        match model.strip_prefix(GPU_PREFIX) {
            Some(id) => id.to_string(),
            None => lemonade_flm_name(model),
        }
    }

    fn lemonade_post(&self, path: &str, body: &Value, wait: Duration) -> Result<Value, FlmError> {
        let resp = self
            .agent
            .post(&format!("{}{path}", self.url()))
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(wait.max(Duration::from_secs(5))))
            .build()
            .send(&serde_json::to_vec(body).unwrap_or_default()[..])
            .map_err(|e| FlmError::new(format!("Can't reach Lemonade at {}: {e}", self.url())))?;
        let status = resp.status().as_u16();
        let mut text = String::new();
        let _ = resp
            .into_body()
            .into_reader()
            .take(1 << 20)
            .read_to_string(&mut text);
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status >= 400 {
            let detail = v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .map(String::from)
                .unwrap_or_else(|| text.chars().take(300).collect());
            return Err(FlmError {
                message: format!("Lemonade returned {status}: {detail}"),
                status: Some(status),
            });
        }
        Ok(v)
    }

    /// Start Lemonade's user service unless it runs. True if this call started it.
    fn start_lemond(&self, wait: Duration) -> Result<bool, FlmError> {
        if self.is_up() {
            return Ok(false);
        }
        let out = Command::new("systemctl")
            .args(["--user", "start", LEMOND_UNIT])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| FlmError::new(format!("Couldn't start Lemonade: {e}")))?;
        if !out.status.success() {
            return Err(FlmError::new(format!(
                "Lemonade didn't start: {}{}",
                String::from_utf8_lossy(&out.stderr).trim(),
                self.diagnose()
            )));
        }
        self.started_lemond
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let deadline = Instant::now() + wait.min(Duration::from_secs(90));
        while Instant::now() < deadline {
            if self.is_up() {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(FlmError::new(format!(
            "Lemonade didn't answer after starting.{}",
            self.diagnose()
        )))
    }

    /// Have Lemonade load this model (and the embedding model) with our
    /// context length. True if this call started Lemonade.
    fn lemonade_load(
        &self,
        model: &str,
        ctx_len: Option<u32>,
        wait: Duration,
    ) -> Result<bool, FlmError> {
        let started = self.start_lemond(wait)?;
        if self.wire_names.lock().unwrap().is_empty() {
            let _ = self.lemonade_models();
        }
        let gpu = model.starts_with(GPU_PREFIX);
        // One model on the graphics chip at a time: free the other first, so
        // making room doesn't push the NPU model out.
        if gpu {
            let others: Vec<String> = self
                .loaded
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.starts_with(GPU_PREFIX) && *m != model)
                .cloned()
                .collect();
            for m in others {
                self.unload_model(&m);
            }
        }
        let mut body = json!({"model_name": self.wire_name(model)});
        if let Some(c) = ctx_len {
            body["ctx_size"] = json!(c);
        }
        self.lemonade_post("/v1/load", &body, wait)?;
        let embed = self.embed_model.lock().unwrap().clone();
        {
            let mut loaded = self.loaded.lock().unwrap();
            // Lemonade keeps one FLM chat model on the NPU: loading this one dropped the other.
            if !gpu {
                loaded.retain(|m| m.starts_with(GPU_PREFIX) || Some(m) == embed.as_ref());
            }
            loaded.insert(model.to_string());
        }
        if !gpu {
            *self.ctx_len.lock().unwrap() = ctx_len;
        }
        *self.last_model.lock().unwrap() = Some(model.to_string());
        if let Some(e) = embed.filter(|e| !self.loaded.lock().unwrap().contains(e))
            && self
                .lemonade_post(
                    "/v1/load",
                    &json!({"model_name": self.wire_name(&e)}),
                    Duration::from_secs(120),
                )
                .is_ok()
        {
            self.loaded.lock().unwrap().insert(e);
        }
        Ok(started)
    }

    /// Free one model Lemonade loaded for us, keeping the others.
    pub fn unload_model(&self, model: &str) {
        if !self.lemonade_mode() {
            return;
        }
        if self.loaded.lock().unwrap().remove(model) {
            let _ = self.lemonade_post(
                "/v1/unload",
                &json!({"model_name": self.wire_name(model)}),
                Duration::from_secs(30),
            );
        }
    }

    /// Whether this backend can keep a second model loaded next to the default
    /// one (Lemonade, with the second on the graphics chip).
    pub fn keeps_both(&self, model: &str) -> bool {
        self.lemonade_mode() && model.starts_with(GPU_PREFIX)
    }

    fn lemonade_models(&self) -> Result<Vec<Model>, FlmError> {
        self.start_lemond(Duration::from_secs(60))?;
        let resp = self
            .agent
            .get(&format!("{}/v1/models?show_all=true", self.url()))
            .config()
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .call()
            .map_err(|e| FlmError::new(format!("Can't reach Lemonade: {e}")))?;
        let mut text = String::new();
        let _ = resp
            .into_body()
            .into_reader()
            .take(16 << 20)
            .read_to_string(&mut text);
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| FlmError::new(format!("Lemonade's model list: {e}")))?;
        let list = parse_lemonade_models(&v, &model_labels());
        if list.is_empty() {
            return Err(FlmError::new("Lemonade lists no models"));
        }
        let mut names = self.wire_names.lock().unwrap();
        for (m, id) in &list {
            names.insert(m.name.clone(), id.clone());
        }
        Ok(list.into_iter().map(|(m, _)| m).collect())
    }
}

/// Lemonade's user service is installed and its server runs on this system.
pub fn lemonade_installed() -> bool {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".config"));
    let unit = [
        PathBuf::from("/usr/lib/systemd/user"),
        PathBuf::from("/etc/systemd/user"),
        config.join("systemd/user"),
    ]
    .iter()
    .any(|d| d.join(LEMOND_UNIT).exists());
    unit && which("lemond").is_some_and(|p| {
        Command::new(p)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn systemctl_user(action: &str) {
    let _ = Command::new("systemctl")
        .args(["--user", action, LEMOND_UNIT])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Drop for FlmClient {
    fn drop(&mut self) {
        self.stop_server();
    }
}

/// Errors that mean the server, not the request, is broken.
pub fn server_trouble(e: &FlmError) -> bool {
    let m = e.message.to_lowercase();
    e.status.is_some_and(|s| s >= 500)
        || m.contains("can't reach fastflowlm")
        || m.contains("connection")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("stopped while")
        || m.contains("broken pipe")
        || m.contains("unexpected end")
        || m.contains("reset by peer")
}

fn npu_driver_trouble(log: &str) -> bool {
    let l = log.to_lowercase();
    [
        "amdxdna",
        "xrt",
        "/dev/accel",
        "hwctx",
        "failed to open device",
        "no npu",
        "device busy",
        "ert_cmd",
        "aie",
    ]
    .iter()
    .any(|w| l.contains(w))
        && (l.contains("error") || l.contains("fail") || l.contains("timeout"))
}

fn own_uid() -> Option<String> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1).map(String::from))
}

/// The port an `flm serve` command line listens on (FLM's default when not given).
fn serve_port(args: &[String]) -> u16 {
    args.windows(2)
        .find(|w| w[0] == "--port" || w[0] == "-p")
        .and_then(|w| w[1].parse().ok())
        .or_else(|| {
            args.iter()
                .find_map(|a| a.strip_prefix("--port=")?.parse().ok())
        })
        .unwrap_or(52625)
}

/// This user's `flm serve` processes on `port` (one serving elsewhere, for other work, is left alone).
fn flm_serve_pids(port: Option<u16>) -> Vec<i32> {
    let Some(uid) = own_uid() else { return vec![] };
    let me = std::process::id() as i32;
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            let status = fs::read_to_string(e.path().join("status")).ok()?;
            let owner = status
                .lines()
                .find(|l| l.starts_with("Uid:"))?
                .split_whitespace()
                .nth(1)?
                .to_string();
            let cmd = fs::read(e.path().join("cmdline")).ok()?;
            let args: Vec<String> = cmd
                .split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).to_string())
                .collect();
            let is_flm = args
                .first()
                .is_some_and(|a| Path::new(a).file_name().is_some_and(|n| n == "flm"));
            let ours = port.is_none_or(|p| serve_port(&args) == p);
            (pid != me
                && owner == uid
                && is_flm
                && args.get(1).is_some_and(|a| a == "serve")
                && ours)
                .then_some(pid)
        })
        .collect()
}

fn alive(pid: i32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        && !fs::read_to_string(format!("/proc/{pid}/status"))
            .unwrap_or_default()
            .contains("State:\tZ")
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
    fn model_names_in_plain_words() {
        assert_eq!(pretty_model("gemma4-it:e2b"), "Gemma 4 E2B");
        assert_eq!(pretty_model("qwen3.5:9b"), "Qwen 3.5 9B");
        assert_eq!(pretty_model("embed-gemma:300m"), "Embed gemma 300M");
        assert_eq!(pretty_model("gpu:Qwen3.5-9B-GGUF"), "Qwen3.5-9B-GGUF");
        assert_eq!(pretty_model("plain"), "plain");
    }

    #[test]
    fn self_info_names_the_backend_and_its_address() {
        let l = self_info(Backend::Lemonade, LEMONADE_URL);
        assert!(
            l.contains("Lemonade Server") && l.contains("http://localhost:13305/api/v1"),
            "{l}"
        );
        let f = self_info(Backend::Flm, DEFAULT_URL);
        assert!(
            f.contains("FastFlowLM") && f.contains("http://localhost:52625/v1"),
            "{f}"
        );
    }

    #[test]
    fn lemonade_backend_for_embeddings() {
        let health = json!({"all_models_loaded": [
            {"model_name": "gemma4-it-e2b-FLM", "backend_url": "http://127.0.0.1:8002/v1", "checkpoint": "gemma4-it:e2b"},
            {"model_name": "embed-gemma-300m-FLM", "backend_url": "http://127.0.0.1:8001/v1", "checkpoint": "embed-gemma:300m"}]});
        assert_eq!(
            lemonade_backend_in(&health, "embed-gemma-300m-FLM"),
            Some(("http://127.0.0.1:8001/v1".into(), "embed-gemma:300m".into()))
        );
        assert_eq!(lemonade_backend_in(&health, "other"), None);
    }

    #[test]
    fn lemonade_models_by_flm_tag_and_gpu_name() {
        let v = json!({"data": [
            {"id": "gemma4-it-e2b-FLM", "recipe": "flm", "downloaded": true,
             "checkpoint": "gemma4-it:e2b", "labels": ["chat"]},
            {"id": "qwen3.5-9b-FLM", "recipe": "flm", "downloaded": false,
             "checkpoints": {"main": "qwen3.5:9b"}, "labels": ["chat", "tool-calling"]},
            {"id": "Qwen3.5-9B-GGUF", "recipe": "llamacpp", "downloaded": true,
             "labels": ["chat", "vision", "tool-calling"]},
            {"id": "Gemma-4-31B-it-GGUF", "recipe": "llamacpp", "downloaded": false,
             "labels": ["chat", "tool-calling"]},
            {"id": "nomic-embed-text-v1-GGUF", "recipe": "llamacpp", "downloaded": true,
             "labels": ["embeddings"]},
            {"id": "Whisper", "recipe": "whispercpp", "downloaded": true, "labels": ["transcription"]}
        ]});
        let mut flm = BTreeMap::new();
        flm.insert(
            "gemma4-it:e2b".to_string(),
            vec!["tool-calling".to_string(), "vision".to_string()],
        );
        let got = parse_lemonade_models(&v, &flm);
        let names: Vec<(&str, bool, &str)> = got
            .iter()
            .map(|(m, id)| (m.name.as_str(), m.installed, id.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("gemma4-it:e2b", true, "gemma4-it-e2b-FLM"),
                ("qwen3.5:9b", false, "qwen3.5-9b-FLM"),
                ("gpu:Qwen3.5-9B-GGUF", true, "Qwen3.5-9B-GGUF"),
            ]
        );
        // FLM's own labels win for FLM models; Lemonade's for the others.
        assert!(got[0].0.can_see() && got[0].0.can_use_tools());
        assert!(got[2].0.can_use_tools() && got[2].0.on_gpu());
        assert_eq!(lemonade_flm_name("qwen3.5:4b"), "qwen3.5-4b-FLM");
    }

    #[test]
    fn graphics_chip_models_only_when_chosen() {
        let models = vec![
            Model::new("gemma4-it:e2b", true, &["tool-calling", "vision"]),
            Model::new("qwen3.5:4b", true, &["tool-calling"]),
            Model::new("gpu:Qwen3.5-9B-GGUF", true, &["tool-calling", "vision"]),
            Model::new("gpu:Gemma-4-E2B-it-GGUF", true, &["tool-calling", "vision"]),
        ];
        assert_eq!(models[2].size_b(), 9.0);
        assert_eq!(models[3].size_b(), 2.0);
        assert_eq!(
            Model::new("gpu:Gemma-4-26B-A4B-it-GGUF", true, &[]).size_b(),
            26.0
        );
        assert_eq!(
            pick_smarter_model(&models, "gemma4-it:e2b", Some(16.0)).as_deref(),
            Some("qwen3.5:4b")
        );
        assert_eq!(
            pick_vision_model(&models, "qwen3.5:4b", Some(16.0)).as_deref(),
            Some("gemma4-it:e2b")
        );
        assert_eq!(default_model(&models[2..]), "gemma4-it:e2b");
    }

    #[test]
    fn lemonade_loads_with_our_context_then_chats_by_its_name() {
        use crate::fake_flm::{FakeFlm, text_reply};
        let fake = FakeFlm::start(vec![
            (200, "{}".into()),          // load the GPU model
            (200, text_reply("Hi.")),    // chat
            (200, text_reply("Again.")), // chat, already loaded
            (200, "{}".into()),          // load the NPU model
            (200, text_reply("Small.")),
            (200, "{}".into()), // unload the GPU model
        ]);
        let c = FlmClient::lemonade(&fake.url);
        let msgs = [json!({"role": "user", "content": "hi"})];
        let opts = ChatOptions {
            think: Some(false),
            ..Default::default()
        };
        for _ in 0..2 {
            let _: Vec<_> = c
                .chat_stream("gpu:Qwen3.5-9B-GGUF", &msgs, None, &opts)
                .unwrap()
                .collect();
        }
        let _: Vec<_> = c
            .chat_stream("gemma4-it:e2b", &msgs, None, &opts)
            .unwrap()
            .collect();
        assert!(c.owns_server() && c.keeps_both("gpu:Qwen3.5-9B-GGUF"));
        c.unload_model("gpu:Qwen3.5-9B-GGUF");
        let r = fake.requests();
        assert_eq!(
            r[0],
            json!({"model_name": "Qwen3.5-9B-GGUF", "ctx_size": DEFAULT_CTX_LEN})
        );
        assert_eq!(r[1]["model"], "Qwen3.5-9B-GGUF");
        assert_eq!(
            r[1]["chat_template_kwargs"],
            json!({"enable_thinking": false})
        );
        assert_eq!(r[2]["model"], "Qwen3.5-9B-GGUF");
        assert_eq!(r[3]["model_name"], "gemma4-it-e2b-FLM");
        assert_eq!(r[4]["model"], "gemma4-it-e2b-FLM");
        assert!(r[4].get("chat_template_kwargs").is_none());
        assert_eq!(r[5], json!({"model_name": "Qwen3.5-9B-GGUF"}));
        assert_eq!(r.len(), 6);
        // Dropping the client unloads what's left; nothing more is queued.
        std::mem::forget(c);
    }

    #[test]
    fn serve_command_sizes_context_and_prefix_cache() {
        assert_eq!(
            serve_command("flm", "gemma4-it:e2b", Some(8192), None, None, true),
            ["flm", "serve", "gemma4-it:e2b", "--ctx-len", "8192"]
        );
        assert_eq!(
            serve_command("flm", "qwen3.5:9b", None, None, Some((1, 0, 9)), true),
            ["flm", "serve", "qwen3.5:9b", "--prefix-cache-mb", "1024"]
        );
        assert_eq!(
            serve_command("flm", "qwen3.5:9b", None, None, Some((1, 0, 11)), true),
            ["flm", "serve", "qwen3.5:9b"],
            "FLM's own defaults"
        );
        assert_eq!(
            serve_command("flm", "gemma4-it:e2b", None, Some(EMBED_MODEL), None, true),
            [
                "flm",
                "serve",
                "gemma4-it:e2b",
                "--embed",
                "embed-gemma:300m"
            ]
        );
        assert_eq!(
            serve_command(
                "flm",
                "qwen3.5:4b",
                Some(8192),
                Some(EMBED_MODEL),
                None,
                false
            ),
            ["flm", "serve", "qwen3.5:4b", "--ctx-len", "8192"],
            "an FLM that rejects the optional flags"
        );
        assert_eq!(parse_version("FLM v1.0.9"), Some((1, 0, 9)));
        assert_eq!(parse_version("flm 1.0.11\n"), Some((1, 0, 11)));
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
        assert_eq!(
            pick_smarter_model(&models, "gemma4-it:e2b", Some(16.0)).as_deref(),
            Some("qwen3.5:9b")
        );
        assert_eq!(pick_smarter_model(&models, "qwen3.5:9b", Some(16.0)), None);
        assert_eq!(
            pick_vision_model(&models, "gemma3:1b", Some(16.0)).as_deref(),
            Some("gemma4-it:e2b")
        );
    }

    #[test]
    fn sse_reads_the_usage_chunk() {
        let mut p = SseParser::default();
        p.feed(r#"data: {"choices":[{"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#)
            .unwrap();
        p.feed(r#"data: {"choices":[],"usage":{"prompt_tokens":2100,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":2048}}}"#)
            .unwrap();
        let StreamEvent::Done { usage, .. } = p.finish() else {
            panic!()
        };
        assert_eq!(
            usage,
            Some(Usage {
                prompt_tokens: 2100,
                cached_tokens: 2048,
                completion_tokens: 12
            })
        );
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
        let StreamEvent::Done {
            tool_calls,
            finish_reason,
            usage,
        } = &events[2]
        else {
            panic!()
        };
        assert_eq!(*usage, None);
        assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(tool_calls[0].name, "get_battery");
        assert_eq!(tool_calls[0].parsed_arguments().unwrap()["a"], 1);
    }

    #[test]
    fn sse_error_and_stream() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed(r#"data: {"error": {"message": "model not found"}}"#)
                .unwrap_err()
                .message,
            "model not found"
        );
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\ndata: [DONE]\n\n";
        let events: Vec<_> = ChatStream::from_reader(std::io::Cursor::new(body.to_string()))
            .map(Result::unwrap)
            .collect();
        assert_eq!(events.len(), 3);
        assert!(
            matches!(&events[2], StreamEvent::Done { tool_calls, .. } if tool_calls.is_empty())
        );
    }

    #[test]
    fn list_labels_and_think() {
        let installed = vec![
            Model::new("gemma4-it:e2b", false, &[]),
            Model::new("qwen3.5:4b", true, &[]),
        ];
        assert_eq!(
            default_model(&installed),
            "qwen3.5:4b",
            "the preferred one isn't downloaded"
        );
        assert_eq!(
            default_model(&[
                Model::new("qwen3.5:4b", true, &[]),
                Model::new("gemma4-it:e2b", true, &[]),
            ]),
            DEFAULT_MODEL
        );
        assert_eq!(
            default_model(&[]),
            "gemma4-it:e2b",
            "nothing listed: the small default, not a model that may be missing"
        );
        assert_eq!(
            default_model(&[Model::new("lfm2:2.6b", true, &["tool-calling"])]),
            "lfm2:2.6b"
        );
        assert!(server_trouble(&FlmError {
            message: "FLM returned 500: oops".into(),
            status: Some(500)
        }));
        assert!(server_trouble(&FlmError::new(
            "Can't reach FastFlowLM at http://127.0.0.1:52625: io: Connection refused"
        )));
        assert!(!server_trouble(&FlmError {
            message: "FLM returned 400: bad tools".into(),
            status: Some(400)
        }));
        assert!(npu_driver_trouble(
            "[XRT] ERROR: failed to open device /dev/accel/accel0"
        ));
        let a = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            (
                serve_port(&a("flm serve m")),
                serve_port(&a("flm serve m --port 9000")),
                serve_port(&a("flm serve m --port=9001"))
            ),
            (52625, 9000, 9001)
        );
        assert!(!npu_driver_trouble("loading model qwen3.5:4b ... ok"));
        let list = "Models:\n  - gemma4-it:e2b ✅\n  - qwen3.5:9b ⏬\n";
        assert_eq!(
            parse_flm_list(list),
            vec![("gemma4-it:e2b".into(), true), ("qwen3.5:9b".into(), false)]
        );
        let labels = parse_model_labels(
            r#"{"models": {"qwen3.5": {"9b": {"label": ["vision", "tool-calling"]}}}}"#,
        );
        assert_eq!(labels["qwen3.5:9b"], ["vision", "tool-calling"]);
        assert_eq!(strip_think("<think>plan</think>  Answer"), "Answer");
        assert_eq!(strip_think("Hi <think>unfinished"), "Hi ");
        let call = ToolCall {
            arguments: "[1]".into(),
            ..Default::default()
        };
        assert!(call.parsed_arguments().is_err());
    }
}
