use std::io::{self, BufRead};

// The line is a table of whole numbers: rows separated by `;`, numbers in a
// row separated by spaces. The result is the table with rows and columns
// exchanged, in the same form. `error` when the rows do not all have the
// same count of numbers, a row is empty, or a number cannot be read.
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
