//! One candidate through the funnel: prefilter, lust check, build, run.
//!
//! `opts.oracle` stops the funnel after a score. WP-14 adds the oracle.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::lust::{self, LustTool, RunClass};
use crate::nugget::{Nugget, Objective};
use crate::supervisor;

/// Where the fast gates run. `lust: None` compiles them with rustc.
pub struct Toolchain {
    pub lust: Option<LustTool>,
}

/// Knobs for one evaluation. `oracle` is the stop point WP-14 extends.
pub struct EvalOptions {
    pub work_dir: PathBuf,
    pub host_memory: u64,
    pub compile_timeout: Duration,
    pub oracle: bool,
}

/// Last gate the candidate reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Prefilter,
    Check,
    Build,
    Run,
    Oracle,
}

impl Gate {
    fn as_str(self) -> &'static str {
        match self {
            Gate::Prefilter => "prefilter",
            Gate::Check => "check",
            Gate::Build => "build",
            Gate::Run => "run",
            Gate::Oracle => "oracle",
        }
    }
}

/// What the funnel decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Rejected,
    Scored,
    Proven,
    Diverged,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Rejected => "rejected",
            Status::Scored => "scored",
            Status::Proven => "proven",
            Status::Diverged => "diverged",
        }
    }
}

/// One diagnostic, with `line` counted from the first line of the candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub line: u32,
    pub col: u32,
    pub fix: String,
}

/// Wall-clock milliseconds of each stage. A stage that did not run is 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ms {
    pub check: u64,
    pub build: u64,
    pub run: u64,
    pub oracle: u64,
}

/// The result of evaluating one candidate. Also one journal line (WP-13).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub hash: String,
    pub gate: Gate,
    pub status: Status,
    pub score: Option<f64>,
    pub passed: u32,
    pub total: u32,
    pub diagnostics: Vec<Diagnostic>,
    pub ms: Ms,
    pub note: String,
}

/// Evaluate `candidate` against `nugget`.
///
/// The hash is FNV-1a 64 of the candidate bytes, written as 16 hex digits.
/// A lust check failure carries the diagnostics from [`lust::check`]: line
/// numbers are moved so line 1 is the first line after `// CRAB:BEGIN`, and
/// a line outside the candidate is 0. That parser already keeps `code` and
/// `message` when a JSON field has the wrong type; this function does not
/// parse the diagnostic text again.
pub fn evaluate(
    nugget: &Nugget,
    candidate: &str,
    tools: &Toolchain,
    opts: &EvalOptions,
) -> crate::error::Result<Verdict> {
    let hash = candidate_hash(candidate);
    let mut verdict = Verdict {
        hash: hash.clone(),
        gate: Gate::Prefilter,
        status: Status::Rejected,
        score: None,
        passed: 0,
        total: u32::try_from(nugget.cases.len()).unwrap_or(u32::MAX),
        diagnostics: Vec::new(),
        ms: Ms::default(),
        note: String::new(),
    };

    if let Err(message) = crate::emit::sandbox_prefilter(candidate) {
        verdict.note = message;
        return Ok(verdict);
    }

    fs::create_dir_all(&opts.work_dir)?;
    let source = opts.work_dir.join(format!("{hash}.rs"));
    fs::write(&source, nugget.splice(candidate))?;

    if let Some(tool) = &tools.lust {
        verdict.gate = Gate::Check;
        let started = Instant::now();
        let checked = lust::check(tool, &source, opts.compile_timeout)?;
        verdict.ms.check = elapsed_ms(started);
        if !checked.ok {
            verdict.diagnostics = map_diagnostics(&checked.diagnostics, nugget, candidate);
            if verdict.diagnostics.is_empty() {
                verdict.note = "lust check failed".to_string();
            }
            return Ok(verdict);
        }
    }

    verdict.gate = Gate::Build;
    let binary = opts.work_dir.join(bin_name(&hash));
    let started = Instant::now();
    let built = if let Some(tool) = &tools.lust {
        lust::build(
            tool,
            &source,
            &binary,
            nugget.max_memory,
            Some(nugget.timeout),
            opts.compile_timeout,
        )
    } else {
        compile_rustc(&source, &binary, opts.compile_timeout)
    };
    verdict.ms.build = elapsed_ms(started);
    if let Err(problem) = built {
        verdict.note = problem.message;
        return Ok(verdict);
    }
    if !binary.is_file() {
        verdict.note = format!("compiler exited 0 but did not write {}", binary.display());
        return Ok(verdict);
    }

    verdict.gate = Gate::Run;
    let started = Instant::now();
    let outcome = run_cases(nugget, &binary, opts)?;
    verdict.ms.run = elapsed_ms(started);
    verdict.passed = outcome.passed;
    if let Some(note) = outcome.stop {
        verdict.note = note;
        return Ok(verdict);
    }
    verdict.status = Status::Scored;
    verdict.score = Some(outcome.score);

    if opts.oracle {
        // WP-14 runs the oracle here and may set `proven` or `diverged`.
    }
    Ok(verdict)
}

