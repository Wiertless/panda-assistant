//! assistantd: the Assistant's background service (see `service`).

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("assistantd {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if assistant_core::service::run().is_break() {
        std::process::exit(1);
    }
}
