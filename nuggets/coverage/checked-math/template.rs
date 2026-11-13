use std::io::{self, BufRead};

// The line is `A OP B`: two whole numbers and one of `+ - * /` between them,
// separated by spaces. The numbers are 32-bit numbers with a sign.
//
// - The result is the answer in decimal. Division rounds toward zero.
// - `overflow` when the answer does not fit in 32 bits with a sign.
// - `div0` for a division by zero.
// - `error` when the line does not have that form or a number does not fit.
//
// Use the `checked_` methods of `i32`.
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
