//! The settings file shared with assistantd (~/.config/panda-assistant/config.json).

use serde_json::{Value, json};

use assistant_core::{paths, store};

fn path() -> std::path::PathBuf {
    paths::config_dir().join("config.json")
}

pub fn load() -> Value {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Change some keys, keeping everything else (the service writes "always_allow").
pub fn update(changes: &[(&str, Value)]) {
    let mut cfg = load();
    for (k, v) in changes {
        cfg[*k] = v.clone();
    }
    let _ = store::write_atomic(&path(), &serde_json::to_string_pretty(&cfg).unwrap_or_default());
}

pub fn str_or(cfg: &Value, key: &str, default: &str) -> String {
    cfg[key].as_str().unwrap_or(default).to_string()
}

pub fn int_or(cfg: &Value, key: &str, default: i64) -> i64 {
    cfg[key].as_i64().unwrap_or(default)
}

pub fn bool_or(cfg: &Value, key: &str, default: bool) -> bool {
    cfg[key].as_bool().unwrap_or(default)
}
