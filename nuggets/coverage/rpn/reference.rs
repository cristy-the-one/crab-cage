enum Token {
    Number(i64),
    Op(char),
}

fn parse(part: &str) -> Option<Token> {
    match part {
        "+" | "-" | "*" | "/" => part.chars().next().map(Token::Op),
        _ => part.parse::<i64>().ok().map(Token::Number),
    }
}

fn eval(line: &str) -> Option<i64> {
    let mut stack: Vec<i64> = Vec::new();
    for part in line.split_whitespace() {
        match parse(part)? {
            Token::Number(n) => stack.push(n),
            Token::Op(op) => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let value = match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    _ => {
                        if b == 0 {
                            return None;
                        }
                        a / b
                    }
                };
                stack.push(value);
            }
        }
    }
    if stack.len() == 1 {
        stack.pop()
    } else {
        None
    }
}

fn solve(line: &str) -> String {
    match eval(line) {
        Some(value) => value.to_string(),
        None => "error".to_string(),
    }
}
