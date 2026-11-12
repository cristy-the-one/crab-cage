use std::io::{self, BufRead};

// Merges ranges of whole numbers.
//
// The line holds ranges separated by spaces. A range is written `lo-hi`,
// two non-negative integers with `lo <= hi`, and covers every integer from
// `lo` to `hi`, both included. The ranges come in any order.
//
// - Two ranges that share an integer are merged into one.
// - So are two ranges that touch: `1-4` and `5-9` become `1-9`, because no
//   integer lies between them. `1-4` and `6-9` stay apart.
// - The result lists the merged ranges by increasing `lo`, in the same
//   `lo-hi` form, separated by one space. An empty line stays empty.
fn merge(line: &str) -> String {
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
        println!("{}", merge(&line));
    }
}
