//! A small Model Context Protocol (MCP) client: other apps' tools.
//!
//! Servers are listed in ~/.config/panda-assistant/mcp.json in the common
//! `mcpServers` format (as in Claude Desktop and Zed), plus "disabled" and
//! "trust" (run that server's tools without asking). Only initialize,
//! tools/list and tools/call are implemented, over stdio or Streamable HTTP.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::paths;
use crate::tool::{Risk, Tool, ToolError};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Across all servers: each tool's schema costs context tokens.
pub const MAX_TOOLS: usize = 16;
const MAX_DESCRIPTION: usize = 240;
const MAX_RESULT: usize = 4000;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub disabled: bool,
    pub trust: bool,
}

pub fn config_path() -> std::path::PathBuf {
    paths::config_dir().join("mcp.json")
}

fn str_map(v: &Value) -> BTreeMap<String, String> {
    v.as_object().into_iter().flatten()
        .map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect()
}

pub fn load_config() -> Result<Vec<ServerConfig>, String> {
    let text = match fs::read_to_string(config_path()) {
        Ok(t) => t,
        Err(_) => return Ok(vec![]),
    };
    let raw: Value = serde_json::from_str(&text).map_err(|e| format!("{} isn't valid JSON: {e}", config_path().display()))?;
    Ok(raw["mcpServers"].as_object().into_iter().flatten().filter(|(_, s)| s.is_object()).map(|(name, s)| ServerConfig {
        name: name.clone(),
        command: s["command"].as_str().unwrap_or("").into(),
        args: s["args"].as_array().into_iter().flatten().map(|a| a.as_str().map(String::from).unwrap_or_else(|| a.to_string())).collect(),
        env: str_map(&s["env"]),
        url: s["url"].as_str().or(s["serverUrl"].as_str()).unwrap_or("").into(),
        headers: str_map(&s["headers"]),
        disabled: s["disabled"].as_bool().unwrap_or(false),
        trust: s["trust"].as_bool().unwrap_or(false),
    }).collect())
}

pub fn public_name(server: &str, tool: &str) -> String {
    let clean = |s: &str| {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                out.push(c);
            } else if !out.ends_with('_') {
                out.push('_');
            }
        }
        let t = out.trim_matches('_').to_string();
        if t.is_empty() { "x".into() } else { t }
    };
    let s: String = clean(server).chars().take(20).collect();
    format!("{s}__{}", clean(tool)).chars().take(64).collect()
}

// -- transports -----------------------------------------------------------------------

trait Transport: Send {
    fn send(&mut self, msg: &Value) -> Result<(), String>;
    fn request(&mut self, msg: &Value, timeout: Duration) -> Result<Value, String>;
    fn close(&mut self);
}

/// JSON-RPC messages, one per line, over a child process's stdin/stdout.
struct Stdio_ {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Option<Value>>,
    log: String,
}

impl Stdio_ {
    fn start(cfg: &ServerConfig) -> Result<Self, String> {
        let dir = paths::state_dir();
        let _ = fs::create_dir_all(&dir);
        let safe: String = cfg.name.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
        let log_path = dir.join(format!("mcp-{safe}.log"));
        let log = OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| e.to_string())?;
        let mut child = Command::new(&cfg.command).args(&cfg.args).envs(&cfg.env).current_dir(paths::home())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(log)
            .spawn().map_err(|e| format!("couldn't start {}: {e}", cfg.command))?;
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                    let _ = tx.send(Some(v)); // banners and other non-JSON lines are skipped
                }
            }
            let _ = tx.send(None);
        });
        Ok(Self { child, stdin, incoming: rx, log: log_path.display().to_string() })
    }
}

impl Transport for Stdio_ {
    fn send(&mut self, msg: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{msg}").and_then(|_| self.stdin.flush()).map_err(|_| "the server stopped".to_string())
    }

    fn request(&mut self, msg: &Value, timeout: Duration) -> Result<Value, String> {
        self.send(msg)?;
        loop {
            match self.incoming.recv_timeout(timeout) {
                Ok(Some(reply)) => {
                    if reply.get("id") == msg.get("id") && (reply.get("result").is_some() || reply.get("error").is_some()) {
                        return Ok(reply);
                    }
                    // The server asks something: only ping is answered.
                    if let (Some(method), Some(id)) = (reply["method"].as_str(), reply.get("id")) {
                        let answer = if method == "ping" { json!({"jsonrpc": "2.0", "id": id, "result": {}}) } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})
                        };
                        self.send(&answer)?;
                    }
                }
                Ok(None) | Err(RecvTimeoutError::Disconnected) => return Err(format!("the server stopped (see {})", self.log)),
                Err(RecvTimeoutError::Timeout) => return Err(format!("no answer within {} seconds", timeout.as_secs())),
            }
        }
    }

    fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Streamable HTTP: POST each message; the answer is JSON or an SSE stream.
