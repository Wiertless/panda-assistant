//! Seeing the screen and images, and running commands in the zed toolbox.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gio::prelude::*;

use super::files::{home_path, short};
use super::run::output;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

/// A downscaled JPEG of an image file, as a data: URL for the model.
pub fn image_data_url(path: &Path, max_side: i32) -> Result<String, ToolError> {
    use gdk_pixbuf::Pixbuf;
    let mut pixbuf = Pixbuf::from_file_at_scale(path, max_side, max_side, true)
        .map_err(|e| ToolError(format!("couldn't read the image: {}", e.message())))?;
    if pixbuf.has_alpha() {
        // JPEG has no alpha: flatten onto white.
        let flat = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, pixbuf.width(), pixbuf.height())
            .ok_or("out of memory")?;
        flat.fill(0xFFFFFFFF);
        pixbuf.composite(&flat, 0, 0, pixbuf.width(), pixbuf.height(), 0.0, 0.0, 1.0, 1.0,
                         gdk_pixbuf::InterpType::Bilinear, 255);
        pixbuf = flat;
    }
    let bytes = pixbuf.save_to_bufferv("jpeg", &[("quality", "85")])
        .map_err(|e| ToolError(format!("couldn't convert the image: {}", e.message())))?;
    Ok(format!("data:image/jpeg;base64,{}", glib::base64_encode(&bytes)))
}

/// A screenshot of the whole screen through the desktop portal; returns the file.
#[allow(deprecated)] // signal_subscribe: the callback form suits a one-off wait
pub fn take_screenshot() -> Result<PathBuf, ToolError> {
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
            .map_err(|e| ToolError(e.message().to_string()))?;
        let token = format!("assistant{}", glib::random_int());
        let sender = bus.unique_name().map(|n| n[1..].replace('.', "_")).unwrap_or_default();
        let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let main_loop = glib::MainLoop::new(Some(&context), false);
        type Answer = Option<(u32, Option<String>)>;
        let result: Rc<RefCell<Answer>> = Rc::new(RefCell::new(None));
        let (r, l) = (result.clone(), main_loop.clone());
        let sub = bus.signal_subscribe(Some("org.freedesktop.portal.Desktop"), Some("org.freedesktop.portal.Request"),
            Some("Response"), Some(&handle), None, gio::DBusSignalFlags::NO_MATCH_RULE, move |_, _, _, _, _, params| {
                let code = params.child_value(0).get::<u32>().unwrap_or(2);
                let uri = glib::VariantDict::new(Some(&params.child_value(1))).lookup_value("uri", None).and_then(|v| v.get::<String>());
                *r.borrow_mut() = Some((code, uri));
                l.quit();
            });
        let options = glib::VariantDict::new(None);
        options.insert("handle_token", &token);
        options.insert("interactive", false);
        let call = bus.call_sync(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot", "Screenshot", Some(&("", options.end()).to_variant()),
            None, gio::DBusCallFlags::NONE, 10000, None::<&gio::Cancellable>);
        if let Err(e) = call {
            bus.signal_unsubscribe(sub);
            return Err(ToolError(format!("the screenshot portal isn't available: {}", e.message())));
        }
        let l = main_loop.clone();
        let timeout = glib::timeout_source_new(Duration::from_secs(60), None, glib::Priority::DEFAULT, move || {
            l.quit();
            glib::ControlFlow::Break
        });
        timeout.attach(Some(&context));
        main_loop.run();
        timeout.destroy();
        bus.signal_unsubscribe(sub);
        match result.take() {
            Some((0, Some(uri))) => gio::File::for_uri(&uri).path().ok_or_else(|| ToolError("the screenshot isn't a local file".into())),
            _ => Err("the screenshot was cancelled or not allowed".into()),
        }
    }).map_err(|e| ToolError(e.to_string()))?
}

pub fn look_at_screen(ctx: &Context, _: &Args) -> ToolResult {
    if let Some(hide) = &ctx.hide_window {
        hide();
        std::thread::sleep(Duration::from_millis(500)); // let the window animate away
    }
    let shot = take_screenshot();
    if let Some(show) = &ctx.show_window {
        show();
    }
    let path = shot?;
    let url = image_data_url(&path, 1024);
    // The portal saved a file just for this; don't leave it behind.
    let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age < Duration::from_secs(120));
    if fresh && path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("screenshot")) {
        let _ = std::fs::remove_file(&path);
    }
    ctx.images.lock().unwrap().push(url?);
    Ok("Took a screenshot of the screen; it's attached to the next message. Describe what's relevant.".into())
}

