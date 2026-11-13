use std::io::{self, BufRead};

// The line is `K: n1 n2 n3 ...`: a window size, a colon, and whole numbers.
//
// - The result is the largest number of every K numbers in a row, in order,
//   separated by a space.
// - With fewer than K numbers the result is empty.
// - `error` when K is not a whole number above 0, a number cannot be read,
//   or the colon is missing.
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
