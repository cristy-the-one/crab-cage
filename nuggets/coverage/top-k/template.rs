use std::io::{self, BufRead};

// The line is `K: n1 n2 n3 ...`: a count, a colon, and whole numbers.
//
// - The result is the K largest numbers, largest first, separated by a space.
//   A number that occurs twice counts twice.
// - With fewer than K numbers, all of them are listed.
// - `error` when K cannot be read, a number cannot be read, or the colon is
//   missing. K may be 0, which gives an empty line.
//
// Use a `BinaryHeap`.
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
