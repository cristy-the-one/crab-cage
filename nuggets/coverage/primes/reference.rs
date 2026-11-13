fn solve(line: &str) -> String {
    let n: usize = match line.trim().parse() {
        Ok(n) if n <= 500 => n,
        _ => return "error".to_string(),
    };
    let mut composite = vec![false; n + 1];
    let mut found: Vec<String> = Vec::new();
    for i in 2..=n {
        if composite[i] {
            continue;
        }
        found.push(i.to_string());
        let mut j = i * i;
        while j <= n {
            composite[j] = true;
            j += i;
        }
    }
    found.join(" ")
}
