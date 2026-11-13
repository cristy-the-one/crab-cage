use std::io::{self, BufRead};

// The line is a kind, a colon, and values separated by spaces:
//
// - `i:` whole numbers. The result is the largest.
// - `f:` decimal numbers. The result is the largest, with one digit after
//   the point.
// - `s:` words. The result is the last one in alphabetical order.
//
// `error` when the kind is unknown, there are no values, or a value cannot
// be read.
//
// Write one generic function `largest<T: PartialOrd + Copy>(items: &[T]) ->
// Option<T>` and use it for all three kinds.
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
