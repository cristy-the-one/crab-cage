fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn solve(line: &str) -> String {
    let mut numbers: Vec<u64> = Vec::new();
    for part in line.split_whitespace() {
        match part.parse::<u64>() {
            Ok(n) if n > 0 => numbers.push(n),
            _ => return "error".to_string(),
        }
    }
    let Some(&first) = numbers.first() else {
        return "error".to_string();
    };
    let g = numbers.iter().fold(first, |acc, &n| gcd(acc, n));
    let l = numbers.iter().fold(first, |acc, &n| acc / gcd(acc, n) * n);
    format!("{g} {l}")
}
