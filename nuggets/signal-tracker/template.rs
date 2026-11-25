use std::io::{self, BufRead};

// Online one-step-ahead sensor predictor.
//
// Each input line is `measured next_true`:
// - On sample 0, `state[0]` is initialized to `measured`.
// - On each sample, `predict(measured, state)` sees ONLY `measured` (noisy +
//   isolated +-35 glitch spikes) and `state: &mut [f64; 8]`, and returns its
//   forecast of `next_true`.
// - Case score is the sum of squared forecast errors `(pred - next_true)^2`.
//   Lower is better.
//
// Regimes across `cases/`:
// - `01-spikes`: flat levels with isolated +-35 glitch spikes and one step change.
// - `02-ramp`: steady +3/step linear ramp with small sensor noise.
// - `03-mixed`: ramps (+2.5/step, -2.0/step), plateaus, and +-37 glitch spikes.
fn predict(measured: f64, state: &mut [f64; 8]) -> f64 {
    // CRAB:BEGIN
    let alpha = 0.12;
    let lo = -50.0;
    let hi = 50.0;
    let delta = (measured - state[0]).max(lo).min(hi);
    state[0] += alpha * delta;
    state[0]
    // CRAB:END
}

fn main() {
    let stdin = io::stdin();
    let mut state = [0.0f64; 8];
    let mut sq_err = 0.0f64;
    let mut first = true;

    for line in stdin.lock().lines() {
        let Ok(raw) = line else {
            std::process::exit(1);
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(m_str) = parts.next() else {
            std::process::exit(1);
        };
        let Some(t_str) = parts.next() else {
            std::process::exit(1);
        };
        let (Ok(measured), Ok(next_true)) = (m_str.parse::<f64>(), t_str.parse::<f64>()) else {
            std::process::exit(1);
        };
        if first {
            state[0] = measured;
            first = false;
        }
        let pred = predict(measured, &mut state);
        if !pred.is_finite() {
            std::process::exit(1);
        }
        let err = pred - next_true;
        sq_err += err * err;
    }

    println!("score: {:.2}", sq_err);
}
