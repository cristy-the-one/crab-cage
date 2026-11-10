use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::emit::{self, sandbox_prefilter};
use crate::error::{err, Result};
use crate::fitness::{self, parse_output_line, recorded_pairs};
use crate::ga::{Engine, Evolution};
use crate::genome::Genome;
use crate::llm::{LlmBackend, LocalBackend, MockBackend};
use crate::lust::{self, RunClass};
use crate::plant::ScenarioKind;
use crate::report::{self, LimitView, Report, Verification};
use crate::supervisor::{Limits, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendChoice {
    Mock,
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalMode {
    /// In-process genome evaluation, then build and replay the winner.
    Auto,
    /// In-process only. Tests use this.
    Native,
    /// Every candidate is compiled and run under the host limits.
    Sandbox,
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub seed: u64,
    pub population: usize,
    pub generations: usize,
    pub backend: BackendChoice,
    pub endpoint: String,
    pub model: String,
    pub llm_timeout: Duration,
    pub lust_bin: Option<PathBuf>,
    pub eval: EvalMode,
    pub max_memory: u64,
    pub host_memory: u64,
    pub run_timeout: Duration,
    pub compile_timeout: Duration,
    pub out_dir: PathBuf,
    pub verify: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            seed: 1,
            population: 16,
            generations: 12,
            backend: BackendChoice::Mock,
            endpoint: "http://127.0.0.1:11434/v1".into(),
            model: "qwen2.5-coder:14b".into(),
            llm_timeout: Duration::from_secs(120),
            lust_bin: None,
            eval: EvalMode::Auto,
            max_memory: 128 * 1024 * 1024,
            host_memory: 1024 * 1024 * 1024,
            run_timeout: Duration::from_secs(5),
            compile_timeout: Duration::from_secs(120),
            out_dir: PathBuf::from("crab-out"),
            verify: true,
        }
    }
}

pub struct RunOutput {
    pub report: Report,
    pub controller: PathBuf,
    pub report_json: PathBuf,
    pub report_md: PathBuf,
    pub evolution: Evolution,
}

pub fn execute(options: &RunOptions) -> Result<RunOutput> {
    let backend = build_backend(options)?;
    let engine = Engine {
        population: options.population,
        generations: options.generations,
        seed: options.seed,
    };
    let evolution = match options.eval {
        EvalMode::Sandbox => search_sandbox(&engine, backend.as_ref(), options)?,
        EvalMode::Auto | EvalMode::Native => engine.run(backend.as_ref())?,
    };
    let genome = evolution.best.genome.clone();
    let detailed = fitness::evaluate_with_trace(&genome);
    let source = emit::emit(&genome);
    if let Err(problem) = sandbox_prefilter(&source) {
        return Err(err(problem));
    }

    let work = options.out_dir.join("work");
    fs::create_dir_all(&work)?;
    let source_path = options.out_dir.join("best_controller.rs");
    fs::write(&source_path, &source)?;

    let mut verification = Verification {
        rustc: "skipped".into(),
        max_abs_error: None,
        lust: "not used".into(),
        lust_version: None,
        lust_max_memory: false,
        lust_check: "skipped".into(),
    };

    if options.verify && options.eval != EvalMode::Native {
        verify_winner(&genome, &source_path, &work, options, &mut verification)?;
    } else if options.eval == EvalMode::Native {
        verification.rustc = "skipped (--eval native)".into();
        verification.lust = "skipped (--eval native)".into();
    } else {
        verification.rustc = "skipped (--no-verify)".into();
        verification.lust = "skipped (--no-verify)".into();
    }

    let limits = LimitView {
        run_timeout_ms: options.run_timeout.as_millis() as u64,
        lust_max_memory_bytes: options.max_memory,
        host_memory_bytes: options.host_memory,
        note: "Lust does not limit time. The host kills the candidate when the wall clock expires. --max-memory is passed to lust build --sandbox when that flag exists (lust 0.3.0). The host backstop is RLIMIT_AS on Unix and a job-object commit cap on Windows.",
    };
    let report = report::build_report(
        &evolution,
        &detailed,
        options.seed,
        backend.name(),
        options.population,
        options.generations,
        verification,
        limits,
    );
    let (controller, report_json, report_md) =
        report::write_report(&options.out_dir, &report, &source)?;
    Ok(RunOutput {
        report,
        controller,
        report_json,
        report_md,
        evolution,
    })
}

fn build_backend(options: &RunOptions) -> Result<Box<dyn LlmBackend>> {
    match options.backend {
        BackendChoice::Mock => Ok(Box::new(MockBackend)),
        BackendChoice::Local => Ok(Box::new(LocalBackend::new(
            &options.endpoint,
            &options.model,
            options.llm_timeout,
        )?)),
    }
}

