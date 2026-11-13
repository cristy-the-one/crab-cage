#[derive(Clone, Copy, PartialEq)]
enum State {
    Locked,
    Unlocked,
}

fn solve(line: &str) -> String {
    let mut state = State::Locked;
    let mut passed = 0u32;
    let mut coins = 0u32;
    for event in line.chars() {
        state = match (state, event) {
            (State::Locked, 'c') => {
                coins += 1;
                State::Unlocked
            }
            (State::Locked, 'p') => State::Locked,
            (State::Unlocked, 'p') => {
                passed += 1;
                State::Locked
            }
            (State::Unlocked, 'c') => {
                coins += 1;
                State::Unlocked
            }
            _ => return "error".to_string(),
        };
    }
    let name = if state == State::Locked {
        "locked"
    } else {
        "unlocked"
    };
    format!("{name} passed={passed} coins={coins}")
}
