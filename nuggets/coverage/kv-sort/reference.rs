struct Pair {
    key: String,
    value: String,
}

fn solve(line: &str) -> String {
    let mut pairs: Vec<Pair> = Vec::new();
    for part in line.split(';') {
        if let Some((key, value)) = part.split_once('=') {
            pairs.push(Pair {
                key: key.trim().to_string(),
                value: value.trim().to_string(),
            });
        }
    }
    pairs.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.value.cmp(&b.value)));
    let parts: Vec<String> = pairs
        .iter()
        .map(|p| format!("{}={}", p.key, p.value))
        .collect();
    parts.join(";")
}
