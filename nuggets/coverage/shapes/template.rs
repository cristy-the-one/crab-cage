use std::io::{self, BufRead};

// The line names a shape and its sizes:
//
// - `circle R` has the area pi times R squared.
// - `rect W H` has the area W times H.
// - `tri B H` has the area B times H divided by 2.
//
// The result is the area with exactly two digits after the point. Sizes are
// decimal numbers. An unknown shape, a missing or extra size, or a size that
// is not a number gives `error`.
//
// Use a trait `Shape` with a method `area`, one struct per shape, and a
// `Box<dyn Shape>`.
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
