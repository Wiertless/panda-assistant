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
pub const VARIANTS: &str = include_str!("../eval/variants.json");

/// A way of running the agent to compare against the others (eval/variants.json):
/// sampling, examples, repair and the claim check on or off, another system
/// prompt, other tool descriptions.
#[derive(Deserialize, Clone, Default, Debug)]
pub struct Variant {
    #[serde(default)]
    pub about: String,
    #[serde(default = "keep", deserialize_with = "maybe_null")]
    pub tool_temperature: Option<Option<f64>>,
    pub examples: Option<bool>,
    pub repair_args: Option<bool>,
    pub nudge: Option<bool>,
    pub force_completion: Option<bool>,
    pub reply_tool: Option<bool>,
    /// "compact": the ~28 broad tools, all sent with every request (no routing).
    pub tool_set: Option<String>,
    pub system_prompt: Option<String>,
    /// Every round must call exactly one tool (reply and ask_user always offered).
    pub required_rounds: Option<bool>,
    /// Route by meaning too, when the eval has an embedding server (default yes).
    pub embed: Option<bool>,
    /// Stand-ins for the prompt fragments: web, memory, escalate, nudge.
    #[serde(default)]
    pub prompt_parts: std::collections::HashMap<String, String>,
    /// Whether the prompt lists the devices and networks (default yes).
    pub known: Option<bool>,
    #[serde(default)]
    pub descriptions: std::collections::HashMap<String, String>,
}

fn keep() -> Option<Option<f64>> {
    None
}

/// `null` means "unset it", absent means "keep the default".
fn maybe_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
    Ok(Some(Option::<f64>::deserialize(d)?))
}