struct Http {
    url: String,
    headers: BTreeMap<String, String>,
    session: Option<String>,
    agent: ureq::Agent,
}

impl Http {
    fn post(&mut self, msg: &Value, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut req = self.agent.post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        let resp = req.config().timeout_global(Some(timeout)).build().send(msg.to_string().as_bytes())
            .map_err(|e| format!("couldn't reach {}: {e}", self.url))?;
        if resp.status().as_u16() >= 400 {
            return Err(format!("HTTP {} from {}", resp.status().as_u16(), self.url));
        }
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            self.session = Some(s.to_string());
        }
        Ok(resp)
    }
}

impl Transport for Http {
    fn send(&mut self, msg: &Value) -> Result<(), String> {
        self.post(msg, Duration::from_secs(15)).map(|_| ())
    }

    fn request(&mut self, msg: &Value, timeout: Duration) -> Result<Value, String> {
        let resp = self.post(msg, timeout)?;
        let sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").contains("text/event-stream");
        let reader = resp.into_body().into_reader();
        if !sse {
            let mut text = String::new();
            reader.take(8 << 20).read_to_string(&mut text).map_err(|e| e.to_string())?;
            return serde_json::from_str(&text).map_err(|_| "the server's answer wasn't JSON".to_string());
        }
        let mut data = vec![];
        for line in BufReader::new(reader).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if let Some(d) = line.strip_prefix("data:") {
                data.push(d.trim_start().to_string());
            } else if line.trim().is_empty() && !data.is_empty() {
                let reply: Value = serde_json::from_str(&data.join("\n")).unwrap_or(Value::Null);
                data.clear();
                if reply.get("id") == msg.get("id") {
                    return Ok(reply);
                }
            }
        }
        Err("the server closed the stream without answering".into())
    }

    fn close(&mut self) {
        if let Some(s) = self.session.take() {
            let mut req = self.agent.delete(&self.url).header("Mcp-Session-Id", &s);
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            let _ = req.call();
        }
    }
}

// -- one server -------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub public_name: String,
    pub title: String,
    pub description: String,
    pub schema: Value,
    pub read_only: bool,
}

pub struct Server {
    pub cfg: ServerConfig,
    pub label: String,
    pub tools: Vec<McpTool>,
    pub error: Option<String>,
    transport: Option<Box<dyn Transport>>,
    next_id: u64,
}

impl Server {
    fn new(cfg: ServerConfig) -> Self {
        Self { label: cfg.name.clone(), cfg, tools: vec![], error: None, transport: None, next_id: 0 }
    }

    pub fn running(&self) -> bool {
        self.transport.is_some()
    }

