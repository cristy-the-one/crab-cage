fn value(c: char) -> Option<u32> {
    match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    }
}

fn solve(line: &str) -> String {
    if line.is_empty() {
        return "error".to_string();
    }
    let mut total: u32 = 0;
    let mut prev: u32 = 0;
    for c in line.chars().rev() {
        let Some(v) = value(c) else {
            return "error".to_string();
        };
        if v < prev {
            total -= v;
        } else {
            total += v;
            prev = v;
        }
    }
    total.to_string()
}
