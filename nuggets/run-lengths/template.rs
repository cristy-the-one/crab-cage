use std::io::{self, BufRead};

// Shortens a line by writing each run of one repeated character once.
//
// - A run of two or more of the same character becomes the length of the
//   run in decimal, then the character: `aaa` becomes `3a`.
// - A run of exactly one character is written as that character alone:
//   `abc` stays `abc`.
// - A digit is the exception, so that the result can be read back: a run of
//   a digit is always written with its length, even when the length is one.
//   `7` becomes `17`, and `a11b` becomes `a21b`.
// - Characters are Unicode characters, not bytes. An empty line stays empty.
fn shorten(line: &str) -> String {
    // CRAB:BEGIN
    line.to_string()
    // CRAB:END
}

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
        println!("{}", shorten(&line));
    }
}
