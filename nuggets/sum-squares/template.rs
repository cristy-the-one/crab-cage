use std::io::{self, BufRead};

// The marked seed is wrong on purpose: it sums the values, not their squares.
fn sum_squares(xs: &[i64]) -> i64 {
    // CRAB:BEGIN
    xs.iter().sum()
    // CRAB:END
}

fn main() {
    let stdin = io::stdin();
    let mut xs = Vec::new();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("stdin: {error}");
                std::process::exit(1);
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.parse::<i64>() {
            Ok(value) => xs.push(value),
            Err(_) => {
                eprintln!("not an integer: {trimmed}");
                std::process::exit(1);
            }
        }
    }
    println!("{}", sum_squares(&xs));
}
