//! stop_all_servers finds and stops a `flm serve` this service didn't start
//! (one left over from a crash). Skipped where a real FastFlowLM is installed,
//! so running the tests can't stop the user's server.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use assistant_core::flm::{FlmClient, which};

#[test]
fn a_stray_flm_serve_is_stopped() {
    if which("flm").is_some() {
        eprintln!("a real flm is installed here; skipping");
        return;
    }
    // A stand-in whose command line reads `flm serve`: a copy of sh running a script called "serve".
    let dir = tempfile::tempdir().unwrap();
    let flm = dir.path().join("flm");
    std::fs::copy("/bin/sh", &flm).unwrap();
    std::fs::write(dir.path().join("serve"), "sleep 300\n").unwrap();
    let mut child = Command::new(&flm)
        .arg("serve")
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // FLM's default port, which the stand-in "serves" on; nothing listens there here.
    let client = FlmClient::new("http://127.0.0.1:52625");
    let started = Instant::now();
    assert_eq!(client.stop_all_servers(), 1);
    assert!(child.wait().is_ok(), "the stray server exited");
    assert!(started.elapsed() < Duration::from_secs(6));
}
