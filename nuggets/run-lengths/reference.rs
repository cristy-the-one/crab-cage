    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        let mut count = 1usize;
        while chars.peek() == Some(&ch) {
            chars.next();
            count += 1;
        }
        if count > 1 || ch.is_ascii_digit() {
            out.push_str(&count.to_string());
        }
        out.push(ch);
    }
    out
