//! Where the assistant keeps its files (the same places as the Python app).

use std::env;
use std::path::PathBuf;

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// ~/.local/share/panda-assistant: memory, skills, chats.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("panda-assistant")
}

/// ~/.config/panda-assistant: config.json, mcp.json.
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("panda-assistant")
}

/// ~/.local/state/panda-assistant: logs.
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("panda-assistant")
}
