fn solve(line: &str) -> String {
    let Ok(n) = line.trim().parse::<u32>() else {
        return "error".to_string();
    };
    format!(
        "ones={} lead={} trail={} swapped={:08x}",
        n.count_ones(),
        n.leading_zeros(),
        n.trailing_zeros(),
        n.swap_bytes()
    )
}
