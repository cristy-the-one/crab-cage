//! One control step. `emit` prints this same arithmetic into the
//! candidate's `fn control` body. The replay test is what keeps them
//! together: if a term is added here, add it there.

use crate::genome::{Genes, LawKind};

pub const RING: usize = 16;
/// Disturbance period, in samples, used by the single-bin tone feature.
/// Scenario A drives coolant flow with a 30-step cycle.
pub const TONE_PERIOD: f64 = 30.0;
const INTEGRAL_LIMIT: f64 = 120.0;

#[derive(Clone, Debug)]
pub struct Memory {
    pub integral: f64,
    pub prev_error: f64,
    pub ring: [f64; RING],
    pub ring_len: usize,
    pub ring_pos: usize,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            integral: 0.0,
            prev_error: 0.0,
            ring: [0.0; RING],
            ring_len: 0,
            ring_pos: 0,
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub heater: f64,
    pub coolant: f64,
    pub predicted_temperature: f64,
}

pub fn push_sample(memory: &mut Memory, sample: f64) {
    memory.ring[memory.ring_pos] = sample;
    memory.ring_pos = (memory.ring_pos + 1) % RING;
    if memory.ring_len < RING {
        memory.ring_len += 1;
    }
}

/// Slope across the window, and the magnitude of one Fourier bin.
pub fn window_features(memory: &Memory, window: usize) -> (f64, f64) {
    let cap = window.clamp(1, RING);
    let n = memory.ring_len.min(cap);
    if n == 0 {
        return (0.0, 0.0);
    }
    let oldest = (memory.ring_pos + RING - n) % RING;
    let mut first = 0.0;
    let mut last = 0.0;
    let mut re = 0.0;
    let mut im = 0.0;
    let omega = std::f64::consts::TAU / TONE_PERIOD;
    for i in 0..n {
        let y = memory.ring[(oldest + i) % RING];
        if i == 0 {
            first = y;
        }
        last = y;
        let angle = omega * (i as f64);
        re += y * angle.cos();
        im += y * angle.sin();
    }
    let slope = if n >= 2 {
        (last - first) / ((n - 1) as f64)
    } else {
        0.0
    };
    let tone = (re * re + im * im).sqrt() / (n as f64);
    (slope, tone)
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub fn step(
    genes: &Genes,
    law: LawKind,
    measured: f64,
    setpoint: f64,
    ambient: f64,
    flow: f64,
    inlet: f64,
    leak: f64,
    memory: &mut Memory,
) -> Output {
    let error = setpoint - measured;
    let derivative = error - memory.prev_error;
    push_sample(memory, measured);
    let (slope, tone) = window_features(memory, genes.window_len());
    let kp = if law.use_feedback() { genes.kp } else { 0.0 };
    let ki = if law.use_integral() { genes.ki } else { 0.0 };
    let kd = if law.use_derivative() { genes.kd } else { 0.0 };
    let k_cool = if law.use_coolant() { genes.k_cool } else { 0.0 };
    let feedforward = if law.use_feedforward() {
        genes.k_amb * (setpoint - ambient)
            + genes.k_flow * flow * (setpoint - inlet)
            + genes.k_leak * leak
    } else {
        0.0
    };
    let trend = if law.use_window() {
        genes.k_trend * slope + genes.k_tone * tone
    } else {
        0.0
    };
    let linear =
        genes.bias + kp * error + ki * memory.integral + kd * derivative + feedforward + trend;

    let (raw, coolant_signal, predicted) = if law.use_model() {
        let heater0 = clamp01(linear);
        let coolant0 = clamp01(k_cool * (measured - setpoint).max(0.0));
        let predicted = measured + genes.pred_heat * heater0
            - genes.pred_cool * coolant0 * (measured - inlet)
            - genes.pred_loss * (measured - ambient)
            - genes.pred_flow * flow * (measured - inlet)
            + genes.pred_leak * leak
            + genes.k_trend * slope;
        let mixed = (1.0 - genes.mix) * error + genes.mix * (setpoint - predicted);
        let raw =
            genes.bias + kp * mixed + ki * memory.integral + kd * derivative + feedforward + trend;
        (raw, predicted - setpoint, predicted)
    } else {
        (linear, measured - setpoint, measured)
    };

    let heater = clamp01(raw);
    let coolant = clamp01(k_cool * coolant_signal.max(0.0));

    if law.use_integral() {
        let integrating =
            (raw >= 0.0 && raw <= 1.0) || (raw > 1.0 && error < 0.0) || (raw < 0.0 && error > 0.0);
        if integrating {
            memory.integral = (memory.integral + error).clamp(-INTEGRAL_LIMIT, INTEGRAL_LIMIT);
        }
    }
    memory.prev_error = error;

    Output {
        heater,
        coolant,
        predicted_temperature: predicted,
    }
}