/// Three lines: outcome, score, then the note, the first diagnostic, or the times.
pub fn format_verdict(verdict: &Verdict) -> String {
    let score = match verdict.score {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    };
    let detail = if !verdict.note.is_empty() {
        one_line(&verdict.note)
    } else if let Some(diag) = verdict.diagnostics.first() {
        one_line(&format!(
            "{}:{}:{}: {}",
            diag.code, diag.line, diag.col, diag.message
        ))
    } else {
        format!(
            "ms check={} build={} run={} oracle={}",
            verdict.ms.check, verdict.ms.build, verdict.ms.run, verdict.ms.oracle
        )
    };
    format!(
        "{} {} {}\npassed {}/{} score {}\n{}\n",
        verdict.status.as_str(),
        verdict.gate.as_str(),
        verdict.hash,
        verdict.passed,
        verdict.total,
        score,
        detail
    )
}

/// FNV-1a 64. Offset basis, then xor and multiply by the prime, per byte.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn candidate_hash(candidate: &str) -> String {
    format!("{:016x}", fnv1a64(candidate.as_bytes()))
}

fn map_diagnostics(
    diagnostics: &[lust::Diagnostic],
    nugget: &Nugget,
    candidate: &str,
) -> Vec<Diagnostic> {
    let last = candidate_line_count(candidate);
    diagnostics
        .iter()
        .map(|diag| Diagnostic {
            code: diag.code.clone(),
            message: diag.message.clone(),
            line: relative_line(diag.line, nugget.region_first_line, last),
            col: diag.col,
            fix: diag.fix.clone(),
        })
        .collect()
}

/// Lines in the candidate. A trailing newline does not add a line.
fn candidate_line_count(candidate: &str) -> usize {
    if candidate.is_empty() {
        0
    } else {
        candidate.lines().count()
    }
}

/// `file_line - region_first_line + 1`, or 0 when that is outside the candidate.
fn relative_line(file_line: u32, region_first_line: usize, last_line: usize) -> u32 {
    let file_line = u64::from(file_line);
    let region_first = u64::try_from(region_first_line).unwrap_or(u64::MAX);
    let last = u64::try_from(last_line).unwrap_or(u64::MAX);
    let Some(relative) = file_line
        .checked_add(1)
        .and_then(|sum| sum.checked_sub(region_first))
    else {
        return 0;
    };
    if relative < 1 || relative > last {
        0
    } else {
        u32::try_from(relative).unwrap_or(0)
    }
}

/// Same rule as `bin_name` in `evolve`: Windows executables end in `.exe`.
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

/// Edition 2024, then 2021 only when rustc rejects the edition itself.
/// The tank replay does this inside `evolve`, which is not part of this package.
fn compile_rustc(source: &Path, output: &Path, timeout: Duration) -> crate::error::Result<()> {
    match lust::compile_rustc(source, output, "2024", timeout) {
        Ok(()) => Ok(()),
        Err(problem) => {
            let text = problem.message.to_lowercase();
            let edition_problem = text.contains("edition")
                && (text.contains("unknown")
                    || text.contains("invalid")
                    || text.contains("must be one of"));
            if !edition_problem {
                return Err(problem);
            }
            lust::compile_rustc(source, output, "2021", timeout)
        }
    }
}

struct RunOutcome {
    passed: u32,
    score: f64,
    /// Set when the candidate is rejected at `run`. The note is the class
    /// name, or why a `minimize` score could not be read.
    stop: Option<String>,
}

