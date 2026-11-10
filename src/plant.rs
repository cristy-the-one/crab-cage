//! Jacketed tank used as the built-in demo.
//!
//! One temperature, two actuators (heater duty and coolant valve), and a
//! handful of measured disturbances. The candidate never sees these
//! coefficients. It sees the sample vector documented in [`INPUT_NAMES`].

use crate::rng::Pcg64;

pub const DT: f64 = 1.0;
pub const K_HEAT: f64 = 1.8;
pub const K_COOL: f64 = 0.9;
pub const K_LOSS: f64 = 0.02;
pub const K_FLOW: f64 = 0.06;
pub const K_LEAK: f64 = 0.45;
pub const HORIZON: usize = 280;

pub const INPUT_NAMES: &[&str] = &[
    "measured_temperature",
    "setpoint",
    "ambient",
    "coolant_flow",
    "inlet_temperature",
    "leak",
];

pub const OUTPUT_NAMES: &[&str] = &["heater", "coolant_valve", "predicted_temperature"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioKind {
    /// Setpoint steps, periodic coolant flow, a mild ambient swing.
    Nominal,
    /// Ambient, flow, inlet, and leak all drift. The trend-shift case.
    TrendShift,
}

#[derive(Clone, Copy, Debug)]
pub struct Disturbance {
    pub setpoint: f64,
    pub ambient: f64,
    pub flow: f64,
    pub inlet: f64,
    pub leak: f64,
    pub noise: f64,
}

pub fn disturbance(kind: ScenarioKind, step: usize, rng: &mut Pcg64) -> Disturbance {
    let t = step as f64;
    let (setpoint, ambient, flow, inlet, leak) = match kind {
        ScenarioKind::Nominal => {
            let setpoint = if step < 90 {
                58.0
            } else if step < 180 {
                70.0
            } else {
                62.0
            };
            let ambient = 21.0 + 1.5 * (std::f64::consts::TAU * t / 140.0).sin();
            let flow = 0.4 + 0.35 * (std::f64::consts::TAU * t / 30.0).sin();
            let inlet = 16.0 + 0.8 * (std::f64::consts::TAU * t / 70.0).sin();
            let leak = 0.9 * (std::f64::consts::TAU * t / 37.0).sin();
            (setpoint, ambient, flow, inlet, leak)
        }
        ScenarioKind::TrendShift => {
            let setpoint = 64.0;
            let ambient = 17.0 + 0.045 * t;
            let flow = 0.25 + 0.003 * t;
            let inlet = 15.0 + 0.012 * t;
            let leak = 0.35 * (std::f64::consts::TAU * t / 28.0).sin() + 0.008 * t;
            (setpoint, ambient, flow, inlet, leak)
        }
    };
    Disturbance {
        setpoint,
        ambient,
        flow: flow.max(0.05),
        inlet,
        leak,
        noise: rng.gauss() * 0.2,
    }
}

pub fn scenario_seed(kind: ScenarioKind) -> u64 {
    match kind {
        ScenarioKind::Nominal => 1,
        ScenarioKind::TrendShift => 2,
    }
}

#[derive(Clone, Debug)]
pub struct Plant {
    pub temperature: f64,
    /// One-step delayed temperature. The controller measures this plus noise.
    pub delayed: f64,
}

impl Plant {
    pub fn new() -> Self {
        Self {
            temperature: 28.0,
            delayed: 28.0,
        }
    }

    pub fn measured(&self, noise: f64) -> f64 {
        self.delayed + noise
    }

    pub fn advance(&mut self, heater: f64, coolant: f64, sample: &Disturbance) -> f64 {
        let t = self.temperature;
        let delta = DT
            * (K_HEAT * heater
                - K_COOL * coolant * (t - sample.inlet)
                - K_LOSS * (t - sample.ambient)
                - K_FLOW * sample.flow * (t - sample.inlet)
                + K_LEAK * sample.leak);
        let next = t + delta;
        self.delayed = t;
        self.temperature = next;
        next
    }
}

impl Default for Plant {
    fn default() -> Self {
        Self::new()
    }
}
