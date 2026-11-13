use std::collections::BTreeMap;

fn solve(line: &str) -> String {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for word in line.split_whitespace() {
        let mut letters: Vec<char> = word.chars().collect();
        letters.sort();
        let key: String = letters.into_iter().collect();
        groups.entry(key).or_default().push(word.to_string());
    }
    let mut lists: Vec<Vec<String>> = groups.into_values().collect();
    for list in lists.iter_mut() {
        list.sort();
    }
    lists.sort_by(|a, b| a[0].cmp(&b[0]));
    let parts: Vec<String> = lists.iter().map(|l| l.join(" ")).collect();
    parts.join(" | ")
}
