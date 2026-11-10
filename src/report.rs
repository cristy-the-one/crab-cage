use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::fitness::Fitness;
use crate::ga::{Evolution, GenerationRecord};
use crate::genome::Genome;

#[derive(Serialize)]
pub struct Report {
    pub schema: u32,
    pub tool: &'static str,
    pub demo: &'static str,
    pub seed: u64,
    pub backend: String,
    pub population: usize,
    pub generations: usize,
    pub evaluations: usize,
    pub best: BestView,
    pub curve: Vec<GenerationRecord>,
    pub pareto: Vec<ParetoView>,
    pub trace_preview: Vec<crate::fitness::TracePoint>,
    pub verification: Verification,
    pub limits: LimitView,
}

#[derive(Serialize)]
pub struct BestView {
    pub fitness: f64,
    pub law: String,
    pub genes: crate::genome::Genes,
    pub nominal_tracking_mse: f64,
    pub trend_shift_tracking_mse: f64,
    pub nominal_prediction_mse: f64,
    pub trend_shift_prediction_mse: f64,
    pub effort: f64,
    pub overshoot: f64,
}

#[derive(Serialize)]
pub struct ParetoView {
    pub fitness: f64,
    pub law: String,
    pub nominal_tracking_mse: f64,
    pub trend_shift_tracking_mse: f64,
    pub prediction_mse: f64,
    pub effort: f64,
}

#[derive(Serialize)]
pub struct Verification {
    pub rustc: String,
    pub max_abs_error: Option<f64>,
    pub lust: String,
    pub lust_version: Option<String>,
    pub lust_max_memory: bool,
    pub lust_time_limit: bool,
    pub lust_check: String,
}

#[derive(Serialize)]
pub struct LimitView {
    pub run_timeout_ms: u64,
    pub lust_max_memory_bytes: u64,
    pub host_memory_bytes: u64,
    pub note: &'static str,
}

pub fn build_report(
    evolution: &Evolution,
    detailed: &Fitness,
    seed: u64,
    backend: &str,
    population: usize,
    generations: usize,
    verification: Verification,
    limits: LimitView,
) -> Report {
    let genome = &evolution.best.genome;
    Report {
        schema: 1,
        tool: "crab-cage",
        demo: "jacketed-tank",
        seed,
        backend: backend.to_string(),
        population,
        generations,
        evaluations: evolution.evaluations,
        best: BestView {
            fitness: detailed.score,
            law: genome.law.to_string(),
            genes: genome.genes.clone(),
            nominal_tracking_mse: detailed.nominal.tracking_mse,
            trend_shift_tracking_mse: detailed.trend_shift.tracking_mse,
            nominal_prediction_mse: detailed.nominal.prediction_mse,
            trend_shift_prediction_mse: detailed.trend_shift.prediction_mse,
            effort: detailed.nominal.effort,
            overshoot: detailed.nominal.overshoot,
        },
        curve: evolution.history.clone(),
        pareto: evolution
            .archive
            .iter()
            .map(|member| ParetoView {
                fitness: member.fitness.score,
                law: member.genome.law.to_string(),
                nominal_tracking_mse: member.fitness.nominal.tracking_mse,
                trend_shift_tracking_mse: member.fitness.trend_shift.tracking_mse,
                prediction_mse: member.fitness.nominal.prediction_mse,
                effort: member.fitness.nominal.effort,
            })
            .collect(),
        trace_preview: detailed.nominal.trace.clone(),
        verification,
        limits,
    }
}

pub fn write_report(
    dir: &Path,
    report: &Report,
    source: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)> {
    fs::create_dir_all(dir)?;
    let controller = dir.join("best_controller.rs");
    let json_path = dir.join("report.json");
    let md_path = dir.join("report.md");
    fs::write(&controller, source)?;
    fs::write(&json_path, serde_json::to_string_pretty(report)?)?;
    fs::write(&md_path, markdown(report))?;
    Ok((controller, json_path, md_path))
}

