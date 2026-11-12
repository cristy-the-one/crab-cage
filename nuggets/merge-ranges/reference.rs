    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for part in line.split_whitespace() {
        let mut ends = part.split('-');
        let lo = ends.next().and_then(|text| text.parse::<u64>().ok());
        let hi = ends.next().and_then(|text| text.parse::<u64>().ok());
        if let (Some(lo), Some(hi)) = (lo, hi) {
            ranges.push((lo, hi));
        }
    }
    ranges.sort();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (lo, hi) in ranges {
        match merged.last_mut() {
            Some(last) if lo <= last.1 + 1 => {
                if hi > last.1 {
                    last.1 = hi;
                }
            }
            _ => merged.push((lo, hi)),
        }
    }
    let parts: Vec<String> = merged.iter().map(|(lo, hi)| format!("{lo}-{hi}")).collect();
    parts.join(" ")
