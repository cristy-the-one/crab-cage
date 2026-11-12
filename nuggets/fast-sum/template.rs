use std::io::{self, Read};

const MODULUS: u64 = 1_000_000_007;

// The sum of the squares of 1..=n, modulo MODULUS.
//
// The marked seed is correct but slow on purpose: it builds each square by
// repeated addition, so it does about n * n / 2 steps. A single loop, or the
// closed form n (n + 1) (2n + 1) / 6, prints the same number much sooner.
fn sum_of_squares_mod(n: u64) -> u64 {
    // CRAB:BEGIN
    let mut sum = 0u64;
    let mut i = 1u64;
    while i <= n {
        let mut square = 0u64;
        let mut k = 0u64;
        while k < i {
            square = (square + i) % MODULUS;
            k += 1;
        }
        sum = (sum + square) % MODULUS;
        i += 1;
    }
    sum
    // CRAB:END
}

fn main() {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("stdin: {error}");
        std::process::exit(1);
    }
    let trimmed = input.trim();
    let n = match trimmed.parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
            eprintln!("not a non-negative integer: {trimmed}");
            std::process::exit(1);
        }
    };
    println!("{}", sum_of_squares_mod(n));
}
