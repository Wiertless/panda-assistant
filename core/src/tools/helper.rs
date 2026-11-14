//! Tools still served by the Python code (`panda_assistant.helper`), because
//! they need GNOME libraries with only GObject-introspection bindings:
//! window text (AT-SPI), file contents (TinySPARQL), calendar and contacts
//! (Evolution Data Server).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::tool::{Args, Context, ToolError, ToolResult};

/// Where the Python package is installed (install.sh puts it here).
fn python_path() -> String {
    std::env::var("PANDA_PYTHONPATH").unwrap_or_else(|_| "/usr/share/panda-assistant".into())
}

pub fn call(tool: &str, args: &Args, timeout: Duration) -> ToolResult {
    let python = std::env::var("PANDA_PYTHON").unwrap_or_else(|_| "python3".into());
    let mut child = Command::new(&python)
        .args(["-m", "panda_assistant.helper", tool])
        .env("PYTHONPATH", python_path())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn()
        .map_err(|e| ToolError(format!("couldn't start the helper for {tool}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(Value::Object(args.clone()).to_string().as_bytes());
    }
    let mut out = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{tool} took too long").into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = reader.join().unwrap_or_default();
    let reply: Value = serde_json::from_str(text.trim()).map_err(|_| ToolError(format!("{tool} isn't available here")))?;
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply["result"].as_str().unwrap_or("").to_string())
    } else {
        Err(reply["error"].as_str().unwrap_or("failed").to_string().into())
    }
}

pub fn read_window(ctx: &Context, args: &Args) -> ToolResult {
    // If the assistant's own window has focus, step aside so the window the
    // user means (the one behind) becomes the focused one.
    if let Some(hide) = &ctx.hide_window {
        hide();
        std::thread::sleep(Duration::from_millis(400));
    }
    let result = call("read_window", args, Duration::from_secs(15));
    if let Some(show) = &ctx.show_window {
        show();
    }
    result
}

pub fn search_file_contents(_: &Context, args: &Args) -> ToolResult {
    call("search_file_contents", args, Duration::from_secs(20))
}

pub fn read_document(_: &Context, args: &Args) -> ToolResult {
    call("read_document", args, Duration::from_secs(30))
}

pub fn get_events(_: &Context, args: &Args) -> ToolResult {
    call("get_events", args, Duration::from_secs(30))
}

pub fn find_contact(_: &Context, args: &Args) -> ToolResult {
    call("find_contact", args, Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_helper_round_trip() {
        let py = std::env::var("PANDA_PYTHON").unwrap_or_else(|_| "python3".into());
        if Command::new(&py).arg("-c").arg("import gi").status().map(|s| !s.success()).unwrap_or(true) {
            return; // no PyGObject here
        }
        let src = concat!(env!("CARGO_MANIFEST_DIR"), "/../src");
        unsafe { std::env::set_var("PANDA_PYTHONPATH", src) };
        let args: Args = serde_json::from_value(serde_json::json!({"query": "?!"})).unwrap();
        // Bad input comes back as a tool error, not a crash.
        assert_eq!(call("search_file_contents", &args, Duration::from_secs(20)).unwrap_err().0, "say what to look for");
        assert!(call("nope", &args, Duration::from_secs(20)).is_err());
    }
}
