//! search_apps / open_search_result against a fake app search provider on a
//! private session bus (its own process, so the environment is the test's).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};

use assistant_core::tool::{Args, Context};
use assistant_core::tools::intents;
use gio::prelude::*;
use serde_json::json;

const XML: &str = r#"<node><interface name="org.gnome.Shell.SearchProvider2">
  <method name="GetInitialResultSet"><arg type="as" name="terms" direction="in"/><arg type="as" direction="out"/></method>
  <method name="GetSubsearchResultSet"><arg type="as" direction="in"/><arg type="as" direction="in"/><arg type="as" direction="out"/></method>
  <method name="GetResultMetas"><arg type="as" direction="in"/><arg type="aa{sv}" direction="out"/></method>
  <method name="ActivateResult"><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="in"/></method>
  <method name="LaunchSearch"><arg type="as" direction="in"/><arg type="u" direction="in"/></method>
</interface></node>"#;

fn args(v: serde_json::Value) -> Args {
    serde_json::from_value(v).unwrap()
}

#[test]
fn asks_and_opens_other_apps_search() {
    let Ok(mut daemon) = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
    else {
        eprintln!("no dbus-daemon, skipping");
        return;
    };
    let mut address = String::new();
    let mut out = BufReader::new(daemon.stdout.take().unwrap()); // kept open: the daemon must outlive the test
    out.read_line(&mut address).unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("gnome-shell/search-providers");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("calc.ini"), "[Shell Search Provider]\nDesktopId=org.example.Calc.desktop\n\
        BusName=org.example.Calc.SearchProvider\nObjectPath=/org/example/Calc/SearchProvider\nVersion=2\n").unwrap();
    // SAFETY: one test in this binary, and the environment is set before any thread starts.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim());
        std::env::set_var("XDG_DATA_DIRS", root.path());
        std::env::set_var("GSETTINGS_SCHEMA_DIR", root.path()); // no schema: nothing turned off
    }

    let opened = Arc::new(Mutex::new(None::<String>));
    let (ready_tx, ready) = mpsc::channel();
    let seen = opened.clone();
    std::thread::spawn(move || {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).unwrap();
            let iface = gio::DBusNodeInfo::for_xml(XML)
                .unwrap()
                .lookup_interface("org.gnome.Shell.SearchProvider2")
                .unwrap();
            let _reg = bus
                .register_object("/org/example/Calc/SearchProvider", &iface)
                .method_call(move |_, _, _, _, method, params, inv| match method {
                    "GetInitialResultSet" => {
                        let (terms,): (Vec<String>,) = params.get().unwrap();
                        let ids: Vec<String> = if terms == ["2+2"] {
                            vec!["2+2".into()]
                        } else {
                            vec![]
                        };
                        inv.return_value(Some(&(ids,).to_variant()));
                    }
                    "GetResultMetas" => {
                        let d = glib::VariantDict::new(None);
                        d.insert("id", "2+2");
                        d.insert("name", "4");
                        d.insert("description", "2+2");
                        let metas = glib::Variant::array_from_iter_with_type(
                            glib::VariantTy::VARDICT,
                            [d.end()],
                        );
                        inv.return_value(Some(&glib::Variant::tuple_from_iter([metas])));
                    }
                    "ActivateResult" => {
                        *seen.lock().unwrap() = params.child_value(0).get::<String>();
                        inv.return_value(None);
                    }
                    _ => inv.return_value(None),
                })
                .build()
                .unwrap();
            bus.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&("org.example.Calc.SearchProvider", 4u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                None::<&gio::Cancellable>,
            )
            .unwrap();
            ready_tx.send(()).unwrap();
            glib::MainLoop::new(Some(&ctx), false).run();
        })
        .unwrap();
    });
    ready.recv().unwrap();

    let ctx = Context::default();
    let out = intents::search_apps(&ctx, &args(json!({"query": "2+2"}))).unwrap();
    assert!(
        out.contains("4 — 2+2 [app org.example.Calc.desktop, id 2+2]"),
        "{out}"
    );
    assert!(
        intents::search_apps(&ctx, &args(json!({"query": "nothing"})))
            .unwrap()
            .starts_with("No app")
    );
    intents::open_search_result(
        &ctx,
        &args(json!({"app": "org.example.Calc", "id": "2+2", "query": "2+2"})),
    )
    .unwrap();
    assert_eq!(opened.lock().unwrap().as_deref(), Some("2+2"));
    assert!(intents::open_search_result(&ctx, &args(json!({"app": "nope", "id": "x"}))).is_err());
    // GDBus raises SIGTERM when the session bus goes away, unless told not to.
    gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
        .unwrap()
        .set_exit_on_close(false);
    daemon.kill().ok();
}
