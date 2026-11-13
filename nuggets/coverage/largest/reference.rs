fn largest<T: PartialOrd + Copy>(items: &[T]) -> Option<T> {
    let mut best = *items.first()?;
    for &item in items {
        if item > best {
            best = item;
        }
    }
    Some(best)
}

fn solve(line: &str) -> String {
    let Some((kind, rest)) = line.split_once(':') else {
        return "error".to_string();
    };
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let answer = match kind.trim() {
        "i" => {
            let mut values: Vec<i64> = Vec::new();
            for part in &parts {
                match part.parse() {
                    Ok(v) => values.push(v),
                    Err(_) => return "error".to_string(),
                }
            }
            largest(&values).map(|v| v.to_string())
        }
        "f" => {
            let mut values: Vec<f64> = Vec::new();
            for part in &parts {
                match part.parse() {
                    Ok(v) => values.push(v),
                    Err(_) => return "error".to_string(),
                }
            }
            largest(&values).map(|v| format!("{v:.1}"))
        }
        "s" => largest(&parts).map(|v| v.to_string()),
        _ => None,
    };
    answer.unwrap_or_else(|| "error".to_string())
}
