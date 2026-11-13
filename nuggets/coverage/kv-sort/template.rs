use std::io::{self, BufRead};

// The line holds `key=value` pairs separated by `;`.
//
// - A part without `=` is skipped. The key is the text before the first `=`.
// - Keys and values are trimmed of spaces.
// - The result is the pairs sorted by key, then by value, written `key=value`
//   and joined with `;`.
//
// Use a struct for a pair.
//
// Between the two markers you may write `use` lines, types, traits and
// helper functions as well as `solve`. Keep the name and the signature of
// `solve`: it gets one input line and returns one output line.
// CRAB:BEGIN
fn solve(line: &str) -> String {
    let _ = line;
    String::new()
}
// CRAB:END

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("stdin: {error}");
                std::process::exit(1);
            }
        };
        println!("{}", solve(&line));
    }
}
