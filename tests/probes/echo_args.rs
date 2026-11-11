//! Prints what it was started with, one fact per line, so a test can see
//! that arguments, the environment and the working directory arrived intact.
use std::io::Read;

fn main() {
    for arg in std::env::args().skip(1) {
        println!("arg=[{arg}]");
    }
    for name in ["CRAB_PROBE_SET", "CRAB_PROBE_SECRET", "CRAB_PROBE_KEPT"] {
        match std::env::var(name) {
            Ok(value) => println!("{name}=[{value}]"),
            Err(_) => println!("{name} unset"),
        }
    }
    let dir = std::env::current_dir().unwrap();
    println!("cwd={}", dir.file_name().unwrap().to_string_lossy());
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    println!("stdin=[{}]", input.trim_end());
    eprintln!("to stderr");
    std::process::exit(7);
}
