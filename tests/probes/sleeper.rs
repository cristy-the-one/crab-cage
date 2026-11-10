fn main() {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 30 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