    fn rpc(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params});
        let reply = self.transport.as_mut().ok_or("not started")?.request(&msg, timeout)?;
        if let Some(err) = reply.get("error") {
            return Err(err["message"].as_str().map(String::from).unwrap_or_else(|| err.to_string()));
        }
        Ok(reply.get("result").cloned().unwrap_or(json!({})))
    }

    fn start(&mut self) -> Result<(), String> {
        if self.running() {
            return Ok(());
        }
        self.error = None;
        if self.cfg.url.is_empty() && self.cfg.command.is_empty() {
            return Err("no command or URL".into());
        }
        self.transport = Some(if !self.cfg.url.is_empty() {
            Box::new(Http { url: self.cfg.url.clone(), headers: self.cfg.headers.clone(), session: None,
                            agent: ureq::Agent::config_builder().http_status_as_error(false).build().into() })
        } else {
            Box::new(Stdio_::start(&self.cfg)?)
        });
        let result = (|| {
            let info = self.rpc("initialize", json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                "clientInfo": {"name": "panda-assistant", "title": "Assistant", "version": env!("CARGO_PKG_VERSION")}}), START_TIMEOUT)?;
            if let Some(t) = info["serverInfo"]["title"].as_str() {
                self.label = t.to_string();
            }
            self.transport.as_mut().unwrap().send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
            self.tools = self.list_tools()?;
            Ok(())
        })();
        if result.is_err() {
            self.stop();
        }
        result
    }

    fn list_tools(&mut self) -> Result<Vec<McpTool>, String> {
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let params = cursor.as_ref().map(|c| json!({"cursor": c})).unwrap_or(json!({}));
            let result = self.rpc("tools/list", params, START_TIMEOUT)?;
            for t in result["tools"].as_array().into_iter().flatten() {
                let Some(name) = t["name"].as_str() else { continue };
                let mut schema = t.get("inputSchema").cloned().filter(Value::is_object).unwrap_or(json!({}));
                let obj = schema.as_object_mut().unwrap();
                obj.remove("$schema");
                obj.entry("type").or_insert(json!("object"));
                obj.entry("properties").or_insert(json!({}));
                let desc = t["description"].as_str().or(t["title"].as_str()).unwrap_or(name);
                let desc: String = desc.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_DESCRIPTION).collect();
                tools.push(McpTool {
                    server: self.cfg.name.clone(),
                    name: name.into(),
                    public_name: public_name(&self.cfg.name, name),
                    title: t["title"].as_str().or(t["annotations"]["title"].as_str()).unwrap_or(name).into(),
                    description: desc,
                    schema,
                    read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                });
            }
            cursor = result["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Run a tool; its text and any images (as data: URLs).
    pub fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<(String, Vec<String>), String> {
        if !self.running() {
            self.start()?;
        }
        let result = self.rpc("tools/call", json!({"name": tool, "arguments": args}), CALL_TIMEOUT)?;
        let mut texts = vec![];
        let mut images = vec![];
        for part in result["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("text") => texts.push(part["text"].as_str().unwrap_or("").to_string()),
                Some("image") if part["data"].is_string() => images.push(format!(
                    "data:{};base64,{}", part["mimeType"].as_str().unwrap_or("image/png"), part["data"].as_str().unwrap())),
                Some("resource") => texts.push(part["resource"]["text"].as_str().or(part["resource"]["uri"].as_str()).unwrap_or("").into()),
                Some("resource_link") => texts.push(format!("{} {}", part["name"].as_str().unwrap_or(""), part["uri"].as_str().unwrap_or("")).trim().into()),
                _ => {}
            }
        }
        if texts.is_empty() && !result["structuredContent"].is_null() {
            texts.push(result["structuredContent"].to_string());
        }
        let mut text = texts.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n").trim().to_string();
        if text.is_empty() {
            text = "Done.".into();
        }
        if text.chars().count() > MAX_RESULT {
            text = format!("{}…", text.chars().take(MAX_RESULT).collect::<String>());
        }
        if result["isError"].as_bool() == Some(true) {
            return Err(text);
        }
        Ok((text, images))
    }

    pub fn stop(&mut self) {
        if let Some(mut t) = self.transport.take() {
            t.close();
        }
    }
}

// -- all servers ---------------------------------------------------------------------------

/// The configured servers, started on first use.
#[derive(Default)]
pub struct Hub {
    pub servers: Mutex<BTreeMap<String, Arc<Mutex<Server>>>>,
    pub config_error: Mutex<Option<String>>,
}

impl Hub {
    pub fn new() -> Arc<Self> {
        let hub = Arc::new(Self::default());
        hub.reload();
        hub
    }

    pub fn reload(&self) {
        let configs = match load_config() {
            Ok(c) => {
                *self.config_error.lock().unwrap() = None;
                c
            }
            Err(e) => {
                *self.config_error.lock().unwrap() = Some(e);
                vec![]
            }
        };
        let mut servers = self.servers.lock().unwrap();
        let mut old = std::mem::take(&mut *servers);
        for cfg in configs {
            match old.remove(&cfg.name) {
                Some(s) if s.lock().unwrap().cfg == cfg => {
                    s.lock().unwrap().error = None; // try a failed server again
                    servers.insert(cfg.name.clone(), s);
                }
                other => {
                    if let Some(s) = other {
                        s.lock().unwrap().stop();
                    }
                    servers.insert(cfg.name.clone(), Arc::new(Mutex::new(Server::new(cfg))));
                }
            }
        }
        for s in old.values() {
            s.lock().unwrap().stop();
        }
    }

    fn enabled(&self) -> Vec<Arc<Mutex<Server>>> {
        self.servers.lock().unwrap().values().filter(|s| !s.lock().unwrap().cfg.disabled).cloned().collect()
    }

