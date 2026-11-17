//! assistantd: the Assistant's background service (see `service`).

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("assistantd {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    // At login (autostart): run only when reminders are waiting; otherwise D-Bus starts it when needed.
    if args.iter().any(|a| a == "--if-reminders") && assistant_core::reminders::pending().is_empty()
    {
        return;
    }
    if args.iter().any(|a| a == "--eval") {
        std::process::exit(assistant_core::eval::main(&args));
    }
    if assistant_core::service::run().is_break() {
        std::process::exit(1);
    }
}
