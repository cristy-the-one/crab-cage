use std::collections::HashMap;

fn solve(line: &str) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for word in line.split_whitespace() {
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let parts: Vec<String> = pairs.iter().map(|(w, c)| format!("{w}:{c}")).collect();
    parts.join(" ")
}