pub fn variant(name: &str) -> Result<Variant, String> {
    let all: std::collections::HashMap<String, Variant> =
        serde_json::from_str(VARIANTS).map_err(|e| format!("variants: {e}"))?;
    all.get(name).cloned().ok_or_else(|| {
        let mut names: Vec<&String> = all.keys().collect();
        names.sort();
        format!(
            "no variant called {name} (variants: {})",
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}
const MACHINE: &str = "About this computer: Framework Laptop 13, AMD Ryzen AI 5 340 with an NPU, 16 GB memory, \
Fedora Silverblue 45 with GNOME. Now: Sunday 2026-11-15 10:00.";
/// The devices and networks the stand-in tools know, as the real prompt lists them.
const KNOWN: &str = "- Paired Bluetooth devices: Jakub's AirPods Pro, Sony WH-1000XM5, MX Master 3
- The user's other devices (Taildrop): jakubs-iphone (iOS), framework (linux)
- Saved networks: Home 5G, Biuro-Gosc, Phone hotspot";

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
    /// The devices and networks listed in the prompt, when not the usual ones.
    pub known: Option<String>,
    #[serde(default)]
    pub stubs: std::collections::HashMap<String, Stub>,
    #[serde(default)]
    pub expect: Vec<Expect>,
    /// Calls that must also happen, in any order relative to `expect`.
    #[serde(default)]
    pub also: Vec<Expect>,
    #[serde(default)]
    pub forbid: Vec<String>,
    /// Words the answer must contain (any of "a|b" each), ignoring case.
    #[serde(default)]
    pub answer: Vec<String>,
    #[serde(default)]
    pub no_tools: bool,
    /// The expected calls may come in any order (independent steps).
    #[serde(default)]
    pub unordered: bool,
    /// What it tests (single, chain, near-tool, enum, no-tool, ask, multi-turn…), for per-capability scores.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Other wordings of the prompt, each judged as a scenario of its own.
    #[serde(default)]
    pub paraphrases: Vec<String>,
}

pub struct Outcome {
    pub name: String,
    pub passed: bool,
    /// The expected tools were among those first offered (core or routed by
    /// the request's words): a miss when not is the router's, not the model's.
    pub routed: bool,
    pub why: String,
    pub calls: Vec<(String, Args)>,
    pub seconds: f64,
    /// Model requests, and the prompt tokens they sent (cached ones included).
    pub requests: usize,
    pub prompt_tokens: u64,
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_lowercase(),
        other => other.to_string().to_lowercase(),
    }
}

fn any_in(want: &str, have: &str) -> bool {
    let have = have.to_lowercase();
    want.to_lowercase()
        .split('|')
        .any(|w| have.contains(w.trim()))
}

fn args_match(want: &serde_json::Map<String, Value>, have: &Args) -> bool {
    want.iter().all(|(k, v)| {
        have.get(k)
            .is_some_and(|h| any_in(&text_of(v), &text_of(h)))
    })
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
                Some(Rule {
                    result: Some(r), ..
                }) => Ok(r.clone()),
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
        let fits =
            |(n, a): &(String, Args)| names.contains(&n.as_str()) && args_match(&want.args, a);
        let from = if s.unordered { 0 } else { next };
        match calls[from..].iter().position(fits) {
            Some(i) => next = from + i + 1,
            None if calls.iter().any(fits) => {
                return Err(format!(
                    "called {} too early, before the step it needs",
                    want.tool
                ));
            }
            None => {
                let tried: Vec<String> = calls
                    .iter()
                    .filter(|(n, _)| names.contains(&n.as_str()))
                    .map(|(n, a)| format!("{n}{}", Value::Object(a.clone())))
                    .collect();
                return Err(if tried.is_empty() {
                    format!("never called {}", want.tool)
                } else {
                    format!(
                        "wanted {}{} but got {}",
                        want.tool,
                        Value::Object(want.args.clone()),
                        tried.join(", ")
                    )
                });
            }
        }
    }
    for want in &s.also {
        let names: Vec<&str> = want.tool.split('|').collect();
        if !calls
            .iter()
            .any(|(n, a)| names.contains(&n.as_str()) && args_match(&want.args, a))
        {
            return Err(format!(
                "never called {}{}",
                want.tool,
                Value::Object(want.args.clone())
            ));
        }
    }
    let forbid: Vec<&str> = s
        .forbid
        .iter()
        .map(String::as_str)
        .chain(["run_command"])
        .collect();
    if let Some((n, _)) = calls.iter().find(|(n, _)| {
        forbid.contains(&n.as_str())
            && !s
                .expect
                .iter()
                .chain(&s.also)
                .any(|e| e.tool.contains(n.as_str()))
    }) {
        return Err(format!("called {n}, which it shouldn't"));
    }
    let lower = answer.to_lowercase();
    if let Some(w) = s.answer.iter().find(|w| !any_in(w, &lower)) {
        return Err(format!(
            "the answer doesn't mention {w}: {}",
            answer.chars().take(120).collect::<String>()
        ));
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
pub fn run(
    client: &Arc<FlmClient>,
    model: &str,
    sees: bool,
    v: &Variant,
    s: &Scenario,
    embedder: Option<&Arc<crate::tools::groups::Embedder>>,
) -> Outcome {
    let calls: Arc<Mutex<Vec<(String, Args)>>> = Arc::default();
    let (log, stubs, descriptions) = (
        calls.clone(),
        Arc::new(s.stubs.clone()),
        Arc::new(v.descriptions.clone()),
    );
    let compact = v.tool_set.as_deref() == Some("compact");
    let source = Arc::new(move || {
        let narrow: Vec<Tool> = crate::tools::all()
            .into_iter()
            .map(|t| {
                let (log, stubs, name) = (log.clone(), stubs.clone(), t.name.clone());
                let description = descriptions
                    .get(&t.name)
                    .cloned()
                    .unwrap_or(t.description.clone());
                Tool {
                    description,
                    run: Arc::new(move |_, args: &Args| {
                        log.lock().unwrap().push((name.clone(), args.clone()));
                        stub_result(stubs.get(&name), args)
                    }),
                    ..t
                }
            })
            .collect();
        // The broad tools forward to the narrow stand-ins, so the judging stays the same.
        if compact {
            crate::tools::compact::compact(narrow)
        } else {
            narrow
        }
    });
    let mut agent = Agent::new(client.clone(), model, source);
    if let Some(t) = v.tool_temperature {
        agent.tool_temperature = t;
    }
    agent.examples = v.examples.unwrap_or(agent.examples);
    agent.repair_args = v.repair_args.unwrap_or(agent.repair_args);
    agent.nudge = v.nudge.unwrap_or(agent.nudge);
    agent.force_completion = v.force_completion.unwrap_or(agent.force_completion);
    agent.reply_tool = v.reply_tool.unwrap_or(agent.reply_tool);
    agent.system_prompt = v.system_prompt.clone();
    agent.required_rounds = v.required_rounds.unwrap_or(agent.required_rounds);
    let embedder = embedder.filter(|_| v.embed.unwrap_or(true));
    agent.embedder = embedder.cloned();
    agent.log_rounds = false;
    agent.prompt_parts = v.prompt_parts.clone();
    if compact {
        agent.route_tools = false; // everything goes along
        agent.examples = false; // they name the narrow tools
    }
    let known = s.known.as_deref().unwrap_or(KNOWN);
    agent.machine = Some(if v.known.unwrap_or(true) {
        format!("{MACHINE}\n{known}")
    } else {
        MACHINE.into()
    });
    let facts: Vec<String> = s.memory.iter().map(|f| format!("- {f}")).collect();
    agent.notes = Some(if facts.is_empty() {
        String::new()
    } else {
        format!("What you remember about the user:\n{}", facts.join("\n"))
    });
    agent.sees = Arc::new(move |_| sees);
    let mut offered: Vec<&str> = vec![crate::tools::groups::CORE];
    for p in s.before.iter().chain([&s.prompt]) {
        offered.extend(crate::tools::groups::groups_for(p));
        if let Some(e) = embedder {
            offered.extend(e.groups_for(p));
        }
    }
    let routed = s.expect.iter().chain(&s.also).all(|e| {
        e.tool
            .split('|')
            .any(|t| offered.contains(&crate::tools::groups::group_of(t)))
    });
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
    let answer = agent
        .messages
        .iter()
        .rev()
        .find(|m| m["role"] == "assistant" && m["content"].as_str().is_some_and(|c| !c.is_empty()))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string();
    let calls = calls.lock().unwrap()[skip..].to_vec();
    let verdict = match result {
        Err(e) => Err(format!("request failed: {}", e.message)),
        Ok(()) => judge(s, &calls, &answer),
    };
    Outcome {
        name: s.name.clone(),
        passed: verdict.is_ok(),
        routed,
        why: verdict.err().unwrap_or_default(),
        calls,
        seconds,
        requests: agent.stats.len(),
        prompt_tokens: agent.stats.iter().map(|r| r.usage.prompt_tokens).sum(),
    }
}

pub fn load(path: Option<&str>) -> Result<Vec<Scenario>, String> {
    let text = match path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
        None => SCENARIOS.to_string(),
    };
    let base: Vec<Scenario> = serde_json::from_str(&text).map_err(|e| format!("scenarios: {e}"))?;
    // Each paraphrase is the same scenario in other words.
    Ok(base
        .into_iter()
        .flat_map(|s| {
            let others: Vec<Scenario> = s
                .paraphrases
                .iter()
                .enumerate()
                .map(|(i, p)| Scenario {
                    name: format!("{} ~{}", s.name, i + 2),
                    prompt: p.clone(),
                    paraphrases: vec![],
                    ..s.clone()
                })
                .collect();
            std::iter::once(Scenario {
                paraphrases: vec![],
                ..s
            })
            .chain(others)
        })
        .collect())
}

type Row = (String, usize, f64, Vec<Outcome>, Vec<String>);

/// The JSON report for the scenarios run so far, marked partial.
fn partial_report(model: &str, variant: &str, repeat: usize, rows: &[Row]) -> String {
    let runs: Vec<&Outcome> = rows.iter().flat_map(|r| r.3.iter()).collect();
    let n = runs.len().max(1) as f64;
    let report = json!({"model": model, "variant": variant, "partial": true, "repeat": repeat,
        "passed": rows.iter().map(|r| r.1).sum::<usize>(), "total": rows.len() * repeat,
        "seconds": rows.iter().map(|r| r.2).sum::<f64>(),
        "model_calls": runs.iter().map(|o| o.requests).sum::<usize>() as f64 / n,
        "prompt_tokens": runs.iter().map(|o| o.prompt_tokens).sum::<u64>() as f64 / n,
        "scenarios": rows.iter().map(|(name, ok, secs, runs, _)| {
            let shown = runs.iter().find(|o| !o.passed).unwrap_or(&runs[0]);
            json!({"name": name, "passed": *ok == repeat, "passed_runs": ok, "runs": repeat, "why": shown.why,
                   "routed": runs.iter().all(|o| o.routed), "seconds": secs,
                   "calls": shown.calls.iter().map(|(n, a)| json!({"tool": n, "args": a})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()});
    serde_json::to_string_pretty(&report).unwrap_or_default()
}

/// `assistantd --eval [--url URL] [--embed-url URL] [--model M] [--variant NAME] [--repeat N] [--scenarios FILE] [--only NAME] [--sees] [--json FILE]`
pub fn main(args: &[String]) -> i32 {
    let opt = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let url = opt("--url");
    let client = Arc::new(
        url.as_deref()
            .map(FlmClient::new)
            .unwrap_or_else(FlmClient::from_env),
    );
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
        scenarios.retain(|s| s.name.contains(&only) || s.tags.contains(&only));
    }
    if args.iter().any(|a| a == "--no-paraphrases") {
        scenarios.retain(|s| !s.name.contains(" ~"));
    }
    if url.is_none() {
        // The NPU: start FastFlowLM with this model, as the service would.
        if let Err(e) = client.ensure_server(
            &model,
            Some(cfg.ctx_len),
            std::time::Duration::from_secs(300),
        ) {
            eprintln!("couldn't start FastFlowLM with {model}: {}", e.message);
            return 2;
        }
    }
    // Routing by meaning: an embedding server next to the chat one (EmbeddingGemma).
    let embedder = opt("--embed-url").map(|u| {
        let mut c = FlmClient::new(&u);
        c.embed_prompts = true;
        Arc::new(crate::tools::groups::Embedder::new(
            Arc::new(c),
            crate::flm::EMBED_MODEL,
        ))
    });
    let sees = args.iter().any(|a| a == "--sees");
    let variant_name = opt("--variant").unwrap_or_else(|| "baseline".into());
    let v = match variant(&variant_name) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // Repeats turn one lucky or unlucky answer into a pass rate.
    let repeat: usize = opt("--repeat")
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
        .max(1);
    println!(
        "Evaluating {model} ({variant_name}: {}) on {} scenarios, {repeat}x",
        v.about,
        scenarios.len()
    );
    let mut rows: Vec<Row> = vec![];
    for s in &scenarios {
        let runs: Vec<Outcome> = (0..repeat)
            .map(|_| run(&client, &model, sees, &v, s, embedder.as_ref()))
            .collect();
        let ok = runs.iter().filter(|o| o.passed).count();
        let secs = runs.iter().map(|o| o.seconds).sum::<f64>() / repeat as f64;
        let shown = runs.iter().find(|o| !o.passed).unwrap_or(&runs[0]);
        let calls: Vec<&str> = shown.calls.iter().map(|(n, _)| n.as_str()).collect();
        let mark = if ok == repeat {
            "✓".to_string()
        } else if ok == 0 {
            "✗".to_string()
        } else {
            format!("{ok}/{repeat}")
        };
        let routing = if shown.routed {
            ""
        } else {
            " [tool not offered by routing]"
        };
        println!(
            "{mark:>3} {:<24} {secs:>5.1}s  {}{}",
            s.name,
            if calls.is_empty() {
                "(no tools)".into()
            } else {
                calls.join(" → ")
            },
            if shown.passed {
                String::new()
            } else {
                format!("\n    {}{routing}", shown.why)
            }
        );
        rows.push((s.name.clone(), ok, secs, runs, s.tags.clone()));
        // Keep a report of what's done so far: a run cut off by a time limit still has one.
        if let Some(path) = opt("--json") {
            let _ = std::fs::write(&path, partial_report(&model, &variant_name, repeat, &rows));
        }
    }
    let passed: usize = rows.iter().map(|r| r.1).sum();
    let total = rows.len() * repeat;
    let secs: f64 = rows.iter().map(|r| r.2).sum();
    let runs_all: Vec<&Outcome> = rows.iter().flat_map(|r| r.3.iter()).collect();
    let requests =
        runs_all.iter().map(|o| o.requests).sum::<usize>() as f64 / runs_all.len().max(1) as f64;
    let prompt =
        runs_all.iter().map(|o| o.prompt_tokens).sum::<u64>() as f64 / runs_all.len().max(1) as f64;
    println!(
        "{model}: {passed}/{total} passed ({:.0}%), {:.1}s per request on average, {requests:.1} model calls and \
{prompt:.0} prompt tokens per request",
        100.0 * passed as f64 / total.max(1) as f64,
        secs / rows.len().max(1) as f64
    );
    // Per capability: where it's weak, not just how much.
    let mut by_tag: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for (_, ok, _, _, tags) in &rows {
        for t in tags {
            let e = by_tag.entry(t.clone()).or_default();
            e.0 += ok;
            e.1 += repeat;
        }
    }
    let tag_line: Vec<String> = by_tag
        .iter()
        .map(|(t, (ok, n))| format!("{t} {ok}/{n}"))
        .collect();
    println!("by capability: {}", tag_line.join(", "));
    // Paraphrases of one scenario are one thing: a cluster passes when all its wordings do.
    let mut clusters: std::collections::BTreeMap<String, bool> = Default::default();
    for (name, ok, ..) in &rows {
        let base = name.split(" ~").next().unwrap_or(name).to_string();
        let all = clusters.entry(base).or_insert(true);
        *all &= *ok == repeat;
    }
    let clusters_ok = clusters.values().filter(|v| **v).count();
    let unrouted = rows
        .iter()
        .filter(|r| r.3.iter().any(|o| !o.routed))
        .count();
    println!(
        "scenarios fully passed: {clusters_ok}/{}; prompts whose tool routing didn't offer: {unrouted}",
        clusters.len()
    );
    if let Some(path) = opt("--json") {
        let report = json!({"model": model, "variant": variant_name, "passed": passed, "total": total, "repeat": repeat,
            "clusters_passed": clusters_ok, "clusters": clusters.len(), "unrouted": unrouted,
            "seconds": secs, "model_calls": requests, "prompt_tokens": prompt, "capabilities": by_tag.iter().map(|(t, (ok, n))| (t.clone(), json!({"passed": ok, "total": n})))
                .collect::<serde_json::Map<_, _>>(),
            "scenarios": rows.iter().map(|(name, ok, secs, runs, _)| {
                let shown = runs.iter().find(|o| !o.passed).unwrap_or(&runs[0]);
                json!({"name": name, "passed": *ok == repeat, "passed_runs": ok, "runs": repeat, "why": shown.why,
                       "routed": runs.iter().all(|o| o.routed),
                       "seconds": secs, "calls": shown.calls.iter().map(|(n, a)| json!({"tool": n, "args": a})).collect::<Vec<_>>()})
            }).collect::<Vec<_>>()});
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
    fn paraphrases_become_scenarios() {
        let all = load(None).unwrap();
        let dark: Vec<&Scenario> = all
            .iter()
            .filter(|s| s.name.starts_with("dark mode ~") || s.name == "dark mode")
            .collect();
        assert!(dark.len() >= 3);
        assert!(
            dark.iter()
                .all(|s| s.expect[0].tool == "set_dark_mode" && s.tags == ["single"])
        );
        assert!(
            all.iter().all(|s| !s.tags.is_empty()),
            "every scenario says what it tests"
        );
    }

    #[test]
    fn scenarios_load_and_name_real_tools() {
        let names: Vec<String> = crate::tools::all().into_iter().map(|t| t.name).collect();
        for s in load(None).unwrap() {
            for e in &s.expect {
                assert!(
                    e.tool.split('|').all(|t| names.iter().any(|n| n == t)),
                    "{}: {}",
                    s.name,
                    e.tool
                );
            }
            for t in s.stubs.keys() {
                assert!(names.contains(t), "{}: stub for {t}", s.name);
            }
        }
    }

    #[test]
    fn variants_load() {
        let bare = variant("no-harness").unwrap();
        assert_eq!(
            (bare.tool_temperature, bare.examples, bare.nudge),
            (Some(None), Some(false), Some(false))
        );
        assert_eq!(
            variant("baseline").unwrap().tool_temperature,
            None,
            "absent keeps the default"
        );
        assert!(variant("nope").unwrap_err().contains("baseline"));
    }

    #[test]
    fn judging() {
        let s: Scenario = serde_json::from_value(json!({"name": "x", "prompt": "p",
            "expect": [{"tool": "save_screenshot"}, {"tool": "send_file", "args": {"device": "jakubs-iphone"}}], "answer": ["sent|wysłan"]})).unwrap();
        let shot = ("save_screenshot".to_string(), args(json!({})));
        let wrong = (
            "send_file".to_string(),
            args(json!({"device": "iPhone (Jakub)"})),
        );
        let right = (
            "send_file".to_string(),
            args(json!({"device": "Jakubs-iPhone", "path": "~/a.png"})),
        );
        assert!(
            judge(
                &s,
                &[shot.clone(), wrong.clone(), right.clone()],
                "Sent it."
            )
            .is_ok()
        );
        assert!(
            judge(&s, &[shot.clone(), wrong.clone()], "Sent")
                .unwrap_err()
                .contains("wanted send_file")
        );
        assert!(
            judge(&s, &[right.clone(), shot.clone()], "Sent")
                .unwrap_err()
                .contains("too early"),
            "order counts"
        );
        let mut any = s.clone();
        any.unordered = true;
        assert!(
            judge(&any, &[right.clone(), shot.clone()], "Sent").is_ok(),
            "unless it doesn't"
        );
        assert!(
            judge(&s, &[shot, right], "Done!")
                .unwrap_err()
                .contains("doesn't mention")
        );
        let rules = Stub::Rules(
            serde_json::from_value(
                json!([{"when": {"device": "jakubs"}, "result": "ok"}, {"error": "nope"}]),
            )
            .unwrap(),
        );
        assert_eq!(
            stub_result(Some(&rules), &args(json!({"device": "jakubs-iphone"}))).unwrap(),
            "ok"
        );
        assert_eq!(
            stub_result(Some(&rules), &args(json!({"device": "x"})))
                .unwrap_err()
                .0,
            "nope"
        );
    }

    #[test]
    fn a_run_against_a_server() {
        let flm = FakeFlm::start(vec![
            (
                200,
                crate::fake_flm::tool_reply("set_dark_mode", r#"{"enabled": true}"#, "c1"),
            ),
            (200, crate::fake_flm::text_reply("Dark mode is on.")),
        ]);
        let s = load(None)
            .unwrap()
            .into_iter()
            .find(|s| s.name == "dark mode")
            .unwrap();
        let o = run(
            &Arc::new(FlmClient::new(&flm.url)),
            "test",
            false,
            &variant("baseline").unwrap(),
            &s,
            None,
        );
        assert!(o.passed, "{}", o.why);
        assert_eq!(o.calls[0].0, "set_dark_mode");
    }
}
