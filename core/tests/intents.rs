//! Intent lookup against real desktop files (its own process, so GIO reads
//! the test's XDG folders).

use std::fs;

use assistant_core::tools::intents::{FILE_MANAGER, default_for};
use gio::prelude::*;

fn desktop(dir: &std::path::Path, id: &str, implements: &str) {
    fs::write(
        dir.join(format!("{id}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName={id}\nExec=true\nImplements={implements}\n"
        ),
    )
    .unwrap();
}

#[test]
fn intentapps_list_picks_the_default_and_checks_implements() {
    let root = tempfile::tempdir().unwrap();
    let apps = root.path().join("share/applications");
    let config = root.path().join("config");
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(&config).unwrap();
    desktop(&apps, "aaa.files", "org.freedesktop.FileManager1;");
    desktop(&apps, "zzz.files", "org.freedesktop.FileManager1;");
    desktop(&apps, "liar", "com.example.Other;");
    // SAFETY: this test binary has one test and sets the environment first.
    unsafe {
        std::env::set_var("XDG_DATA_DIRS", root.path().join("share"));
        std::env::set_var("XDG_DATA_HOME", root.path().join("nothing"));
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CONFIG_DIRS", root.path().join("etc"));
        std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    }
    // No list: the first implementation by desktop ID.
    assert_eq!(
        default_for(FILE_MANAGER).unwrap().id().unwrap(),
        "aaa.files.desktop"
    );
    // The user's choice wins, but only apps that really implement the intent count.
    fs::write(
        config.join("intentapps.list"),
        "[Default Applications]\norg.freedesktop.FileManager1=liar.desktop;zzz.files.desktop;\n",
    )
    .unwrap();
    assert_eq!(
        default_for(FILE_MANAGER).unwrap().id().unwrap(),
        "zzz.files.desktop"
    );
    assert!(default_for("com.example.Nothing1").is_none());
}
