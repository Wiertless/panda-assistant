//! Scored scenarios for picking and checking models (`assistantd --eval`).
//!
//! Each scenario is a request with stand-in tool results; it passes when the
//! model calls the expected tools, in order, with the expected arguments,
//! and its answer says what it should. The real tool schemas, prompt and
//! routing are used, nothing runs on the computer. It works against
//! FastFlowLM on the NPU or any OpenAI-compatible server (CI uses llama.cpp).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::{Agent, Callbacks};
use crate::flm::FlmClient;
use crate::tool::{Args, Tool, ToolError};

pub const SCENARIOS: &str = include_str!("../eval/scenarios.json");
const MACHINE: &str = "About this computer: Framework Laptop 13, AMD Ryzen AI 5 340 with an NPU, 16 GB memory, \
Fedora Silverblue 45 with GNOME. Now: Sunday 2026-11-15 10:00.";

#[derive(Deserialize, Clone, Default)]
pub struct Expect {
    /// One tool, or several allowed ones: "find_files|search_file_contents".
    pub tool: String,
    /// Each value must appear in the argument (any of "a|b"), ignoring case.
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
}

#[derive(Deserialize, Clone)]
pub struct Rule {
    #[serde(default)]
    pub when: serde_json::Map<String, Value>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub enum Stub {
    Always(String),
    Rules(Vec<Rule>),
}

#[derive(Deserialize, Clone)]
pub struct Scenario {
    pub name: String,
    /// Earlier messages in the same chat, asked first (their tool calls aren't judged).
    #[serde(default)]
    pub before: Vec<String>,
    pub prompt: String,
    #[serde(default)]
    pub memory: Vec<String>,
    #[serde(default)]
    pub stubs: std::collections::HashMap<String, Stub>,
    #[serde(default)]
    pub expect: Vec<Expect>,
    #[serde(default)]
    pub forbid: Vec<String>,
    /// Words the answer must contain (any of "a|b" each), ignoring case.
    #[serde(default)]
    pub answer: Vec<String>,
    #[serde(default)]
    pub no_tools: bool,
}

pub struct Outcome {
    pub name: String,
    pub passed: bool,
    pub why: String,
    pub calls: Vec<(String, Args)>,
    pub seconds: f64,
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_lowercase(),
        other => other.to_string().to_lowercase(),
    }
}

fn any_in(want: &str, have: &str) -> bool {
    want.to_lowercase().split('|').any(|w| have.contains(w.trim()))
}

fn args_match(want: &serde_json::Map<String, Value>, have: &Args) -> bool {
    want.iter().all(|(k, v)| have.get(k).is_some_and(|h| any_in(&text_of(v), &text_of(h))))
}

/// What a stand-in tool answers for these arguments.
fn stub_result(stub: Option<&Stub>, args: &Args) -> Result<String, ToolError> {
    match stub {
        None => Ok("Done.".into()),
        Some(Stub::Always(s)) => Ok(s.clone()),
        Some(Stub::Rules(rules)) => {
            let rule = rules.iter().find(|r| args_match(&r.when, args));
            match rule {
                Some(Rule { error: Some(e), .. }) => Err(ToolError(e.clone())),
                Some(Rule { result: Some(r), .. }) => Ok(r.clone()),
                _ => Ok("Done.".into()),
            }
        }
    }
}

