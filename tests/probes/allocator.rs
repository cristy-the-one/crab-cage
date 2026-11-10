use std::io::Read;

fn main() {
    let mut byte = [0u8; 1];
    let _ = std::io::stdin().read(&mut byte);
    let mb: usize = std::env::var("PROBE_MB")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(160);
    let mut hold = Vec::new();
    for _ in 0..mb {
        let mut chunk = vec![1u8; 1024 * 1024];
        chunk[0] = 1;
        chunk[1024 * 1024 - 1] = 1;
        hold.push(chunk);
    }
    println!("{}", hold.len());
}
