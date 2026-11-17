//! What a tool is: a JSON schema for the model, a risk level and a function.
//!
//! "Safe" tools run straight away; "confirm" tools wait for the user to press
//! Allow. Only what can't be undone or sends something out asks.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

pub type Args = Map<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolError(pub String);

impl<T: Into<String>> From<T> for ToolError {
    fn from(s: T) -> Self {
        ToolError(s.into())
    }
}

/// Marks an error that retrying can't fix (a missing service, nothing running).
pub const CANNOT: &str = "[cannot] ";

impl ToolError {
    /// An error no other call can work around: the turn ends with it.
    pub fn cannot(msg: impl Into<String>) -> Self {
        ToolError(format!("{CANNOT}{}", msg.into()))
    }
    pub fn is_final(&self) -> bool {
        self.0.starts_with(CANNOT)
    }
    /// The message without the marker.
    pub fn text(&self) -> &str {
        self.0.strip_prefix(CANNOT).unwrap_or(&self.0)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type ToolResult = Result<String, ToolError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Confirm,
}

type Hook = Arc<dyn Fn() + Send + Sync>;
type Clipboard = Arc<dyn Fn(&str) + Send + Sync>;

/// What tools may need from the app around them.
#[derive(Default, Clone)]
pub struct Context {
    pub copy_to_clipboard: Option<Clipboard>,
    /// Step the assistant's window aside (and back) around a screenshot or window read.
    pub hide_window: Option<Hook>,
    pub show_window: Option<Hook>,
    /// Pictures a tool produced (data: URLs) for the agent to show the model.
    pub images: Arc<Mutex<Vec<String>>>,
    /// The process of the window that asked (so reading "the window" skips it).
    pub client_pid: Arc<std::sync::atomic::AtomicU32>,
    /// Reminders were added or removed: the service reschedules its timer.
    pub reminders_changed: Option<Hook>,
    /// What the last changes replaced, newest last, so "undo that" can put it back.
    pub undo: Arc<Mutex<Vec<UndoStep>>>,
    /// Ask the model a one-off question in a fresh context (system, user) ->
    /// answer, for tools that work on long text (documents, a selection).
    pub ask_model: Option<AskModel>,
}

pub type AskModel = Arc<dyn Fn(&str, &str) -> Result<String, ToolError> + Send + Sync>;

#[cfg(test)]
mod undo_tests {
    use super::*;

    #[test]
    fn undo_newest_first() {
        let ctx = Context::default();
        let log = Arc::new(Mutex::new(vec![]));
        for i in 0..25 {
            let l = log.clone();
            ctx.can_undo(&format!("change {i}"), move || {
                l.lock().unwrap().push(i);
                Ok(format!("{i} back."))
            });
        }
        let copy = ctx.clone();
        assert_eq!(
            copy.undo_last().unwrap(),
            "Undid “change 24”: 24 back.",
            "clones share the stack"
        );
        assert_eq!(ctx.undo_last().unwrap(), "Undid “change 23”: 23 back.");
        for _ in 0..18 {
            ctx.undo_last().unwrap();
        }
        assert!(
            ctx.undo_last().unwrap_err().0.contains("nothing to undo"),
            "only the last 20 are kept"
        );
    }
}

pub type UndoFn = Arc<dyn Fn() -> Result<String, ToolError> + Send + Sync>;

#[derive(Clone)]
pub struct UndoStep {
    /// What the change was ("Dark mode on"), for "Undid: …".
    pub what: String,
    pub run: UndoFn,
}

const UNDO_KEEP: usize = 20;

impl Context {
    /// Record how to reverse a change that just happened.
    pub fn can_undo(
        &self,
        what: &str,
        run: impl Fn() -> Result<String, ToolError> + Send + Sync + 'static,
    ) {
        let mut u = self.undo.lock().unwrap();
        u.push(UndoStep {
            what: what.to_string(),
            run: Arc::new(run),
        });
        let extra = u.len().saturating_sub(UNDO_KEEP);
        u.drain(..extra);
    }

    /// Reverse the newest change.
    pub fn undo_last(&self) -> Result<String, ToolError> {
        let step = self
            .undo
            .lock()
            .unwrap()
            .pop()
            .ok_or("there's nothing to undo")?;
        (step.run)().map(|done| format!("Undid “{}”: {done}", step.what.trim_end_matches('.')))
    }
}

pub type RunFn = Arc<dyn Fn(&Context, &Args) -> ToolResult + Send + Sync>;
pub type TextFn = Arc<dyn Fn(&Args) -> String + Send + Sync>;

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub risk: Risk,
    pub run: RunFn,
    pub summary: TextFn,
    /// Exactly what a confirmed action will do, shown on the Allow card.
    pub detail: Option<TextFn>,
}

impl Tool {
    pub fn new(
        name: &str,
        description: &str,
        parameters: Value,
        risk: Risk,
        run: impl Fn(&Context, &Args) -> ToolResult + Send + Sync + 'static,
        summary: impl Fn(&Args) -> String + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            risk,
            run: Arc::new(run),
            summary: Arc::new(summary),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Fn(&Args) -> String + Send + Sync + 'static) -> Self {
        self.detail = Some(Arc::new(detail));
        self
    }

    pub fn schema(&self) -> Value {
        json!({"type": "function", "function": {
            "name": self.name, "description": self.description, "parameters": self.parameters}})
    }
}

/// A JSON-schema object with these properties.
pub fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

pub fn arg_str(args: &Args, key: &str) -> String {
    match args.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

pub fn arg_bool(args: &Args, key: &str) -> Result<bool, ToolError> {
    match args.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.to_lowercase().as_str() {
            "true" | "on" | "yes" => Ok(true),
            "false" | "off" | "no" => Ok(false),
            _ => Err(format!("'{key}' must be true or false").into()),
        },
        _ => Err(format!("'{key}' must be true or false").into()),
    }
}

pub fn arg_percent(args: &Args, key: &str, low: i64) -> Result<i64, ToolError> {
    let value = match args.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .ok_or_else(|| ToolError(format!("'{key}' must be a number from {low} to 100")))?
    .round() as i64;
    if !(low..=100).contains(&value) {
        return Err(format!("'{key}' must be from {low} to 100").into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_helpers() {
        let args: Args =
            serde_json::from_value(json!({"on": "yes", "pct": "42.4", "big": 500, "n": 3}))
                .unwrap();
        assert!(arg_bool(&args, "on").unwrap());
        assert!(arg_bool(&args, "missing").is_err());
        assert_eq!(arg_percent(&args, "pct", 0).unwrap(), 42);
        assert_eq!(
            arg_percent(&args, "big", 0).unwrap_err().0,
            "'big' must be from 0 to 100"
        );
        assert_eq!(arg_str(&args, "n"), "3");
    }
}