pub fn look_at_image(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    if !path.is_file() {
        return Err(format!("{} isn't a file", short(&path)).into());
    }
    ctx.images.lock().unwrap().push(image_data_url(&path, 1024)?);
    Ok(format!("Opened {}; it's attached to the next message.", path.file_name().unwrap().to_string_lossy()))
}

// -- commands, only inside the zed toolbox ---------------------------------------------

pub const TOOLBOX: &str = "zed";
const TOOLBOX_SETUP: &str = "/usr/libexec/silverpanda/zed-toolbox-setup";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT: usize = 3000;

fn toolbox_ready() -> Result<(), ToolError> {
    if crate::flm::which("toolbox").is_none() {
        return Err("toolbox isn't installed".into());
    }
    if output(&["podman", "container", "exists", TOOLBOX], Duration::from_secs(30), None).is_ok_and(|o| o.code == 0) {
        return Ok(());
    }
    if !Path::new(TOOLBOX_SETUP).exists() {
        return Err(format!("there's no '{TOOLBOX}' toolbox; create one with: toolbox create {TOOLBOX}").into());
    }
    let out = output(&[TOOLBOX_SETUP], Duration::from_secs(1200), None)?;
    if out.code != 0 {
        let err = format!("{}{}", out.stderr, out.stdout);
        let tail: String = err.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("couldn't set up the {TOOLBOX} toolbox: {tail}").into());
    }
    Ok(())
}

pub fn run_command(_: &Context, args: &Args) -> ToolResult {
    let command = arg_str(args, "command").trim().to_string();
    if command.is_empty() {
        return Err("no command given".into());
    }
    if command.chars().count() > 2000 {
        return Err("the command is too long".into());
    }
    let raw = arg_str(args, "folder");
    let folder = home_path(if raw.trim().is_empty() { "~" } else { &raw }, true)?;
    if !folder.is_dir() {
        return Err(format!("{} isn't a folder", short(&folder)).into());
    }
    toolbox_ready()?;
    let out = output(&["toolbox", "run", "--container", TOOLBOX, "bash", "-lc", &command], COMMAND_TIMEOUT, Some(&folder))
        .map_err(|e| if e.0.contains("didn't finish") {
            ToolError(format!("still running after {} seconds, so it was stopped", COMMAND_TIMEOUT.as_secs()))
        } else { e })?;
    let text = [out.stdout.trim_matches('\n'), out.stderr.trim_matches('\n')].into_iter()
        .filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join("\n");
    let text = text.trim();
    let n = text.chars().count();
    let shown = if n > MAX_OUTPUT { format!("…{}", text.chars().skip(n - MAX_OUTPUT).collect::<String>()) } else { text.to_string() };
    Ok(format!("Exit code {}.{}", out.code, if shown.is_empty() { " No output.".into() } else { format!("\n{shown}") }))
}

pub fn command_detail(args: &Args) -> String {
    let folder = arg_str(args, "folder");
    format!("{}{}", arg_str(args, "command"), if folder.trim().is_empty() { String::new() } else { format!("\n(in {})", folder.trim()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn commands_run_in_the_toolbox() {
        let h = temp_home();
        let root = crate::paths::home();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.join("proj")).unwrap();
        let log = root.join("log");
        for (name, body) in [("podman", "exit 0\n".to_string()),
                             ("toolbox", format!("echo \"$PWD|$@\" >> {}\necho built; echo warn >&2; exit 3\n", log.display()))] {
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", format!("{}:{old}", bin.display())) };
        let args: Args = serde_json::from_value(serde_json::json!({"command": "make && ls", "folder": "proj"})).unwrap();
        let out = run_command(&Context::default(), &args);
        unsafe { std::env::set_var("PATH", old) };
        assert_eq!(out.unwrap(), "Exit code 3.\nbuilt\nwarn");
        let real = std::fs::canonicalize(root.join("proj")).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), format!("{}|run --container zed bash -lc make && ls", real.display()));
        assert_eq!(command_detail(&args), "make && ls\n(in proj)");
        let bad: Args = serde_json::from_value(serde_json::json!({"command": "ls", "folder": "/etc"})).unwrap();
        assert!(run_command(&Context::default(), &bad).is_err());
        drop(h);
    }

    #[test]
    fn images_become_small_jpegs() {
        let _h = temp_home();
        let path = crate::paths::home().join("pic.png");
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 2000, 1000).unwrap();
        pb.fill(0x3584e4ff);
        pb.savev(&path, "png", &[]).unwrap();
        let url = image_data_url(&path, 1024).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,/9j/"));
        let ctx = Context::default();
        let args: Args = serde_json::from_value(serde_json::json!({"path": "pic.png"})).unwrap();
        look_at_image(&ctx, &args).unwrap();
        assert_eq!(ctx.images.lock().unwrap().len(), 1);
    }
}