pub fn markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# Crab Cage report\n\n");
    out.push_str("Jacketed-tank demo. Lower fitness is better. The nominal scenario steps the setpoint through three holds with a periodic coolant-flow disturbance. The trend-shift scenario ramps ambient, flow, inlet temperature, and the leak.\n\n");
    out.push_str(&format!(
        "Backend `{backend}`, seed {seed}, population {pop}, generations {gens}, evaluations {evals}.\n\n",
        backend = report.backend,
        seed = report.seed,
        pop = report.population,
        gens = report.generations,
        evals = report.evaluations,
    ));
    out.push_str(&format!(
        "## Best\n\n- fitness: {fit:.6}\n- law: `{law}`\n- nominal tracking MSE: {nom:.4}\n- trend-shift tracking MSE: {trend:.4}\n- nominal prediction MSE: {pred:.4}\n- trend-shift prediction MSE: {tpred:.4}\n- actuator effort: {effort:.6}\n- overshoot: {over:.4}\n\n",
        fit = report.best.fitness,
        law = report.best.law,
        nom = report.best.nominal_tracking_mse,
        trend = report.best.trend_shift_tracking_mse,
        pred = report.best.nominal_prediction_mse,
        tpred = report.best.trend_shift_prediction_mse,
        effort = report.best.effort,
        over = report.best.overshoot,
    ));
    out.push_str(
        "## Fitness curve\n\n| generation | best | law | proposal |\n|---:|---:|---|---|\n",
    );
    for row in &report.curve {
        out.push_str(&format!(
            "| {g} | {fit:.4} | `{law}` | {note} |\n",
            g = row.generation,
            fit = row.best_fitness,
            law = row.best_law,
            note = row.note.replace('|', "/"),
        ));
    }
    out.push_str("\n## Pareto archive\n\nTracking, trend-shift tracking, prediction, and effort. A member stays only when no other member is better on every objective.\n\n");
    out.push_str("| fitness | law | tracking | trend | prediction | effort |\n|---:|---|---:|---:|---:|---:|\n");
    for row in &report.pareto {
        out.push_str(&format!(
            "| {fit:.3} | `{law}` | {tr:.3} | {trend:.3} | {pred:.3} | {eff:.5} |\n",
            fit = row.fitness,
            law = row.law,
            tr = row.nominal_tracking_mse,
            trend = row.trend_shift_tracking_mse,
            pred = row.prediction_mse,
            eff = row.effort,
        ));
    }
    out.push_str("\n## Prediction against the plant\n\nA slice of the nominal closed loop. `predicted` is the controller's one-step temperature; `next` is the plant.\n\n");
    out.push_str("| step | temperature | setpoint | heater | predicted | next |\n|---:|---:|---:|---:|---:|---:|\n");
    for point in report.trace_preview.iter().step_by(2) {
        out.push_str(&format!(
            "| {step} | {t:.2} | {sp:.2} | {h:.3} | {p:.2} | {n:.2} |\n",
            step = point.step,
            t = point.temperature,
            sp = point.setpoint,
            h = point.heater,
            p = point.predicted,
            n = point.actual_next,
        ));
    }
    out.push_str("\n## Sandbox\n\n");
    out.push_str(&format!("- rustc: {}\n", report.verification.rustc));
    if let Some(err) = report.verification.max_abs_error {
        out.push_str(&format!("- replay max abs error: {err:.3e}\n"));
    }
    out.push_str(&format!("- lust: {}\n", report.verification.lust));
    out.push_str(&format!(
        "- lust check: {}\n",
        report.verification.lust_check
    ));
    out.push_str(&format!(
        "- limits: wall {} ms, lust --max-memory {} bytes, host backstop {} bytes\n",
        report.limits.run_timeout_ms,
        report.limits.lust_max_memory_bytes,
        report.limits.host_memory_bytes
    ));
    out.push_str(&format!("\n{}\n", report.limits.note));
    out
}

pub fn best_genome(evolution: &Evolution) -> &Genome {
    &evolution.best.genome
}
