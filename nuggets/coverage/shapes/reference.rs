trait Shape {
    fn area(&self) -> f64;
}

struct Circle(f64);
struct Rect(f64, f64);
struct Tri(f64, f64);

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.0 * self.0
    }
}

impl Shape for Rect {
    fn area(&self) -> f64 {
        self.0 * self.1
    }
}

impl Shape for Tri {
    fn area(&self) -> f64 {
        self.0 * self.1 / 2.0
    }
}

fn shape(line: &str) -> Option<Box<dyn Shape>> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    let mut sizes: Vec<f64> = Vec::new();
    for part in parts {
        sizes.push(part.parse().ok()?);
    }
    match (name, sizes.as_slice()) {
        ("circle", [r]) => Some(Box::new(Circle(*r))),
        ("rect", [w, h]) => Some(Box::new(Rect(*w, *h))),
        ("tri", [b, h]) => Some(Box::new(Tri(*b, *h))),
        _ => None,
    }
}

fn solve(line: &str) -> String {
    match shape(line) {
        Some(shape) => format!("{:.2}", shape.area()),
        None => "error".to_string(),
    }
}