/// Did the run do what the scenario asks?
pub fn judge(s: &Scenario, calls: &[(String, Args)], answer: &str) -> Result<(), String> {
    if s.no_tools && !calls.is_empty() {
        return Err(format!("called {} for a chat", calls[0].0));
    }
    let mut next = 0;
    for want in &s.expect {
        let names: Vec<&str> = want.tool.split('|').collect();
        match calls[next..].iter().position(|(n, a)| names.contains(&n.as_str()) && args_match(&want.args, a)) {
            Some(i) => next += i + 1,
            None => {
                let tried: Vec<String> = calls.iter().filter(|(n, _)| names.contains(&n.as_str()))
                    .map(|(n, a)| format!("{n}{}", Value::Object(a.clone()))).collect();
                return Err(if tried.is_empty() { format!("never called {}", want.tool) } else {
                    format!("wanted {}{} but got {}", want.tool, Value::Object(want.args.clone()), tried.join(", "))
                });
            }
        }
    }
    let forbid: Vec<&str> = s.forbid.iter().map(String::as_str).chain(["run_command"]).collect();
    if let Some((n, _)) = calls.iter().find(|(n, _)| forbid.contains(&n.as_str()) && !s.expect.iter().any(|e| e.tool.contains(n.as_str()))) {
        return Err(format!("called {n}, which it shouldn't"));
    }
    let lower = answer.to_lowercase();
    if let Some(w) = s.answer.iter().find(|w| !any_in(w, &lower)) {
        return Err(format!("the answer doesn't mention {w}: {}", answer.chars().take(120).collect::<String>()));
    }
    Ok(())
}

struct Quiet;
impl Callbacks for Quiet {
    fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

/// Run one scenario against a model.
pub fn run(client: &Arc<FlmClient>, model: &str, sees: bool, s: &Scenario) -> Outcome {
    let calls: Arc<Mutex<Vec<(String, Args)>>> = Arc::default();
    let (log, stubs) = (calls.clone(), Arc::new(s.stubs.clone()));
    let source = Arc::new(move || {
        crate::tools::all().into_iter().map(|t| {
            let (log, stubs, name) = (log.clone(), stubs.clone(), t.name.clone());
            Tool { run: Arc::new(move |_, args: &Args| {
                log.lock().unwrap().push((name.clone(), args.clone()));
                stub_result(stubs.get(&name), args)
            }), ..t }
        }).collect()
    });
    let mut agent = Agent::new(client.clone(), model, source);
    agent.machine = Some(MACHINE.into());
    let facts: Vec<String> = s.memory.iter().map(|f| format!("- {f}")).collect();
    agent.notes = Some(if facts.is_empty() { String::new() } else { format!("What you remember about the user:\n{}", facts.join("\n")) });
    agent.sees = Arc::new(move |_| sees);
    let started = Instant::now();
    let mut result = Ok(());
    for earlier in &s.before {
        result = agent.ask(earlier, &Quiet, &[]);
        if result.is_err() {
            break;
        }
    }
    let skip = calls.lock().unwrap().len();
    if result.is_ok() {
        result = agent.ask(&s.prompt, &Quiet, &[]);
    }
    let seconds = started.elapsed().as_secs_f64();
    let answer = agent.messages.iter().rev().find(|m| m["role"] == "assistant" && m["content"].as_str().is_some_and(|c| !c.is_empty()))
        .and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
    let calls = calls.lock().unwrap()[skip..].to_vec();
    let verdict = match result {
        Err(e) => Err(format!("request failed: {}", e.message)),
        Ok(()) => judge(s, &calls, &answer),
    };
    Outcome { name: s.name.clone(), passed: verdict.is_ok(), why: verdict.err().unwrap_or_default(), calls, seconds }
}

pub fn load(path: Option<&str>) -> Result<Vec<Scenario>, String> {
    let text = match path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
        None => SCENARIOS.to_string(),
    };
    serde_json::from_str(&text).map_err(|e| format!("scenarios: {e}"))
}

