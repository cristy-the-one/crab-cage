use std::io::{self, BufRead};

// Normalizes a Unix-style path, as text. No file system is consulted.
//
// - `/` separates components. Repeated separators count as one.
// - A `.` component is dropped.
// - A `..` component removes the component before it.
// - An absolute path (one that starts with `/`) cannot go above the root:
//   a `..` at the root is dropped.
// - A relative path can: a `..` with nothing before it to remove is kept.
// - The result has no trailing `/`. An absolute path that normalizes to
//   nothing is `/`. A relative path that normalizes to nothing is `.`.
fn normalize(path: &str) -> String {
    // CRAB:BEGIN
let mut components: Vec<&str> = path.split('/').filter(|c| !c.is_empty() && c != ".").collect();
while components.len() > 1 && components.last() == Some(&"..") {
    components.pop();
    components.pop();
}
if components.is_empty() {
    components.push("/");
} else if components.len() == 1 && components[0] == ".." {
    components[0] = ".";
}
components.join("/")
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
        println!("{}", normalize(&line));
    }
}
