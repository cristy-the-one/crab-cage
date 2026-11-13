fn solve(line: &str) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 {
        return "error".to_string();
    }
    let from: u32 = match parts[1].parse() {
        Ok(b) if (2..=36).contains(&b) => b,
        _ => return "error".to_string(),
    };
    let to: u32 = match parts[2].parse() {
        Ok(b) if (2..=36).contains(&b) => b,
        _ => return "error".to_string(),
    };
    let Ok(mut value) = u64::from_str_radix(parts[0], from) else {
        return "error".to_string();
    };
    if value == 0 {
        return "0".to_string();
    }
    let mut digits: Vec<char> = Vec::new();
    while value > 0 {
        let digit = (value % u64::from(to)) as u32;
        digits.push(char::from_digit(digit, to).unwrap_or('?'));
        value /= u64::from(to);
    }
    digits.iter().rev().collect()
}
