fn solve(line: &str) -> String {
    let Some((k, rest)) = line.split_once(':') else {
        return "error".to_string();
    };
    let k: usize = match k.trim().parse() {
        Ok(k) if k > 0 => k,
        _ => return "error".to_string(),
    };
    let mut numbers: Vec<i64> = Vec::new();
    for part in rest.split_whitespace() {
        match part.parse() {
            Ok(n) => numbers.push(n),
            Err(_) => return "error".to_string(),
        }
    }
    let parts: Vec<String> = numbers
        .windows(k)
        .map(|w| w.iter().copied().max().unwrap_or(0).to_string())
        .collect();
    parts.join(" ")
}
