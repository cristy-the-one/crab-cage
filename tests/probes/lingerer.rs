//! With no arguments: start a copy of itself with `child`, inheriting stdio,
//! print its pid and `parent done`, and exit. With `child`: sleep 30 s while
//! holding the inherited stdout and stderr open.

use std::io::Write;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("child") {
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }
    let exe = std::env::current_exe().expect("current_exe");
    let child = std::process::Command::new(exe)
        .arg("child")
        .spawn()
        .expect("spawn child");
    let mut out = std::io::stdout();
    writeln!(out, "child {}", child.id()).unwrap();
    writeln!(out, "parent done").unwrap();
    out.flush().unwrap();
}
