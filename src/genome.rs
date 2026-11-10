use serde::{Deserialize, Serialize};

use crate::rng::Pcg64;

/// Which terms of the fixed control law are active.
///
/// The numeric genes always exist. A law that does not mention a term
/// leaves that gene at zero, so the emitted Rust stays one function body
/// and only the literals change. That is the shape lust can re-check as a
/// body patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LawKind {
    /// Constant bias, prediction copies the measurement.
    Hold,
    /// Proportional feedback.
    Proportional,
    /// Proportional plus integral.
    Pi,
    /// PI plus derivative and a coolant valve on positive error.
    Pid,
    /// PI plus disturbance feedforward and the coolant valve.
    PiFeedforward,
    /// Feedforward, a one-step linear predictor, and window features.
    PiFeedforwardPredict,
}

impl LawKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Proportional => "proportional",
            Self::Pi => "pi",
            Self::Pid => "pid",
            Self::PiFeedforward => "pi_feedforward",
            Self::PiFeedforwardPredict => "pi_feedforward_predict",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "hold" => Some(Self::Hold),
            "proportional" | "p" => Some(Self::Proportional),
            "pi" => Some(Self::Pi),
            "pid" => Some(Self::Pid),
            "pi_feedforward" | "pi_ff" => Some(Self::PiFeedforward),
            "pi_feedforward_predict" | "pi_ff_pred" => Some(Self::PiFeedforwardPredict),
            _ => None,
        }
    }

    pub fn use_integral(self) -> bool {
        matches!(
            self,
            Self::Pi | Self::Pid | Self::PiFeedforward | Self::PiFeedforwardPredict
        )
    }

    pub fn use_feedback(self) -> bool {
        !matches!(self, Self::Hold)
    }

    pub fn use_derivative(self) -> bool {
        matches!(self, Self::Pid)
    }

    pub fn use_feedforward(self) -> bool {
        matches!(self, Self::PiFeedforward | Self::PiFeedforwardPredict)
    }

    pub fn use_coolant(self) -> bool {
        matches!(
            self,
            Self::Pid | Self::PiFeedforward | Self::PiFeedforwardPredict
        )
    }

    pub fn use_window(self) -> bool {
        matches!(self, Self::PiFeedforwardPredict)
    }

    pub fn use_model(self) -> bool {
        matches!(self, Self::PiFeedforwardPredict)
    }
}

impl std::fmt::Display for LawKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Continuous parameters of a candidate. Every field is a finite `f64`
/// inside [`BOUNDS`]. `window` is rounded to an integer length in 2..=16
/// when the controller reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Genes {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
    pub bias: f64,
    pub k_amb: f64,
    pub k_flow: f64,
    pub k_leak: f64,
    pub k_cool: f64,
    pub pred_heat: f64,
    pub pred_loss: f64,
    pub pred_flow: f64,
    pub pred_leak: f64,
    pub pred_cool: f64,
    pub mix: f64,
    pub k_trend: f64,
    pub k_tone: f64,
    pub window: f64,
}

#[derive(Clone, Copy)]
pub struct Bound {
    pub lo: f64,
    pub hi: f64,
}

/// Inclusive ranges. The order is the mutation order.
pub const BOUNDS: &[(&str, Bound)] = &[
    ("kp", Bound { lo: 0.0, hi: 0.8 }),
    ("ki", Bound { lo: 0.0, hi: 0.2 }),
    ("kd", Bound { lo: 0.0, hi: 0.25 }),
    ("bias", Bound { lo: -0.05, hi: 0.4 }),
    ("k_amb", Bound { lo: 0.0, hi: 0.08 }),
    ("k_flow", Bound { lo: 0.0, hi: 0.2 }),
    ("k_leak", Bound { lo: -0.5, hi: 0.1 }),
    ("k_cool", Bound { lo: 0.0, hi: 1.2 }),
    ("pred_heat", Bound { lo: 0.0, hi: 4.0 }),
    ("pred_loss", Bound { lo: 0.0, hi: 0.1 }),
    ("pred_flow", Bound { lo: 0.0, hi: 0.25 }),
    ("pred_leak", Bound { lo: -0.2, hi: 1.0 }),
    ("pred_cool", Bound { lo: 0.0, hi: 2.0 }),
    ("mix", Bound { lo: 0.0, hi: 1.0 }),
    ("k_trend", Bound { lo: -0.5, hi: 0.5 }),
    ("k_tone", Bound { lo: -0.5, hi: 0.5 }),
    ("window", Bound { lo: 2.0, hi: 16.0 }),
];

