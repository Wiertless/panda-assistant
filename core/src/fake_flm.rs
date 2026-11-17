//! A tiny stand-in for FastFlowLM's OpenAI endpoint, scripted per test.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

pub fn text_reply(text: &str) -> String {
    sse(&[
        json!({"choices": [{"delta": {"content": text}}]}),
        json!({"choices": [{"delta": {"content": null}, "finish_reason": "stop"}]}),
    ])
}

pub fn tool_reply(name: &str, args: &str, id: &str) -> String {
    let half = args.len() / 2;
    sse(&[
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": id, "type": "function",
                                                      "function": {"name": name, "arguments": ""}}]}}]}),
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": &args[..half]}}]}}]}),
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": &args[half..]}}]}}]}),
        json!({"choices": [{"delta": {"content": null}, "finish_reason": "tool_calls"}]}),
    ])
}

/// Serves queued replies in order and records every request body.
pub struct FakeFlm {
    pub url: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl FakeFlm {
    pub fn start(replies: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let requests = Arc::new(Mutex::new(vec![]));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let (status, text, kind) = if first.starts_with("POST") {
                    recorded
                        .lock()
                        .unwrap()
                        .push(serde_json::from_slice(&body).unwrap_or(Value::Null));
                    let (status, text) = replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or((500, "no reply queued".into()));
                    if status == 200 {
                        (200, text, "text/event-stream")
                    } else {
                        (
                            status,
                            json!({"error": text}).to_string(),
                            "application/json",
                        )
                    }
                } else {
                    (200, r#"{"version":"test"}"#.to_string(), "application/json")
                };
                let mut out = stream;
                let _ = write!(
                    out,
                    "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        Self { url, requests }
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}
