use std::io::{self, BufRead};

// Reads a Roman numeral (`I V X L C D M`, upper case) and gives its value in
// decimal. A smaller numeral before a larger one is subtracted: `IV` is 4,
// `MCMXCIV` is 1994. A line with any other character, or an empty line, gives
// `error`.
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
