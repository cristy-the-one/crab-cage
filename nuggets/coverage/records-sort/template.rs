use std::io::{self, BufRead};

// The line holds records `name:age:city` separated by `;`.
//
// - The result is the names, separated by `,`, in this order: by city in
//   alphabetical order; in the same city the oldest first; at the same age by
//   name in alphabetical order.
// - `error` when a record does not have exactly three parts or an age is not
//   a whole number from 0 to 150. An empty line gives an empty line.
//
// Use a struct for a record and `std::cmp::Ordering`.
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
