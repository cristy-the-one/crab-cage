use std::collections::HashSet;

fn solve(line: &str) -> String {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut kept: Vec<&str> = Vec::new();
    for word in line.split_whitespace() {
        if seen.insert(word) {
            kept.push(word);
        }
    }
    kept.join(" ")
}
