use std::io::{self, BufRead};

// Reads a whole number that fits in 32 bits without a sign and writes four
// facts about its bits, in this form:
//
// `ones=N lead=N trail=N swapped=XXXXXXXX`
//
// - `ones`: how many bits are 1.
// - `lead`: how many zero bits come before the first 1 bit, from the top.
// - `trail`: how many zero bits come after the last 1 bit. For 0 it is 32.
// - `swapped`: the number with its four bytes in reverse order, as eight
//   hexadecimal digits in lower case.
//
// Anything else gives `error`.
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
