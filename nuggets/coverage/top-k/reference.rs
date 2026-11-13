use std::collections::BinaryHeap;

fn solve(line: &str) -> String {
    let Some((k, rest)) = line.split_once(':') else {
        return "error".to_string();
    };
    let Ok(k) = k.trim().parse::<usize>() else {
        return "error".to_string();
    };
    let mut heap: BinaryHeap<i64> = BinaryHeap::new();
    for part in rest.split_whitespace() {
        match part.parse() {
            Ok(n) => heap.push(n),
            Err(_) => return "error".to_string(),
        }
    }
    let mut out: Vec<String> = Vec::new();
    while out.len() < k {
        match heap.pop() {
            Some(n) => out.push(n.to_string()),
            None => break,
        }
    }
    out.join(" ")
}
