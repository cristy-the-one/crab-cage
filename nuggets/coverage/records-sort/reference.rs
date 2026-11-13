use std::cmp::Ordering;

struct Record {
    name: String,
    age: u8,
    city: String,
}

fn order(a: &Record, b: &Record) -> Ordering {
    a.city
        .cmp(&b.city)
        .then(b.age.cmp(&a.age))
        .then_with(|| a.name.cmp(&b.name))
}

fn solve(line: &str) -> String {
    let mut records: Vec<Record> = Vec::new();
    for part in line.split(';') {
        if part.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = part.split(':').collect();
        if fields.len() != 3 {
            return "error".to_string();
        }
        let age = match fields[1].trim().parse::<u8>() {
            Ok(age) if age <= 150 => age,
            _ => return "error".to_string(),
        };
        records.push(Record {
            name: fields[0].trim().to_string(),
            age,
            city: fields[2].trim().to_string(),
        });
    }
    records.sort_by(order);
    let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
    names.join(",")
}
