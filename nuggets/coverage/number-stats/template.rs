use std::io::{self, BufRead};

// The line holds whole numbers separated by spaces. The result is three
// facts, in this form:
//
// `evens=S steps=D1,D2,... neg=P`
//
// - `evens`: the sum of the squares of the even numbers.
// - `steps`: the difference between each number and the one before it, in
//   order, separated by commas. With fewer than two numbers it is `-`.
// - `neg`: the position of the first negative number, counting from 0, or
//   `-` when there is none.
//
// `error` when a number cannot be read. An empty line gives
// `evens=0 steps=- neg=-`.
//
// Use iterator methods (`filter`, `map`, `sum`, `windows`, `position`).
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
