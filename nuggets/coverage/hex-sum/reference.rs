fn sum(line: &str) -> Result<u64, std::num::ParseIntError> {
    let mut total: u64 = 0;
    for part in line.split_whitespace() {
        total += u64::from(u8::from_str_radix(part, 16)?);
    }
    Ok(total)
}

fn solve(line: &str) -> String {
    match sum(line) {
        Ok(total) => total.to_string(),
        Err(_) => "error".to_string(),
    }
}