impl Genes {
    pub fn zeros() -> Self {
        Self {
            kp: 0.0,
            ki: 0.0,
            kd: 0.0,
            bias: 0.0,
            k_amb: 0.0,
            k_flow: 0.0,
            k_leak: 0.0,
            k_cool: 0.0,
            pred_heat: 0.0,
            pred_loss: 0.0,
            pred_flow: 0.0,
            pred_leak: 0.0,
            pred_cool: 0.0,
            mix: 0.0,
            k_trend: 0.0,
            k_tone: 0.0,
            window: 8.0,
        }
    }

    pub fn window_len(&self) -> usize {
        self.window.round().clamp(2.0, 16.0) as usize
    }

    fn as_mut_fields(&mut self) -> [&mut f64; 17] {
        [
            &mut self.kp,
            &mut self.ki,
            &mut self.kd,
            &mut self.bias,
            &mut self.k_amb,
            &mut self.k_flow,
            &mut self.k_leak,
            &mut self.k_cool,
            &mut self.pred_heat,
            &mut self.pred_loss,
            &mut self.pred_flow,
            &mut self.pred_leak,
            &mut self.pred_cool,
            &mut self.mix,
            &mut self.k_trend,
            &mut self.k_tone,
            &mut self.window,
        ]
    }

    pub fn clamp(&mut self) {
        for ((_, bound), slot) in BOUNDS.iter().zip(self.as_mut_fields()) {
            if !slot.is_finite() {
                *slot = 0.0;
            }
            *slot = slot.clamp(bound.lo, bound.hi);
        }
    }

    pub fn mutate(&mut self, rng: &mut Pcg64, sigma: f64) {
        for ((_, bound), slot) in BOUNDS.iter().zip(self.as_mut_fields()) {
            if rng.chance(0.65) {
                let span = bound.hi - bound.lo;
                *slot += rng.gauss() * sigma * span;
            }
        }
        self.clamp();
    }

    /// Blend crossover. `alpha` is the weight of `self`.
    pub fn crossover(&self, other: &Self, alpha: f64) -> Self {
        let mut child = self.clone();
        let a = alpha.clamp(0.0, 1.0);
        let b = 1.0 - a;
        let left = self.as_slice();
        let right = other.as_slice();
        for (slot, (l, r)) in child
            .as_mut_fields()
            .into_iter()
            .zip(left.into_iter().zip(right))
        {
            *slot = a * l + b * r;
        }
        child.clamp();
        child
    }

    fn as_slice(&self) -> [f64; 17] {
        [
            self.kp,
            self.ki,
            self.kd,
            self.bias,
            self.k_amb,
            self.k_flow,
            self.k_leak,
            self.k_cool,
            self.pred_heat,
            self.pred_loss,
            self.pred_flow,
            self.pred_leak,
            self.pred_cool,
            self.mix,
            self.k_trend,
            self.k_tone,
            self.window,
        ]
    }

    pub fn set_named(&mut self, name: &str, value: f64) -> bool {
        let Some(index) = BOUNDS.iter().position(|(n, _)| *n == name) else {
            return false;
        };
        {
            let fields = self.as_mut_fields();
            *fields[index] = value;
        }
        self.clamp();
        true
    }

    pub fn floor_named(&mut self, name: &str, value: f64, direction: Floor) -> bool {
        let Some(index) = BOUNDS.iter().position(|(n, _)| *n == name) else {
            return false;
        };
        {
            let fields = self.as_mut_fields();
            let current = *fields[index];
            *fields[index] = match direction {
                Floor::AtLeast => current.max(value),
                Floor::AtMost => current.min(value),
                Floor::Set => value,
            };
        }
        self.clamp();
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Floor {
    AtLeast,
    AtMost,
    Set,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Genome {
    pub law: LawKind,
    pub genes: Genes,
}

impl Genome {
    pub fn new(law: LawKind, mut genes: Genes) -> Self {
        genes.clamp();
        Self { law, genes }
    }
}

/// A round-trip Rust literal for a finite `f64`.
pub fn rust_f64(value: f64) -> String {
    if !value.is_finite() {
        return "0.0".to_string();
    }
    let text = format!("{value:?}");
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{text}.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_round_trip() {
        for value in [0.0, 1.0, -1.0, 0.02, 0.1, -0.075, 1.8, 1e-4, 12.0] {
            let literal = rust_f64(value);
            let parsed: f64 = literal.parse().unwrap();
            assert_eq!(parsed.to_bits(), value.to_bits(), "{literal}");
        }
    }

    #[test]
    fn clamp_rejects_nan() {
        let mut genes = Genes::zeros();
        genes.kp = f64::NAN;
        genes.clamp();
        assert_eq!(genes.kp, 0.0);
    }
}