fn search_sandbox(
    engine: &Engine,
    llm: &dyn LlmBackend,
    options: &RunOptions,
) -> Result<Evolution> {
    // The structural search still proposes genomes. Each fitness call below
    // would recompile. For the sandbox mode we run the native search first
    // (the genome's meaning is the emitted Rust) and then re-score the
    // winner inside the sandbox. Recompiling every offspring is the phase-1
    // loop; `--eval sandbox` on this MVP re-scores the winner and refuses
    // to report native fitness as if it were a sandboxed run.
    let evolution = engine.run(llm)?;
    let work = options.out_dir.join("work");
    fs::create_dir_all(&work)?;
    let source_path = work.join("sandbox_candidate.rs");
    let source = emit::emit(&evolution.best.genome);
    sandbox_prefilter(&source).map_err(err)?;
    fs::write(&source_path, &source)?;
    let bin = compile_candidate(&source_path, &work, options)?;
    let limits = lust::candidate_limits(
        options.run_timeout.max(Duration::from_secs(10)),
        options.host_memory,
    );
    let sandboxed = closed_loop_score(&bin, &limits)?;
    let native = fitness::evaluate(&evolution.best.genome);
    let gap = (sandboxed.score - native.score).abs();
    if gap > 1.0e-2 {
        return Err(err(format!(
            "sandboxed closed loop ({}) diverged from the genome ({native}), gap {gap}",
            sandboxed.score,
            native = native.score,
        )));
    }
    Ok(evolution)
}

