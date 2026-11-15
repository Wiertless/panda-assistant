//! Running helper programs: never through a shell, always with a time limit.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::flm::which;
use crate::tool::ToolError;

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program; Err if it's missing or takes longer than `timeout`.
pub fn output(cmd: &[&str], timeout: Duration, cwd: Option<&std::path::Path>) -> Result<Output, ToolError> {
    if which(cmd[0]).is_none() && !cmd[0].starts_with('/') {
        return Err(format!("{} is not installed", cmd[0]).into());
    }
    let mut c = Command::new(cmd[0]);
    c.args(&cmd[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        c.current_dir(dir);
    }
    let mut child = c.spawn().map_err(|e| ToolError(format!("couldn't run {}: {e}", cmd[0])))?;
    // Read both pipes on threads so a chatty program can't fill one and stall.
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t1 = std::thread::spawn(move || { let mut s = String::new(); let _ = so.read_to_string(&mut s); s });
    let t2 = std::thread::spawn(move || { let mut s = String::new(); let _ = se.read_to_string(&mut s); s });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| ToolError(e.to_string()))? {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} didn't finish within {} seconds", cmd[0], timeout.as_secs()).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output { code: status.code().unwrap_or(-1), stdout: t1.join().unwrap_or_default(), stderr: t2.join().unwrap_or_default() })
}

/// Run a program and return its trimmed output, or its error text.
pub fn run(cmd: &[&str], timeout_secs: u64) -> Result<String, ToolError> {
    let out = output(cmd, Duration::from_secs(timeout_secs), None)?;
    if out.code != 0 {
        let msg = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        return Err(if msg.is_empty() { format!("{} failed", cmd[0]) } else { msg.to_string() }.into());
    }
    Ok(out.stdout.trim().to_string())
}

/// A blocking D-Bus call on the session or system bus.
#[allow(clippy::too_many_arguments)]
pub fn dbus_call(system: bool, name: &str, path: &str, iface: &str, method: &str, args: Option<&glib::Variant>,
                 reply: Option<&glib::VariantTy>, timeout_ms: i32) -> Result<glib::Variant, glib::Error> {
    let bus = gio::bus_get_sync(if system { gio::BusType::System } else { gio::BusType::Session },
                                None::<&gio::Cancellable>)?;
    bus.call_sync(Some(name), path, iface, method, args, reply, gio::DBusCallFlags::NONE, timeout_ms,
                  None::<&gio::Cancellable>)
}

/// Set a D-Bus property.
/// A D-Bus property's value.
pub fn get_property(system: bool, name: &str, path: &str, iface: &str, prop: &str) -> Result<glib::Variant, glib::Error> {
    use glib::prelude::*;
    let v = dbus_call(system, name, path, "org.freedesktop.DBus.Properties", "Get", Some(&(iface, prop).to_variant()),
                      Some(glib::VariantTy::new("(v)").unwrap()), 5000)?;
    Ok(v.child_value(0).as_variant().unwrap_or(v))
}

pub fn set_property(system: bool, name: &str, path: &str, iface: &str, prop: &str, value: glib::Variant)
                    -> Result<(), glib::Error> {
    use glib::prelude::*;
    dbus_call(system, name, path, "org.freedesktop.DBus.Properties", "Set",
              Some(&(iface, prop, value).to_variant()), None, 5000).map(|_| ())
}
