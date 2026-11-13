use std::io::{self, BufRead};

// The line holds bytes in hexadecimal, separated by spaces, such as `ff 01 a0`.
//
// - The result is their sum in decimal.
// - Upper and lower case digits are both allowed. An empty line gives `0`.
// - If any part is not a byte in hexadecimal (not hex, or above `ff`), the
//   result is `error`.
//
// Use `Result` and the `?` operator in a helper function.
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
