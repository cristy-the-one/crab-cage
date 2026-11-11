//! One candidate through the funnel: prefilter, lust check, build, run, oracle.
//!
//! When `opts.oracle` is set and the candidate scored, rustc is the oracle.
//! With no lust binary the fast build already is rustc, so that step is
//! `proven` without a second compile.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::lust::{self, LustTool, RunClass};
use crate::nugget::{Nugget, Objective};
use crate::supervisor::{self, StopKind};

/// The file lust compiles when its daemon is on. One path for every
/// candidate keeps the daemon's session warm.
pub const SLOT_FILE: &str = "slot.rs";

/// Where the fast gates run. `lust: None` compiles them with rustc.
pub struct Toolchain {
    pub lust: Option<LustTool>,
}

/// Knobs for one evaluation. `oracle` compares a scored fast build with rustc.
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
    let spliced = nugget.splice(candidate);
    fs::write(&source, &spliced)?;
    // lust's daemon keeps one warm session per source path. With the daemon
    // on, lust always compiles the same file, `slot.rs`, rewritten for each
    // candidate; `<hash>.rs` stays as the record and is what rustc compiles.
    let fast_source = match &tools.lust {
        Some(tool) if tool.daemon => {
            let slot = opts.work_dir.join(SLOT_FILE);
            fs::write(&slot, &spliced)?;
            slot
        }
        _ => source.clone(),
    };

    if let Some(tool) = &tools.lust {
        verdict.gate = Gate::Check;
        let started = Instant::now();
        let checked = lust::check(tool, &fast_source, opts.compile_timeout)?;
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
            &fast_source,
            &binary,
            nugget.max_memory,
            Some(nugget.timeout),
            opts.compile_timeout,
        )
    } else {
        lust::compile_rustc_any_edition(&source, &binary, opts.compile_timeout)
            .map(|_| ())
            .map_err(crate::error::Error::from)
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

    // Rejections returned above, so the status here is `scored`.
    if opts.oracle {
        let started = Instant::now();
        run_oracle(
            &mut verdict,
            nugget,
            &source,
            &binary,
            &outcome.runs,
            tools,
            opts,
        )?;
        verdict.ms.oracle = elapsed_ms(started);
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

/// The `hash` of a verdict: FNV-1a 64 of the candidate text, as 16 hex
/// digits. The journal is keyed by it, so a caller that asks the journal
/// whether a candidate was seen must use this function.
pub fn candidate_hash(candidate: &str) -> String {
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

struct CaseCapture {
    stdout: String,
    kind: StopKind,
}

struct RunOutcome {
    passed: u32,
    score: f64,
    /// Set when the candidate is rejected at `run`. The note is the class
    /// name, or why a `minimize` score could not be read.
    stop: Option<String>,
    /// One entry per case that actually ran, in case order. A scored
    /// candidate has one entry per nugget case; the oracle compares these
    /// bytes and exit kinds with a fresh rustc run.
    runs: Vec<CaseCapture>,
}

/// Lust's own `--time-limit` stays `nugget.timeout`. The host wall clock is
/// 500 ms later, so an L0533 stop (stdout flushed, exit 101) is the normal
/// case and the host kill is only the backstop.
fn host_limits(nugget: &Nugget, opts: &EvalOptions) -> supervisor::Limits {
    let wall = nugget.timeout.saturating_add(Duration::from_millis(500));
    lust::candidate_limits(wall, opts.host_memory)
}

fn run_oracle(
    verdict: &mut Verdict,
    nugget: &Nugget,
    source: &Path,
    fast_bin: &Path,
    runs: &[CaseCapture],
    tools: &Toolchain,
    opts: &EvalOptions,
) -> crate::error::Result<()> {
    verdict.gate = Gate::Oracle;
    let Some(tool) = &tools.lust else {
        verdict.status = Status::Proven;
        return Ok(());
    };
    if runs.len() != nugget.cases.len() {
        return Err(crate::error::err("oracle expected one fast run per case"));
    }
    let oracle_bin = opts
        .work_dir
        .join(bin_name(&format!("{}.oracle", verdict.hash)));
    let edition = match lust::compile_rustc_any_edition(source, &oracle_bin, opts.compile_timeout) {
        Ok(edition) => edition,
        Err(lust::RustcError::Rejected { message }) => {
            save_rustc_rejection(
                verdict,
                nugget,
                tool,
                source,
                fast_bin,
                &oracle_bin,
                opts,
                &message,
            )?;
            return Ok(());
        }
        Err(problem) => return Err(problem.into()),
    };
    if !oracle_bin.is_file() {
        return Err(crate::error::err(format!(
            "rustc exited 0 but did not write {}",
            oracle_bin.display()
        )));
    }
    let limits = host_limits(nugget, opts);
    for (case, fast) in nugget.cases.iter().zip(runs.iter()) {
        let result =
            supervisor::run_capture(Command::new(&oracle_bin), &limits, case.input.as_bytes())?;
        if result.stdout == fast.stdout && result.kind == fast.kind {
            continue;
        }
        let again =
            supervisor::run_capture(Command::new(&oracle_bin), &limits, case.input.as_bytes())?;
        if again.stdout != result.stdout || again.kind != result.kind {
            verdict.status = Status::Rejected;
            verdict.note = "nondeterministic output".to_string();
            return Ok(());
        }
        write_case_divergence(
            opts,
            &verdict.hash,
            source,
            &case.input,
            &fast.stdout,
            &result.stdout,
            &divergence_readme(
                tool,
                nugget,
                source,
                fast_bin,
                &oracle_bin,
                &edition,
                &case.id,
                fast.kind,
                result.kind,
            ),
        )?;
        verdict.status = Status::Diverged;
        return Ok(());
    }
    verdict.status = Status::Proven;
    Ok(())
}

fn save_rustc_rejection(
    verdict: &mut Verdict,
    nugget: &Nugget,
    tool: &LustTool,
    source: &Path,
    fast_bin: &Path,
    oracle_bin: &Path,
    opts: &EvalOptions,
    message: &str,
) -> crate::error::Result<()> {
    let dir = divergence_dir(&opts.work_dir, &verdict.hash);
    fs::create_dir_all(&dir)?;
    let _ = fs::copy(source, dir.join("source.rs"))?;
    fs::write(dir.join("rustc.stderr"), message)?;
    let readme = format!(
        "\
lust version: {version}
fast: {fast}
oracle: {oracle}
",
        version = tool.version,
        fast = fast_command(tool, nugget, source, fast_bin),
        oracle = rustc_command("2024", oracle_bin, source),
    );
    fs::write(dir.join("README.txt"), readme)?;
    verdict.status = Status::Diverged;
    verdict.note = "lust accepted, rustc rejected".to_string();
    Ok(())
}

fn write_case_divergence(
    opts: &EvalOptions,
    hash: &str,
    source: &Path,
    case_input: &str,
    fast_stdout: &str,
    oracle_stdout: &str,
    readme: &str,
) -> crate::error::Result<()> {
    let dir = divergence_dir(&opts.work_dir, hash);
    fs::create_dir_all(&dir)?;
    let _ = fs::copy(source, dir.join("source.rs"))?;
    fs::write(dir.join("case.in"), case_input)?;
    fs::write(dir.join("fast.stdout"), fast_stdout)?;
    fs::write(dir.join("oracle.stdout"), oracle_stdout)?;
    fs::write(dir.join("README.txt"), readme)?;
    Ok(())
}

fn divergence_dir(work_dir: &Path, hash: &str) -> PathBuf {
    work_dir.join("divergences").join(hash)
}

fn fast_command(tool: &LustTool, nugget: &Nugget, source: &Path, fast_bin: &Path) -> String {
    let mut cmd = format!("{} build --sandbox", tool.bin.display());
    if tool.supports_max_memory {
        cmd.push_str(" --max-memory ");
        cmd.push_str(&nugget.max_memory.to_string());
    }
    if tool.supports_time_limit {
        cmd.push_str(" --time-limit ");
        cmd.push_str(&lust::format_time_limit(nugget.timeout));
    }
    cmd.push_str(" -o ");
    cmd.push_str(&fast_bin.display().to_string());
    cmd.push(' ');
    cmd.push_str(&source.display().to_string());
    cmd
}

fn divergence_readme(
    tool: &LustTool,
    nugget: &Nugget,
    source: &Path,
    fast_bin: &Path,
    oracle_bin: &Path,
    edition: &str,
    case_id: &str,
    fast_kind: StopKind,
    oracle_kind: StopKind,
) -> String {
    format!(
        "\
lust version: {version}
fast: {fast}
oracle: {oracle}
case: {case_id}
fast exit: {fast_kind:?}
oracle exit: {oracle_kind:?}
",
        version = tool.version,
        fast = fast_command(tool, nugget, source, fast_bin),
        oracle = rustc_command(edition, oracle_bin, source),
    )
}

/// Same profile `lust::compile_rustc` passes: optimized, with the debug
/// assertions and overflow checks lust leaves on.
fn rustc_command(edition: &str, oracle_bin: &Path, source: &Path) -> String {
    format!(
        "rustc --edition {edition} -C opt-level=2 -C debuginfo=0 -C debug-assertions=on -C overflow-checks=on -o {} {}",
        oracle_bin.display(),
        source.display()
    )
}

fn run_cases(nugget: &Nugget, bin: &Path, opts: &EvalOptions) -> crate::error::Result<RunOutcome> {
    let limits = host_limits(nugget, opts);
    let mut passed = 0u32;
    let mut mismatches = 0u32;
    let mut sum = 0.0f64;
    let mut unclean: Option<RunClass> = None;
    let mut bad_score: Option<String> = None;
    let mut runs = Vec::with_capacity(nugget.cases.len());

    for case in &nugget.cases {
        let result = supervisor::run_capture(Command::new(bin), &limits, case.input.as_bytes())?;
        let class = lust::classify(&result);
        // A timeout or other unclean stop rejects the candidate. Later cases
        // would not change that, so they are not run. A Clean run with the
        // wrong output is still scored and the remaining cases still run.
        if class == RunClass::Clean {
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
        } else {
            unclean = Some(class);
        }
        runs.push(CaseCapture {
            stdout: result.stdout,
            kind: result.kind,
        });
        if unclean.is_some() {
            break;
        }
    }

    if let Some(class) = unclean {
        return Ok(RunOutcome {
            passed,
            score: 0.0,
            stop: Some(class_name(class).to_string()),
            runs,
        });
    }
    if let Some(note) = bad_score {
        return Ok(RunOutcome {
            passed,
            score: 0.0,
            stop: Some(note),
            runs,
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
            runs,
        });
    }
    Ok(RunOutcome {
        passed,
        score,
        stop: None,
        runs,
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