    /// Start enabled servers that aren't running, in parallel; returns problems.
    pub fn start_all(&self) -> Vec<String> {
        let todo: Vec<_> = self.enabled().into_iter()
            .filter(|s| { let s = s.lock().unwrap(); !s.running() && s.error.is_none() }).collect();
        let handles: Vec<_> = todo.into_iter().map(|s| std::thread::spawn(move || {
            let mut srv = s.lock().unwrap();
            match srv.start() {
                Ok(()) => None,
                Err(e) => {
                    srv.error = Some(e.clone());
                    Some(format!("{}: {e}", srv.cfg.name))
                }
            }
        })).collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
    }

    /// The running servers' tools as assistant tools. Tools that say they
    /// only read run straight away; the rest ask first unless trusted.
    pub fn tools(&self) -> Vec<Tool> {
        let mut out = vec![];
        for server in self.enabled() {
            let (tools, label, trust) = {
                let s = server.lock().unwrap();
                if !s.running() {
                    continue;
                }
                (s.tools.clone(), s.label.clone(), s.cfg.trust)
            };
            for t in tools {
                let srv = server.clone();
                let name = t.name.clone();
                let title = t.title.clone();
                let lbl = label.clone();
                out.push(Tool::new(&t.public_name, &format!("[{label}] {}", t.description), t.schema.clone(),
                    if t.read_only || trust { Risk::Safe } else { Risk::Confirm },
                    move |ctx, args| {
                        let (text, images) = srv.lock().unwrap().call(&name, args).map_err(ToolError)?;
                        ctx.images.lock().unwrap().extend(images);
                        Ok(text)
                    },
                    move |_| format!("{lbl}: {title}"))
                    .with_detail(|a| if a.is_empty() { String::new() } else {
                        serde_json::to_string_pretty(a).unwrap_or_default().chars().take(600).collect()
                    }));
                if out.len() >= MAX_TOOLS {
                    return out;
                }
            }
        }
        out
    }

    pub fn stop_all(&self) {
        for s in self.servers.lock().unwrap().values() {
            let mut s = s.lock().unwrap();
            s.stop();
            s.error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use crate::tool::Context;

    fn server_script() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fake_mcp_server.py").to_string()
    }

    fn write_config(servers: Value) {
        let path = config_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, json!({"mcpServers": servers}).to_string()).unwrap();
    }

    #[test]
    fn names_and_config() {
        let _h = temp_home();
        assert_eq!(public_name("My Notes!", "add.note"), "My_Notes__add_note");
        assert!(public_name(&"s".repeat(80), &"t".repeat(80)).len() <= 64);
        assert!(load_config().unwrap().is_empty());
        fs::create_dir_all(paths::config_dir()).unwrap();
        fs::write(config_path(), "{nope").unwrap();
        assert!(Hub::new().config_error.lock().unwrap().as_ref().unwrap().contains("isn't valid JSON"));
    }

    #[test]
    fn stdio_server_end_to_end() {
        let _h = temp_home();
        if crate::flm::which("python3").is_none() {
            return;
        }
        write_config(json!({"notes": {"command": "python3", "args": [server_script()]},
                            "broken": {"command": "/nonexistent/server"},
                            "off": {"command": "python3", "args": [server_script()], "disabled": true}}));
        let hub = Hub::new();
        let problems = hub.start_all();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("broken:"));
        assert!(hub.start_all().is_empty()); // failures aren't retried every message
        let tools = hub.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["notes__get_note", "notes__add_note"]);
        assert_eq!(tools[0].risk, Risk::Safe);
        assert_eq!(tools[1].risk, Risk::Confirm);
        assert!(tools[0].parameters.get("$schema").is_none());
        let ctx = Context::default();
        let args: Map<String, Value> = serde_json::from_value(json!({"title": "shop"})).unwrap();
        assert_eq!((tools[0].run)(&ctx, &args).unwrap(), "Note shop: buy milk");
        assert_eq!(ctx.images.lock().unwrap().as_slice(), ["data:image/png;base64,aGk="]);
        let missing: Map<String, Value> = serde_json::from_value(json!({"title": "missing"})).unwrap();
        assert_eq!((tools[0].run)(&ctx, &missing).unwrap_err().0, "No such note");
        assert_eq!((tools[1].summary)(&Map::new()), "Notes: add.note");
        hub.stop_all();
    }
}
