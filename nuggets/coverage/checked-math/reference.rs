fn solve(line: &str) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 {
        return "error".to_string();
    }
    let (Ok(a), Ok(b)) = (parts[0].parse::<i32>(), parts[2].parse::<i32>()) else {
        return "error".to_string();
    };
    let result = match parts[1] {
        "+" => a.checked_add(b),
        "-" => a.checked_sub(b),
        "*" => a.checked_mul(b),
        "/" => {
            if b == 0 {
                return "div0".to_string();
            }
            a.checked_div(b)
        }
        _ => return "error".to_string(),
    };
    match result {
        Some(value) => value.to_string(),
        None => "overflow".to_string(),
    }
}
