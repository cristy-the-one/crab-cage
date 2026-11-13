use std::io::{self, BufRead};

// Evaluates the line as an expression in reverse Polish notation.
//
// - Parts are separated by spaces: whole numbers (they may be negative) and
//   the operators `+ - * /`.
// - Division is whole-number division that rounds toward zero.
// - The result is the one value left, in decimal.
// - `error` for: an unknown part, an operator with fewer than two values, a
//   division by zero, an empty line, or more than one value left at the end.
//
// Use an enum for the parts, and `i64` for the values.
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
