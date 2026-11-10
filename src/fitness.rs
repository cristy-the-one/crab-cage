//! Closed-loop fitness for the jacketed tank.
//!
//! Lower is better. The scalar is a weighted sum. The Pareto archive keeps
//! the four objectives separate: nominal tracking, trend-shift tracking,
//! prediction error, and actuator movement.

use serde::{Deserialize, Serialize};

use crate::control::{self, Memory, Output};
use crate::genome::Genome;
use crate::plant::{self, Disturbance, Plant, ScenarioKind, HORIZON};
use crate::rng::Pcg64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TracePoint {
    pub step: usize,
    pub temperature: f64,
    pub setpoint: f64,
    pub measured: f64,
    pub heater: f64,
    pub coolant: f64,
    pub predicted: f64,
    pub actual_next: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioScore {
    pub tracking_mse: f64,
    pub effort: f64,
    pub prediction_mse: f64,
    pub overshoot: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<TracePoint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fitness {
    pub score: f64,
    pub nominal: ScenarioScore,
    pub trend_shift: ScenarioScore,
}

#[derive(Clone, Copy, Debug)]
pub struct Weights {
    pub nominal_tracking: f64,
    pub trend_tracking: f64,
    pub effort: f64,
    pub nominal_prediction: f64,
    pub trend_prediction: f64,
    pub nominal_overshoot: f64,
    pub trend_overshoot: f64,
}

impl Weights {
    pub fn demo() -> Self {
        Self {
            nominal_tracking: 1.0,
            trend_tracking: 0.65,
            effort: 4.0,
            nominal_prediction: 0.35,
            trend_prediction: 0.2,
            nominal_overshoot: 0.4,
            trend_overshoot: 0.25,
        }
    }
}

impl Fitness {
    pub fn objectives(&self) -> [f64; 4] {
        [
            self.nominal.tracking_mse,
            self.trend_shift.tracking_mse,
            self.nominal.prediction_mse,
            self.nominal.effort,
        ]
    }

    pub fn dominates(&self, other: &Self) -> bool {
        let a = self.objectives();
        let b = other.objectives();
        let mut strictly = false;
        for (left, right) in a.iter().zip(b.iter()) {
            if left > right {
                return false;
            }
            if left < right {
                strictly = true;
            }
        }
        strictly
    }
}

pub fn evaluate(genome: &Genome) -> Fitness {
    evaluate_with(genome, false)
}

pub fn evaluate_with_trace(genome: &Genome) -> Fitness {
    evaluate_with(genome, true)
}

fn evaluate_with(genome: &Genome, trace: bool) -> Fitness {
    let weights = Weights::demo();
    let nominal = rollout(genome, ScenarioKind::Nominal, trace);
    let trend_shift = rollout(genome, ScenarioKind::TrendShift, trace);
    let score = weights.nominal_tracking * nominal.tracking_mse
        + weights.trend_tracking * trend_shift.tracking_mse
        + weights.effort * nominal.effort
        + weights.nominal_prediction * nominal.prediction_mse
        + weights.trend_prediction * trend_shift.prediction_mse
        + weights.nominal_overshoot * nominal.overshoot
        + weights.trend_overshoot * trend_shift.overshoot;
    Fitness {
        score: if score.is_finite() {
            score
        } else {
            f64::MAX / 4.0
        },
        nominal,
        trend_shift,
    }
}

pub fn rollout(genome: &Genome, kind: ScenarioKind, trace: bool) -> ScenarioScore {
    let mut plant = Plant::new();
    let mut memory = Memory::new();
    let mut rng = Pcg64::new(plant::scenario_seed(kind), 11);
    let mut tracking = 0.0;
    let mut effort = 0.0;
    let mut prediction = 0.0;
    let mut overshoot = 0.0;
    let mut prev_heater = 0.0;
    let mut prev_coolant = 0.0;
    let mut points = Vec::new();

    for step in 0..HORIZON {
        let sample = plant::disturbance(kind, step, &mut rng);
        let measured = plant.measured(sample.noise);
        let output = control::step(
            &genome.genes,
            genome.law,
            measured,
            sample.setpoint,
            sample.ambient,
            sample.flow,
            sample.inlet,
            sample.leak,
            &mut memory,
        );
        let next = plant.advance(output.heater, output.coolant, &sample);
        let temperature = plant.delayed;
        tracking += (temperature - sample.setpoint).powi(2);
        effort += (output.heater - prev_heater).powi(2) + (output.coolant - prev_coolant).powi(2);
        prediction += (output.predicted_temperature - next).powi(2);
        let excess = temperature - (sample.setpoint + 1.5);
        if excess > 0.0 {
            overshoot += excess.powi(2);
        }
        if trace && (step % 5 == 0 || step + 1 == HORIZON) {
            points.push(TracePoint {
                step,
                temperature,
                setpoint: sample.setpoint,
                measured,
                heater: output.heater,
                coolant: output.coolant,
                predicted: output.predicted_temperature,
                actual_next: next,
            });
        }
        prev_heater = output.heater;
        prev_coolant = output.coolant;
    }

    let n = HORIZON as f64;
    ScenarioScore {
        tracking_mse: tracking / n,
        effort: effort / n,
        prediction_mse: prediction / n,
        overshoot: overshoot / n,
        trace: points,
    }
}

/// Drive an external controller that speaks the line protocol.
/// `exchange` sends one input line and returns the controller's output line.
pub fn evaluate_exchange<F>(exchange: &mut F) -> Fitness
where
    F: FnMut(ScenarioKind, &Disturbance, f64) -> Option<Output>,
{
    let weights = Weights::demo();
    let nominal = rollout_exchange(ScenarioKind::Nominal, exchange);
    let trend_shift = rollout_exchange(ScenarioKind::TrendShift, exchange);
    let score = match (&nominal, &trend_shift) {
        (Some(nominal), Some(trend_shift)) => {
            weights.nominal_tracking * nominal.tracking_mse
                + weights.trend_tracking * trend_shift.tracking_mse
                + weights.effort * nominal.effort
                + weights.nominal_prediction * nominal.prediction_mse
                + weights.trend_prediction * trend_shift.prediction_mse
                + weights.nominal_overshoot * nominal.overshoot
                + weights.trend_overshoot * trend_shift.overshoot
        }
        _ => f64::MAX / 4.0,
    };
    Fitness {
        score,
        nominal: nominal.unwrap_or(failed_scenario()),
        trend_shift: trend_shift.unwrap_or(failed_scenario()),
    }
}

fn failed_scenario() -> ScenarioScore {
    ScenarioScore {
        tracking_mse: 1.0e6,
        effort: 1.0e6,
        prediction_mse: 1.0e6,
        overshoot: 1.0e6,
        trace: Vec::new(),
    }
}

fn rollout_exchange<F>(kind: ScenarioKind, exchange: &mut F) -> Option<ScenarioScore>
where
    F: FnMut(ScenarioKind, &Disturbance, f64) -> Option<Output>,
{
    let mut plant = Plant::new();
    let mut rng = Pcg64::new(plant::scenario_seed(kind), 11);
    let mut tracking = 0.0;
    let mut effort = 0.0;
    let mut prediction = 0.0;
    let mut overshoot = 0.0;
    let mut prev_heater = 0.0;
    let mut prev_coolant = 0.0;
    for step in 0..HORIZON {
        let sample = plant::disturbance(kind, step, &mut rng);
        let measured = plant.measured(sample.noise);
        let output = exchange(kind, &sample, measured)?;
        let next = plant.advance(output.heater, output.coolant, &sample);
        let temperature = plant.delayed;
        tracking += (temperature - sample.setpoint).powi(2);
        effort += (output.heater - prev_heater).powi(2) + (output.coolant - prev_coolant).powi(2);
        prediction += (output.predicted_temperature - next).powi(2);
        let excess = temperature - (sample.setpoint + 1.5);
        if excess > 0.0 {
            overshoot += excess.powi(2);
        }
        prev_heater = output.heater;
        prev_coolant = output.coolant;
    }
    let n = HORIZON as f64;
    Some(ScenarioScore {
        tracking_mse: tracking / n,
        effort: effort / n,
        prediction_mse: prediction / n,
        overshoot: overshoot / n,
        trace: Vec::new(),
    })
}

pub fn format_input_line(measured: f64, sample: &Disturbance) -> String {
    format!(
        "{:.10} {:.10} {:.10} {:.10} {:.10} {:.10}",
        measured, sample.setpoint, sample.ambient, sample.flow, sample.inlet, sample.leak
    )
}

pub fn parse_output_line(line: &str) -> Option<Output> {
    let mut values = [0.0; 3];
    let mut n = 0;
    for token in line.split_whitespace() {
        if n >= 3 {
            return None;
        }
        let value: f64 = token.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        values[n] = value;
        n += 1;
    }
    if n != 3 {
        return None;
    }
    Some(Output {
        heater: values[0],
        coolant: values[1],
        predicted_temperature: values[2],
    })
}

#[derive(Clone, Debug)]
pub struct IoPair {
    pub input: String,
    pub output: Output,
}

/// Inputs the native controller saw, paired with the outputs it produced.
/// Replaying the input lines is an open-loop check of `fn control`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{Genes, Genome, LawKind};

    #[test]
    fn integral_control_beats_a_weak_proportional() {
        let mut weak_genes = Genes::zeros();
        weak_genes.kp = 0.02;
        let weak = evaluate(&Genome::new(LawKind::Proportional, weak_genes));
        let mut pi_genes = Genes::zeros();
        pi_genes.kp = 0.15;
        pi_genes.ki = 0.035;
        pi_genes.bias = 0.02;
        let pi = evaluate(&Genome::new(LawKind::Pi, pi_genes));
        assert!(
            weak.score > 400.0,
            "weak proportional should miss the setpoint, score {}",
            weak.score
        );
        assert!(
            pi.score < weak.score * 0.5,
            "pi {} should beat proportional {}",
            pi.score,
            weak.score
        );
    }
}

pub fn recorded_pairs(genome: &Genome, kind: ScenarioKind) -> Vec<IoPair> {
    let mut plant = Plant::new();
    let mut memory = Memory::new();
    let mut rng = Pcg64::new(plant::scenario_seed(kind), 11);
    let mut pairs = Vec::with_capacity(HORIZON);
    for step in 0..HORIZON {
        let sample = plant::disturbance(kind, step, &mut rng);
        let measured = plant.measured(sample.noise);
        let output = control::step(
            &genome.genes,
            genome.law,
            measured,
            sample.setpoint,
            sample.ambient,
            sample.flow,
            sample.inlet,
            sample.leak,
            &mut memory,
        );
        pairs.push(IoPair {
            input: format_input_line(measured, &sample),
            output,
        });
        let _ = plant.advance(output.heater, output.coolant, &sample);
    }
    pairs
}
