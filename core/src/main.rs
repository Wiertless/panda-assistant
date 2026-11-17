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
    // For testing: `assistantd --tool NAME '{"arg": "value"}'` runs one tool and prints its result.
    if let Some(i) = args.iter().position(|a| a == "--tool") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        let json = args.get(i + 2).map_or("{}", String::as_str);
        let Some(tool) = assistant_core::tools::all()
            .into_iter()
            .find(|t| t.name == name)
        else {
            eprintln!("no tool named {name}");
            std::process::exit(2);
        };
        let args = serde_json::from_str(json).unwrap_or_default();
        match (tool.run)(&Default::default(), &args) {
            Ok(r) => println!("{r}"),
            Err(e) => {
                eprintln!("error: {}", e.0);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--eval") {
        std::process::exit(assistant_core::eval::main(&args));
    }
    if assistant_core::service::run().is_break() {
        std::process::exit(1);
    }
}
