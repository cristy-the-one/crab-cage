use std::io::{self, BufRead};

// The line is `VALUE FROM TO`: a number written in base FROM, to be written
// in base TO. Bases are from 2 to 36. Digits above 9 are the letters `a` to
// `z`; the value may use upper or lower case, the result uses lower case.
// `error` when a base is out of range, the value has a digit that does not
// belong to base FROM, the value does not fit in 64 bits without a sign, or
// the line does not have exactly three parts.
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