/// `assistantd --eval [--url URL] [--model M] [--scenarios FILE] [--only NAME] [--sees] [--json FILE]`
pub fn main(args: &[String]) -> i32 {
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let url = opt("--url");
    let client = Arc::new(url.as_deref().map(FlmClient::new).unwrap_or_else(FlmClient::from_env));
    let cfg = crate::service::Config::load();
    let model = opt("--model").unwrap_or(cfg.model);
    let mut scenarios = match load(opt("--scenarios").as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if let Some(only) = opt("--only") {
        scenarios.retain(|s| s.name.contains(&only));
    }
    if url.is_none() {
        // The NPU: start FastFlowLM with this model, as the service would.
        if let Err(e) = client.ensure_server(&model, Some(cfg.ctx_len), std::time::Duration::from_secs(300)) {
            eprintln!("couldn't start FastFlowLM with {model}: {}", e.message);
            return 2;
        }
    }
    let sees = args.iter().any(|a| a == "--sees");
    println!("Evaluating {model} on {} scenarios", scenarios.len());
    let mut outcomes = vec![];
    for s in &scenarios {
        let o = run(&client, &model, sees, s);
        let calls: Vec<&str> = o.calls.iter().map(|(n, _)| n.as_str()).collect();
        println!("{} {:<22} {:>5.1}s  {}{}", if o.passed { "✓" } else { "✗" }, o.name, o.seconds,
                 if calls.is_empty() { "(no tools)".into() } else { calls.join(" → ") },
                 if o.passed { String::new() } else { format!("\n    {}", o.why) });
        outcomes.push(o);
    }
    let passed = outcomes.iter().filter(|o| o.passed).count();
    let secs: f64 = outcomes.iter().map(|o| o.seconds).sum();
    println!("{model}: {passed}/{} passed, {:.1}s per request on average", outcomes.len(), secs / outcomes.len().max(1) as f64);
    if let Some(path) = opt("--json") {
        let report = json!({"model": model, "passed": passed, "total": outcomes.len(), "seconds": secs,
            "scenarios": outcomes.iter().map(|o| json!({"name": o.name, "passed": o.passed, "why": o.why, "seconds": o.seconds,
                "calls": o.calls.iter().map(|(n, a)| json!({"tool": n, "args": a})).collect::<Vec<_>>()})).collect::<Vec<_>>()});
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_flm::FakeFlm;

    fn args(v: Value) -> Args {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn scenarios_load_and_name_real_tools() {
        let names: Vec<String> = crate::tools::all().into_iter().map(|t| t.name).collect();
        for s in load(None).unwrap() {
            for e in &s.expect {
                assert!(e.tool.split('|').all(|t| names.iter().any(|n| n == t)), "{}: {}", s.name, e.tool);
            }
            for t in s.stubs.keys() {
                assert!(names.contains(t), "{}: stub for {t}", s.name);
            }
        }
    }

    #[test]
    fn judging() {
        let s: Scenario = serde_json::from_value(json!({"name": "x", "prompt": "p",
            "expect": [{"tool": "save_screenshot"}, {"tool": "send_file", "args": {"device": "jakubs-iphone"}}], "answer": ["sent|wysłan"]})).unwrap();
        let shot = ("save_screenshot".to_string(), args(json!({})));
        let wrong = ("send_file".to_string(), args(json!({"device": "iPhone (Jakub)"})));
        let right = ("send_file".to_string(), args(json!({"device": "Jakubs-iPhone", "path": "~/a.png"})));
        assert!(judge(&s, &[shot.clone(), wrong.clone(), right.clone()], "Sent it.").is_ok());
        assert!(judge(&s, &[shot.clone(), wrong.clone()], "Sent").unwrap_err().contains("wanted send_file"));
        assert!(judge(&s, &[right.clone(), shot.clone()], "Sent").is_err(), "order counts");
        assert!(judge(&s, &[shot, right], "Done!").unwrap_err().contains("doesn't mention"));
        let rules = Stub::Rules(serde_json::from_value(json!([{"when": {"device": "jakubs"}, "result": "ok"}, {"error": "nope"}])).unwrap());
        assert_eq!(stub_result(Some(&rules), &args(json!({"device": "jakubs-iphone"}))).unwrap(), "ok");
        assert_eq!(stub_result(Some(&rules), &args(json!({"device": "x"}))).unwrap_err().0, "nope");
    }

    #[test]
    fn a_run_against_a_server() {
        let flm = FakeFlm::start(vec![
            (200, crate::fake_flm::tool_reply("set_dark_mode", r#"{"enabled": true}"#, "c1")),
            (200, crate::fake_flm::text_reply("Dark mode is on.")),
        ]);
        let s = load(None).unwrap().into_iter().find(|s| s.name == "dark mode").unwrap();
        let o = run(&Arc::new(FlmClient::new(&flm.url)), "test", false, &s);
        assert!(o.passed, "{}", o.why);
        assert_eq!(o.calls[0].0, "set_dark_mode");
    }
}
