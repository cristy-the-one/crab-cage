fn solve(line: &str) -> String {
    let kept: Vec<char> = line
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let n = kept.len();
    for i in 0..n / 2 {
        if kept[i] != kept[n - 1 - i] {
            return "no".to_string();
        }
    }
    "yes".to_string()
}
