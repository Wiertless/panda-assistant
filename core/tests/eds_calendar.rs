//! create_event and its undo against a real Evolution Data Server on a
//! private bus (run under dbus-run-session with a throwaway HOME; skipped
//! unless PANDA_EDS_TEST is set).

use assistant_core::tool::{Args, Context};
use assistant_core::tools::personal::create_event;

#[test]
fn an_event_goes_into_the_calendar_and_undo_removes_it() {
    if std::env::var_os("PANDA_EDS_TEST").is_none() {
        eprintln!("set PANDA_EDS_TEST=1 under dbus-run-session to run this");
        return;
    }
    let ctx = Context::default();
    let args: Args = serde_json::from_value(
        serde_json::json!({"title": "Lunch with Ola", "start": "friday 13:00", "minutes": 45}),
    )
    .unwrap();
    let out = create_event(&ctx, &args).unwrap();
    eprintln!("{out}");
    assert!(
        out.contains("Lunch with Ola") && out.contains("13:00"),
        "{out}"
    );
    let undone = ctx.undo_last().unwrap();
    eprintln!("{undone}");
    assert!(undone.contains("gone"));
}
