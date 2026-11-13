use std::io::{self, BufRead};

// Reads a whole number from 1 to 3999 and writes it as a Roman numeral, with
// the usual subtractive forms (4 is `IV`, 9 is `IX`, 40 is `XL`, 90 is `XC`,
// 400 is `CD`, 900 is `CM`). Anything else gives `error`.
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
