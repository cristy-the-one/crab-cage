use std::io::{self, BufRead};

// Checks the brackets `( ) [ ] { }` of the line. Other characters are skipped.
//
// - `ok` when every bracket is closed by its own kind in the right order.
// - `bad N` when a closing bracket does not match: N is the position of that
//   bracket in the line, counting characters from 0.
// - `open N` when the line ends with N brackets still open.
//
// Use a `Vec` as a stack.
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
