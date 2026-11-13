use std::io::{self, BufRead};

// Groups the words of the line that are anagrams of each other.
//
// - Words are separated by whitespace and are used as written.
// - Inside a group the words are in alphabetical order, separated by a space.
// - The groups are ordered by their first word, and separated by ` | `.
//
// Use a `BTreeMap` from the sorted letters of a word to the words.
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
