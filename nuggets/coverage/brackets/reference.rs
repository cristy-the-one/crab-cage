fn solve(line: &str) -> String {
    let mut stack: Vec<char> = Vec::new();
    for (i, c) in line.chars().enumerate() {
        match c {
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let want = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if stack.pop() != Some(want) {
                    return format!("bad {i}");
                }
            }
            _ => {}
        }
    }
    if stack.is_empty() {
        "ok".to_string()
    } else {
        format!("open {}", stack.len())
    }
}
