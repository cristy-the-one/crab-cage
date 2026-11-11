//! Print 4000 numbered lines of 100 bytes (400,000 bytes) on stdout and
//! 1000 on stderr, then exit 0.

use std::io::Write;

fn main() {
    let mut out = std::io::stdout().lock();
    for i in 0..4000 {
        writeln!(out, "{i:08} {}", "x".repeat(90)).unwrap();
    }
    out.flush().unwrap();
    let mut err = std::io::stderr().lock();
    for i in 0..1000 {
        writeln!(err, "{i:08} {}", "e".repeat(90)).unwrap();
    }
    err.flush().unwrap();
}
