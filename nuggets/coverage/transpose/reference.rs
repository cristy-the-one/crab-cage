fn solve(line: &str) -> String {
    let mut rows: Vec<Vec<i64>> = Vec::new();
    for row in line.split(';') {
        let mut numbers: Vec<i64> = Vec::new();
        for part in row.split_whitespace() {
            match part.parse() {
                Ok(n) => numbers.push(n),
                Err(_) => return "error".to_string(),
            }
        }
        if numbers.is_empty() {
            return "error".to_string();
        }
        rows.push(numbers);
    }
    let width = rows[0].len();
    if rows.iter().any(|row| row.len() != width) {
        return "error".to_string();
    }
    let mut out: Vec<String> = Vec::new();
    for column in 0..width {
        let parts: Vec<String> = rows.iter().map(|row| row[column].to_string()).collect();
        out.push(parts.join(" "));
    }
    out.join(";")
}
