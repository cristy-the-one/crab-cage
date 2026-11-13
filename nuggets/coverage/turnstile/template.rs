use std::io::{self, BufRead};

// A turnstile is locked or unlocked, and starts locked. The line is a row of
// events, one character each: `c` is a coin, `p` is a push.
//
// - Locked: a coin unlocks it; a push does nothing.
// - Unlocked: a push lets one person through and locks it; a coin is kept
//   but changes nothing.
//
// The result is `STATE passed=N coins=M`: the state at the end (`locked` or
// `unlocked`), how many people went through, and how many coins were put in.
// Any other character in the line gives `error`.
//
// Use an enum for the state and a `match` on the state and the event.
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
