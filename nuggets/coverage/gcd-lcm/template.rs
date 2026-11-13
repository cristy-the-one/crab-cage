use std::io::{self, BufRead};

// The line holds whole numbers above 0, separated by spaces. The result is
// their greatest common divisor, a space, and their least common multiple.
// One number gives that number twice. An empty line, a zero, a negative
// number or anything that is not a number gives `error`. Use `u64`.
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
