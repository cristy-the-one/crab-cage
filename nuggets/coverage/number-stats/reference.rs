fn solve(line: &str) -> String {
    let mut numbers: Vec<i64> = Vec::new();
    for part in line.split_whitespace() {
        match part.parse() {
            Ok(n) => numbers.push(n),
            Err(_) => return "error".to_string(),
        }
    }
    let evens: i64 = numbers.iter().filter(|n| *n % 2 == 0).map(|n| n * n).sum();
    let steps: Vec<String> = numbers
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).to_string())
        .collect();
    let steps = if steps.is_empty() {
        "-".to_string()
    } else {
        steps.join(",")
    };
    let neg = match numbers.iter().position(|n| *n < 0) {
        Some(at) => at.to_string(),
        None => "-".to_string(),
    };
    format!("evens={evens} steps={steps} neg={neg}")
}
