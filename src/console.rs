//! Standard output for `run` and `eval` that a closed reader cannot panic.
//!
//! `println!` panics with `failed printing to stdout` when the reader has
//! gone, as in `crab-cage run … | head -1`. Here a failed write marks
//! standard output closed and later writes are dropped. The command goes on
//! and exits as it would have; only its output ends.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static CLOSED: AtomicBool = AtomicBool::new(false);

/// Write `text` to standard output and flush it. A write or flush error
/// (a broken pipe, most often) ends the output for the rest of the process.
pub fn print(text: &str) {
    if CLOSED.load(Ordering::Relaxed) {
        return;
    }
    let mut out = std::io::stdout().lock();
    if out
        .write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .is_err()
    {
        CLOSED.store(true, Ordering::Relaxed);
    }
}

/// [`print`] with a newline after `text`.
pub fn line(text: &str) {
    let mut owned = String::with_capacity(text.len() + 1);
    owned.push_str(text);
    owned.push('\n');
    print(&owned);
}
