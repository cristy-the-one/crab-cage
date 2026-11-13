fn leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn solve(line: &str) -> String {
    let parts: Vec<&str> = line.trim().split('-').collect();
    if parts.len() != 3 {
        return "error".to_string();
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].parse::<u32>(),
        parts[1].parse::<usize>(),
        parts[2].parse::<u32>(),
    ) else {
        return "error".to_string();
    };
    let mut days = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if leap(year) {
        days[1] = 29;
    }
    if month < 1 || month > 12 || day < 1 || day > days[month - 1] {
        return "error".to_string();
    }
    let before: u32 = days[..month - 1].iter().sum();
    (before + day).to_string()
}
