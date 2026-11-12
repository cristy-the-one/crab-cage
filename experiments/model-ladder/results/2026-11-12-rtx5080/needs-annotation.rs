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
let mut ranges: Vec<(usize, usize)> = line.split_whitespace()
    .map(|s| {
        let parts: Vec<&str> = s.split('-').collect();
        (parts[0].parse().unwrap(), parts[1].parse().unwrap())
    })
    .collect();

ranges.sort_by_key(|&(lo, _)| lo);

let mut merged_ranges = Vec::new();
for range in ranges {
    if let Some(last_range) = merged_ranges.last_mut() {
        if range.0 <= last_range.1 + 1 {
            last_range.1 = last_range.1.max(range.1);
            continue;
        }
    }
    merged_ranges.push(range);
}

merged_ranges.into_iter()
    .map(|(lo, hi)| format!("{}-{}", lo, hi))
    .collect::<Vec<String>>()
    .join(" ")
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
