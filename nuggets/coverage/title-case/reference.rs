fn solve(line: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for word in line.split_whitespace() {
        let mut chars = word.chars();
        let mut out = String::new();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            let rest: String = chars.collect();
            out.push_str(&rest.to_lowercase());
        }
        words.push(out);
    }
    words.join(" ")
}