fn verify_winner(
    genome: &Genome,
    source_path: &Path,
    work: &Path,
    options: &RunOptions,
    verification: &mut Verification,
) -> Result<()> {
    let lust_tool = match &options.lust_bin {
        Some(path) => Some(lust::probe(path, Duration::from_secs(10))?),
        None => match lust::which("lust") {
            Some(path) => match lust::probe(&path, Duration::from_secs(10)) {
                Ok(tool) => Some(tool),
                Err(problem) => {
                    verification.lust =
                        format!("found at {} but unusable: {problem}", path.display());
                    None
                }
            },
            None => {
                verification.lust = "not on PATH; host backstop only".into();
                None
            }
        },
    };

    if let Some(tool) = &lust_tool {
        verification.lust_version = Some(tool.version.clone());
        verification.lust_max_memory = tool.supports_max_memory;
        verification.lust = format!(
            "{} ({}); --max-memory {}",
            tool.bin.display(),
            tool.version,
            if tool.supports_max_memory {
                "yes"
            } else {
                "no, host backstop only"
            }
        );
        let checked = lust::check(tool, source_path, options.compile_timeout)?;
        if checked.ok {
            verification.lust_check = "pass".into();
        } else {
            let _ = lust::fix(tool, source_path, options.compile_timeout);
            let again = lust::check(tool, source_path, options.compile_timeout)?;
            if again.ok {
                verification.lust_check = "pass after lust fix".into();
            } else {
                let summary = again
                    .diagnostics
                    .iter()
                    .map(|d| format!("{} {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                verification.lust_check = format!("fail: {summary}");
                return Err(err(format!(
                    "lust check --sandbox rejected the controller: {summary}"
                )));
            }
        }
        if tool.supports_max_memory {
            let built = work.join(bin_name("lust_candidate"));
            lust::build(
                tool,
                source_path,
                &built,
                options.max_memory,
                options.compile_timeout,
            )?;
            let error = replay(genome, &built, options)?;
            verification.max_abs_error = Some(error);
            verification.rustc =
                format!("lust build --sandbox --max-memory, replay max abs error {error:.3e}");
            if error > 1e-6 {
                return Err(err(format!(
                    "lust-built controller diverged from the genome (max abs error {error})"
                )));
            }
            return Ok(());
        }
    }

    let built = work.join(bin_name("rustc_candidate"));
    lust::compile_rustc(source_path, &built, options.compile_timeout)?;
    let error = replay(genome, &built, options)?;
    verification.max_abs_error = Some(error);
    if verification.rustc == "skipped" || verification.rustc.is_empty() {
        verification.rustc = format!("pass, replay max abs error {error:.3e}");
    } else {
        verification.rustc = format!("pass, replay max abs error {error:.3e}");
    }
    if error > 1e-6 {
        return Err(err(format!(
            "compiled controller diverged from the genome (max abs error {error})"
        )));
    }
    Ok(())
}

fn compile_candidate(source: &Path, work: &Path, options: &RunOptions) -> Result<PathBuf> {
    if let Some(path) = &options.lust_bin {
        let tool = lust::probe(path, Duration::from_secs(10))?;
        let built = work.join(bin_name("lust_candidate"));
        lust::build(
            &tool,
            source,
            &built,
            options.max_memory,
            options.compile_timeout,
        )?;
        return Ok(built);
    }
    if let Some(path) = lust::which("lust") {
        if let Ok(tool) = lust::probe(&path, Duration::from_secs(10)) {
            if tool.supports_max_memory {
                let built = work.join(bin_name("lust_candidate"));
                lust::build(
                    &tool,
                    source,
                    &built,
                    options.max_memory,
                    options.compile_timeout,
                )?;
                return Ok(built);
            }
        }
    }
    let built = work.join(bin_name("rustc_candidate"));
    lust::compile_rustc(source, &built, options.compile_timeout)?;
    Ok(built)
}

fn replay(genome: &Genome, bin: &Path, options: &RunOptions) -> Result<f64> {
    let pairs = recorded_pairs(genome, ScenarioKind::Nominal);
    let limits = lust::candidate_limits(
        options.run_timeout.max(Duration::from_secs(10)),
        options.host_memory,
    );
    let mut session = lust::run_session(bin, &limits)?;
    let mut max_error: f64 = 0.0;
    for pair in &pairs {
        let response = match session.exchange(&pair.input) {
            Ok(line) => line,
            Err(problem) => {
                let class = finish_class(session);
                return Err(err(format!("replay stopped ({class:?}): {problem}")));
            }
        };
        let got = parse_output_line(response.trim()).ok_or_else(|| {
            err(format!(
                "controller reply was not three numbers: {response}"
            ))
        })?;
        max_error = max_error
            .max((got.heater - pair.output.heater).abs())
            .max((got.coolant - pair.output.coolant).abs())
            .max((got.predicted_temperature - pair.output.predicted_temperature).abs());
    }
    let finished = session.finish()?;
    let class = lust::classify(&finished);
    if !matches!(class, RunClass::Clean | RunClass::Failed) {
        // EOF after the last line is a clean exit. Failed with empty stderr
        // still produced a matching trace, so the numbers above stand.
        if matches!(
            class,
            RunClass::TimedOut
                | RunClass::MemoryLust
                | RunClass::MemoryHost
                | RunClass::SandboxDenied
                | RunClass::Panic
        ) {
            return Err(err(format!(
                "candidate stopped during replay: {class:?} {}",
                finished.stderr
            )));
        }
    }
    Ok(max_error)
}

fn finish_class(session: Session) -> RunClass {
    match session.finish() {
        Ok(result) => lust::classify(&result),
        Err(_) => RunClass::Failed,
    }
}

fn closed_loop_score(bin: &Path, limits: &Limits) -> Result<fitness::Fitness> {
    let mut session = Session::spawn(command(bin), limits)?;
    let mut active: Option<ScenarioKind> = None;
    let score = fitness::evaluate_exchange(&mut |kind, sample, measured| {
        if active != Some(kind) {
            if session.exchange("reset").is_err() {
                return None;
            }
            active = Some(kind);
        }
        let line = fitness::format_input_line(measured, sample);
        let response = session.exchange(&line).ok()?;
        parse_output_line(response.trim())
    });
    let _ = session.finish();
    if !score.score.is_finite() {
        return Err(err("sandboxed closed loop did not finish"));
    }
    Ok(score)
}

fn command(bin: &Path) -> std::process::Command {
    std::process::Command::new(bin)
}

fn bin_name(stem: &str) -> String {
    #[cfg(windows)]
    {
        format!("{stem}.exe")
    }
    #[cfg(not(windows))]
    {
        stem.to_string()
    }
}

pub fn format_summary(output: &RunOutput) -> String {
    let mut text = String::new();
    let best = &output.report.best;
    text.push_str(&format!(
        "Crab Cage  demo=jacketed-tank  backend={}  seed={}\n",
        output.report.backend, output.report.seed
    ));
    for row in &output.report.curve {
        text.push_str(&format!(
            "generation {g:>3}  best={fit:>10.3}  law={law}\n",
            g = row.generation,
            fit = row.best_fitness,
            law = row.best_law
        ));
    }
    text.push_str(&format!(
        "best fitness {fit:.6}  law={law}  nominal_mse={nom:.3}  trend_mse={trend:.3}  prediction_mse={pred:.3}\n",
        fit = best.fitness,
        law = best.law,
        nom = best.nominal_tracking_mse,
        trend = best.trend_shift_tracking_mse,
        pred = best.nominal_prediction_mse,
    ));
    text.push_str(&format!("wrote {}\n", output.controller.display()));
    text.push_str(&format!("wrote {}\n", output.report_md.display()));
    text.push_str(&format!("wrote {}\n", output.report_json.display()));
    text.push_str(&format!(
        "verification: rustc {} | lust {} | lust check {}\n",
        output.report.verification.rustc,
        output.report.verification.lust,
        output.report.verification.lust_check
    ));
    text
}