fn run_cases(nugget: &Nugget, bin: &Path, opts: &EvalOptions) -> crate::error::Result<RunOutcome> {
    let limits = lust::candidate_limits(nugget.timeout, opts.host_memory);
    let mut passed = 0u32;
    let mut mismatches = 0u32;
    let mut sum = 0.0f64;
    let mut unclean: Option<RunClass> = None;
    let mut bad_score: Option<String> = None;

    for case in &nugget.cases {
        let result = supervisor::run_capture(Command::new(bin), &limits, case.input.as_bytes())?;
        let class = lust::classify(&result);
        // A timeout or other unclean stop rejects the candidate. Later cases
        // would not change that, so they are not run. A Clean run with the
        // wrong output is still scored and the remaining cases still run.
        if class != RunClass::Clean {
            unclean = Some(class);
            break;
        }
        match nugget.objective {
            Objective::PassAll => {
                let expected = case.expected.as_deref().unwrap_or("");
                if result.stdout.trim_end() == expected.trim_end() {
                    passed = passed.saturating_add(1);
                } else {
                    mismatches = mismatches.saturating_add(1);
                }
            }
            Objective::Minimize => match parse_score(&result.stdout) {
                Ok(value) => {
                    sum += value;
                    passed = passed.saturating_add(1);
                }
                Err(note) => {
                    if bad_score.is_none() {
                        bad_score = Some(note);
                    }
                }
            },
        }
    }

    if let Some(class) = unclean {
        return Ok(RunOutcome {
            passed,
            score: 0.0,
            stop: Some(class_name(class).to_string()),
        });
    }
    if let Some(note) = bad_score {
        return Ok(RunOutcome {
            passed,
            score: 0.0,
            stop: Some(note),
        });
    }
    let score = match nugget.objective {
        Objective::PassAll => f64::from(mismatches),
        Objective::Minimize => sum,
    };
    if nugget.objective == Objective::Minimize && !score.is_finite() {
        return Ok(RunOutcome {
            passed,
            score: 0.0,
            stop: Some("non-finite score".to_string()),
        });
    }
    Ok(RunOutcome {
        passed,
        score,
        stop: None,
    })
}

fn class_name(class: RunClass) -> &'static str {
    match class {
        RunClass::Clean => "Clean",
        RunClass::TimedOut => "TimedOut",
        RunClass::TimeLust => "TimeLust",
        RunClass::MemoryLust => "MemoryLust",
        RunClass::MemoryHost => "MemoryHost",
        RunClass::SandboxDenied => "SandboxDenied",
        RunClass::Panic => "Panic",
        RunClass::Failed => "Failed",
    }
}

/// Last non-empty stdout line, `score: <f64>`. Missing and non-finite are errors.
fn parse_score(stdout: &str) -> Result<f64, String> {
    let Some(line) = stdout.lines().rev().find(|line| !line.trim().is_empty()) else {
        return Err("missing score".to_string());
    };
    let Some(rest) = line.trim().strip_prefix("score:") else {
        return Err("missing score".to_string());
    };
    let Ok(value) = rest.trim().parse::<f64>() else {
        return Err("missing score".to_string());
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err("non-finite score".to_string())
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn one_line(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        "-".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_the_known_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
        assert_eq!(candidate_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(candidate_hash("a").len(), 16);
    }

    #[test]
    fn lines_are_counted_from_the_first_candidate_line() {
        // File line 6 is region_first_line, so it is candidate line 1.
        let candidate = "    let value: i64 = \"no\";\n    value\n";
        assert_eq!(candidate_line_count(candidate), 2);
        assert_eq!(relative_line(6, 6, 2), 1);
        assert_eq!(relative_line(7, 6, 2), 2);
        assert_eq!(relative_line(5, 6, 2), 0);
        assert_eq!(relative_line(8, 6, 2), 0);
        assert_eq!(relative_line(0, 6, 2), 0);

        let nugget = Nugget {
            name: "n".to_string(),
            objective: Objective::PassAll,
            timeout: Duration::from_millis(1),
            max_memory: 1,
            prefix: String::new(),
            seed: String::new(),
            suffix: String::new(),
            region_first_line: 6,
            cases: Vec::new(),
        };
        let raw = vec![
            lust::Diagnostic {
                code: "L0110".to_string(),
                message: "mismatched types".to_string(),
                severity: "error".to_string(),
                line: 6,
                col: 18,
                fix: "Change the value.".to_string(),
                auto_edits: 1,
                raw: String::new(),
            },
            lust::Diagnostic {
                code: "L0102".to_string(),
                message: "outside".to_string(),
                severity: "error".to_string(),
                line: 4,
                col: 1,
                fix: String::new(),
                auto_edits: 0,
                raw: String::new(),
            },
        ];
        let mapped = map_diagnostics(&raw, &nugget, candidate);
        assert_eq!(mapped[0].line, 1);
        assert_eq!(mapped[0].col, 18);
        assert_eq!(mapped[0].code, "L0110");
        assert_eq!(mapped[0].fix, "Change the value.");
        assert_eq!(mapped[1].line, 0);
        assert_eq!(mapped[1].code, "L0102");
    }
}
