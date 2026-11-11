//! Lower a genome to one Rust file.
//!
//! The harness (`main`, the memory struct, the window features) is stable.
//! Only the literals inside `fn control` change, which is the edit shape
//! lust's warm daemon can re-check as a body patch. The file uses `std::io`
//! and arithmetic. It does not use `unsafe`, threads, the filesystem, the
//! network, the environment, or processes.

use crate::control::TONE_PERIOD;
use crate::genome::{rust_f64, Genome};
use crate::plant::{INPUT_NAMES, OUTPUT_NAMES};

pub fn emit(genome: &Genome) -> String {
    let mut out = String::with_capacity(8 * 1024);
    let genome_json = serde_json::to_string(genome).unwrap_or_else(|_| "{}".to_string());
    push(
        &mut out,
        &format!(
            "//! Crab Cage controller for the jacketed-tank demo.\n\
             //! Valid Rust, in the subset lust's sandbox accepts.\n\
             //! Inputs: {inputs}.\n\
             //! Outputs: {outputs}.\n\
             //! crab-cage-genome: {genome_json}\n",
            inputs = INPUT_NAMES.join(", "),
            outputs = OUTPUT_NAMES.join(", "),
        ),
    );
    push(&mut out, HARNESS_HEAD);
    push(&mut out, &control_fn(genome));
    push(&mut out, HARNESS_MAIN);
    out
}

fn push(out: &mut String, text: &str) {
    out.push_str(text);
    if !text.ends_with('\n') {
        out.push('\n');
    }
}

fn control_fn(genome: &Genome) -> String {
    let g = &genome.genes;
    let law = genome.law;
    let use_model = law.use_model();
    let use_integral = law.use_integral();
    let use_feedback = law.use_feedback();
    let use_derivative = law.use_derivative();
    let use_feedforward = law.use_feedforward();
    let use_coolant = law.use_coolant();
    let use_window = law.use_window();
    format!(
        r#"fn control(
    measured: f64,
    setpoint: f64,
    ambient: f64,
    flow: f64,
    inlet: f64,
    leak: f64,
    memory: &mut Memory,
) -> Output {{
    let kp: f64 = {kp};
    let ki: f64 = {ki};
    let kd: f64 = {kd};
    let bias: f64 = {bias};
    let k_amb: f64 = {k_amb};
    let k_flow: f64 = {k_flow};
    let k_leak: f64 = {k_leak};
    let k_cool: f64 = {k_cool};
    let pred_heat: f64 = {pred_heat};
    let pred_loss: f64 = {pred_loss};
    let pred_flow: f64 = {pred_flow};
    let pred_leak: f64 = {pred_leak};
    let pred_cool: f64 = {pred_cool};
    let mix: f64 = {mix};
    let k_trend: f64 = {k_trend};
    let k_tone: f64 = {k_tone};
    let window: usize = {window};
    let use_model: bool = {use_model};
    let use_integral: bool = {use_integral};
    let use_feedback: bool = {use_feedback};
    let use_derivative: bool = {use_derivative};
    let use_feedforward: bool = {use_feedforward};
    let use_coolant: bool = {use_coolant};
    let use_window: bool = {use_window};

    let error = setpoint - measured;
    let derivative = error - memory.prev_error;
    push_sample(memory, measured);
    let (slope, tone) = window_features(memory, window);
    let kp_use = if use_feedback {{ kp }} else {{ 0.0 }};
    let ki_use = if use_integral {{ ki }} else {{ 0.0 }};
    let kd_use = if use_derivative {{ kd }} else {{ 0.0 }};
    let k_cool_use = if use_coolant {{ k_cool }} else {{ 0.0 }};
    let feedforward = if use_feedforward {{
        k_amb * (setpoint - ambient) + k_flow * flow * (setpoint - inlet) + k_leak * leak
    }} else {{
        0.0
    }};
    let trend = if use_window {{
        k_trend * slope + k_tone * tone
    }} else {{
        0.0
    }};
    let linear = bias + kp_use * error + ki_use * memory.integral + kd_use * derivative + feedforward + trend;

    let raw: f64;
    let coolant_signal: f64;
    let predicted: f64;
    if use_model {{
        let heater0 = clamp01(linear);
        let coolant0 = clamp01(k_cool_use * (measured - setpoint).max(0.0));
        predicted = measured
            + pred_heat * heater0
            - pred_cool * coolant0 * (measured - inlet)
            - pred_loss * (measured - ambient)
            - pred_flow * flow * (measured - inlet)
            + pred_leak * leak
            + k_trend * slope;
        let mixed = (1.0 - mix) * error + mix * (setpoint - predicted);
        raw = bias + kp_use * mixed + ki_use * memory.integral + kd_use * derivative + feedforward + trend;
        coolant_signal = predicted - setpoint;
    }} else {{
        raw = linear;
        coolant_signal = measured - setpoint;
        predicted = measured;
    }}

    let heater = clamp01(raw);
    let coolant = clamp01(k_cool_use * coolant_signal.max(0.0));
    if use_integral {{
        let integrating = (raw >= 0.0 && raw <= 1.0)
            || (raw > 1.0 && error < 0.0)
            || (raw < 0.0 && error > 0.0);
        if integrating {{
            memory.integral = clamp_range(memory.integral + error, -120.0, 120.0);
        }}
    }}
    memory.prev_error = error;
    Output {{
        heater,
        coolant,
        predicted_temperature: predicted,
    }}
}}
"#,
        kp = rust_f64(g.kp),
        ki = rust_f64(g.ki),
        kd = rust_f64(g.kd),
        bias = rust_f64(g.bias),
        k_amb = rust_f64(g.k_amb),
        k_flow = rust_f64(g.k_flow),
        k_leak = rust_f64(g.k_leak),
        k_cool = rust_f64(g.k_cool),
        pred_heat = rust_f64(g.pred_heat),
        pred_loss = rust_f64(g.pred_loss),
        pred_flow = rust_f64(g.pred_flow),
        pred_leak = rust_f64(g.pred_leak),
        pred_cool = rust_f64(g.pred_cool),
        mix = rust_f64(g.mix),
        k_trend = rust_f64(g.k_trend),
        k_tone = rust_f64(g.k_tone),
        window = g.window_len(),
        use_model = use_model,
        use_integral = use_integral,
        use_feedback = use_feedback,
        use_derivative = use_derivative,
        use_feedforward = use_feedforward,
        use_coolant = use_coolant,
        use_window = use_window,
    )
}

