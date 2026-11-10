//! Stand-in for the lust CLI. Tests compile this with rustc and put it on
//! the path Crab Cage is told to use. It is not the real compiler.

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process;

fn main() {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("fake-lust"));
    let mode_path = PathBuf::from(format!("{}.crabmode", exe.display()));
    if let Ok(mode) = fs::read_to_string(&mode_path) {
        run_program(mode.trim());
        return;
    }
    let args: Vec<String> = env::args().skip(1).collect();
    if let Ok(log) = env::var("FAKE_LUST_LOG") {
        let mut line = args.join(" ");
        line.push('\n');
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = file.write_all(line.as_bytes());
        }
    }
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("lust 0.3.0-fake");
        }
        Some("build") if args.get(1).map(String::as_str) == Some("--help") => {
            println!("Usage: lust build <file.rs> --sandbox --max-memory <size> --time-limit <duration> -o <out>");
        }
        Some("check") => cmd_check(&args),
        Some("fix") => cmd_fix(&args),
        Some("build") => cmd_build(&args, &exe),
        _ => {
            eprintln!("fake lust: unknown command");
            process::exit(2);
        }
    }
}

fn cmd_check(args: &[String]) {
    if !args.iter().any(|arg| arg == "--sandbox") {
        eprintln!("expected --sandbox");
        process::exit(3);
    }
    let file = rust_file(args);
    let text = fs::read_to_string(&file).unwrap_or_default();
    if text.contains("BROKEN_TOKEN") {
        println!(
            "{{\"code\":\"L0102\",\"message\":\"cannot find value `BROKEN_TOKEN` in this scope\",\"span\":{{\"file\":\"{file}\",\"line\":1,\"col\":1}},\"fix\":\"replace BROKEN_TOKEN\",\"edits\":[]}}",
            file = file
        );
        process::exit(1);
    }
    if args.iter().any(|arg| arg == "--json") {
        println!("{{\"schema\":1,\"ok\":true}}");
    }
}

fn cmd_fix(args: &[String]) {
    let file = rust_file(args);
    let text = fs::read_to_string(&file).unwrap_or_default();
    let _ = fs::write(&file, text.replace("BROKEN_TOKEN", "0.0"));
}

fn cmd_build(args: &[String], exe: &std::path::Path) {
    if !args.iter().any(|arg| arg == "--sandbox")
        || !args.iter().any(|arg| arg == "--max-memory")
        || !args.iter().any(|arg| arg == "--time-limit")
    {
        eprintln!("build requires --sandbox, --max-memory and --time-limit");
        process::exit(3);
    }
    let out = match flag(args, "-o") {
        Some(path) => path,
        None => {
            eprintln!("missing -o");
            process::exit(3);
        }
    };
    let file = rust_file(args);
    let text = fs::read_to_string(&file).unwrap_or_default();
    fs::copy(exe, &out).unwrap_or_else(|err| {
        eprintln!("copy failed: {err}");
        process::exit(4);
    });
    let mode = if text.contains("MODE_MEMORY") {
        "memory"
    } else {
        "echo"
    };
    let _ = fs::write(format!("{out}.crabmode"), mode);
}

fn run_program(mode: &str) {
    if mode == "memory" {
        eprintln!(
            "error[L0532]: this program was built with --max-memory 1048576 and tried to use more memory"
        );
        process::exit(101);
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text == "reset" {
            writeln!(stdout, "ok").ok();
            stdout.flush().ok();
            continue;
        }
        writeln!(stdout, "0.5000000000 0.0000000000 28.0000000000").ok();
        stdout.flush().ok();
    }
}

fn rust_file(args: &[String]) -> String {
    args.iter()
        .rev()
        .find(|arg| arg.ends_with(".rs"))
        .cloned()
        .unwrap_or_default()
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
