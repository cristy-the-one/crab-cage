use std::io::{self, BufRead};

// Counts the words of the line.
//
// - Words are separated by whitespace and compared in lower case.
// - The result lists each word once as `word:count`, separated by one space.
// - Order: the highest count first; words with the same count in alphabetical
//   order.
// - An empty line gives an empty line.
//
// Use a map from `std::collections` for the counts.
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