const HARNESS_HEAD: &str = r#"const RING: usize = 16;
const TONE_PERIOD: f64 = 30.0;

struct Memory {
    integral: f64,
    prev_error: f64,
    ring: [f64; RING],
    ring_len: usize,
    ring_pos: usize,
}

struct Output {
    heater: f64,
    coolant: f64,
    predicted_temperature: f64,
}

fn empty_memory() -> Memory {
    Memory {
        integral: 0.0,
        prev_error: 0.0,
        ring: [0.0; RING],
        ring_len: 0,
        ring_pos: 0,
    }
}

fn push_sample(memory: &mut Memory, sample: f64) {
    memory.ring[memory.ring_pos] = sample;
    memory.ring_pos = (memory.ring_pos + 1) % RING;
    if memory.ring_len < RING {
        memory.ring_len += 1;
    }
}

fn window_features(memory: &Memory, window: usize) -> (f64, f64) {
    let cap = if window < 1 {
        1
    } else if window > RING {
        RING
    } else {
        window
    };
    let n = if memory.ring_len < cap {
        memory.ring_len
    } else {
        cap
    };
    if n == 0 {
        return (0.0, 0.0);
    }
    let oldest = (memory.ring_pos + RING - n) % RING;
    let mut first = 0.0;
    let mut last = 0.0;
    let mut re = 0.0;
    let mut im = 0.0;
    let omega = 6.283185307179586 / TONE_PERIOD;
    let mut i = 0;
    while i < n {
        let y = memory.ring[(oldest + i) % RING];
        if i == 0 {
            first = y;
        }
        last = y;
        let angle = omega * (i as f64);
        re += y * libm_cos(angle);
        im += y * libm_sin(angle);
        i += 1;
    }
    let slope = if n >= 2 {
        (last - first) / ((n - 1) as f64)
    } else {
        0.0
    };
    let tone = (re * re + im * im).sqrt() / (n as f64);
    (slope, tone)
}

fn libm_cos(angle: f64) -> f64 {
    angle.cos()
}

fn libm_sin(angle: f64) -> f64 {
    angle.sin()
}

fn clamp01(value: f64) -> f64 {
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

fn clamp_range(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

"#;

const HARNESS_MAIN: &str = r#"fn parse_input(line: &str, out: &mut [f64; 6]) -> bool {
    let mut n = 0;
    for token in line.split_whitespace() {
        if n >= 6 {
            return false;
        }
        match token.parse::<f64>() {
            Ok(value) if value.is_finite() => {
                out[n] = value;
                n += 1;
            }
            _ => return false,
        }
    }
    n == 6
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut memory = empty_memory();
    let mut line = String::new();
    loop {
        line.clear();
        let read = match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text == "reset" {
            memory = empty_memory();
            let _ = std::io::Write::write_all(&mut stdout, b"ok\n");
            let _ = std::io::Write::flush(&mut stdout);
            continue;
        }
        let mut sample = [0.0; 6];
        if !parse_input(text, &mut sample) {
            let _ = std::io::Write::write_all(&mut stdout, b"err\n");
            let _ = std::io::Write::flush(&mut stdout);
            continue;
        }
        let output = control(
            sample[0],
            sample[1],
            sample[2],
            sample[3],
            sample[4],
            sample[5],
            &mut memory,
        );
        let _ = std::io::Write::write_fmt(
            &mut stdout,
            format_args!(
                "{:.10} {:.10} {:.10}\n",
                output.heater, output.coolant, output.predicted_temperature
            ),
        );
        let _ = std::io::Write::flush(&mut stdout);
    }
}
"#;

/// Patterns that must not appear in a candidate we are willing to execute
/// outside lust's checker. Lust `--sandbox` is the real check when the
/// binary is present; this is the host-side prefilter.
pub fn sandbox_prefilter(source: &str) -> Result<(), String> {
    let stripped = strip_comments(source);
    let needles = [
        "unsafe",
        "async",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
        "std::os",
        "include!",
        "include_str!",
        "include_bytes!",
        "env!",
        "option_env!",
    ];
    for needle in needles {
        if stripped.contains(needle) {
            return Err(format!(
                "candidate contains `{needle}`, which the sandbox refuses"
            ));
        }
    }
    Ok(())
}

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// The tone period literal in the harness must match the host constant.
pub fn tone_period_literal() -> f64 {
    let _ = TONE_PERIOD;
    30.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{Genes, Genome, LawKind};

    #[test]
    fn emitted_source_passes_the_prefilter() {
        let source = emit(&Genome::new(LawKind::Pi, Genes::zeros()));
        sandbox_prefilter(&source).unwrap();
        assert!(source.contains("fn control("));
        assert!(source.contains("fn main()"));
        assert!(!source.contains("unsafe"));
    }

    #[test]
    fn prefilter_catches_filesystem() {
        let err = sandbox_prefilter("fn main() { let _ = std::fs::read(\"a\"); }").unwrap_err();
        assert!(err.contains("std::fs"));
    }
}
