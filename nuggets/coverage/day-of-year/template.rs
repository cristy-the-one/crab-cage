use std::io::{self, BufRead};

// Reads a date `YYYY-MM-DD` and gives the number of the day in its year:
// January 1 is 1, December 31 is 365 or 366.
//
// - A year is a leap year when it divides by 4, except that a year that
//   divides by 100 is one only when it also divides by 400.
// - `error` for anything that is not a real date in that form: a month
//   outside 1 to 12, a day the month does not have, missing parts, or text
//   that is not a number.
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
