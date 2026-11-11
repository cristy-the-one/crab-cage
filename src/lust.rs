//! Talks to an external `lust` binary. Crab Cage does not vendor lust.
//!
//! A candidate is checked with `lust check --json --sandbox`, optionally
//! repaired with `lust fix`, then built with
//! `lust build --sandbox --max-memory <bytes> --time-limit <duration> -o <bin>`.
//! `--time-limit` is passed when the binary advertises it. The host still
//! kills on the wall clock: a straight-line body is not probed, and a lust
//! that predates the flag has no guest deadline.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{err, Result};
use crate::supervisor::{self, ExecResult, Limits, Session, StopKind};

#[derive(Clone, Debug)]
pub struct LustTool {
    pub bin: PathBuf,
    pub version: String,
    pub supports_max_memory: bool,
    pub supports_time_limit: bool,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: String,
    pub line: u32,
    pub col: u32,
    pub fix: String,
    pub auto_edits: usize,
    pub raw: String,
}

#[derive(Clone, Debug)]
pub struct CheckResult {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunClass {
    Clean,
    TimedOut,
    /// The guest printed `error[L0533]` and exited. Lust's own deadline.
    TimeLust,
    MemoryLust,
    MemoryHost,
    SandboxDenied,
    Panic,
    Failed,
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if name.contains('/') || name.contains('\\') || path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
            let cmd = dir.join(format!("{name}.cmd"));
            if cmd.is_file() {
                return Some(cmd);
            }
        }
    }
    None
}

pub fn probe(bin: &Path, timeout: Duration) -> Result<LustTool> {
    let version = tool_output(bin, &["--version"], timeout, &[("LUST_NO_DAEMON", "1")])?;
    let version = first_line(&version.stdout, &version.stderr);
    if version.is_empty() {
        return Err(err(format!(
            "{} --version produced no output",
            bin.display()
        )));
    }
    // Real lust prints its usage text when it is run with no arguments.
    // Only the `usage:` line and its indented continuation lines count:
    // an error message that merely contains a flag name proves nothing.
    let usage = tool_output(bin, &[], timeout, &[("LUST_NO_DAEMON", "1")]).unwrap_or(ExecResult {
        kind: StopKind::Exited(2),
        stdout: String::new(),
        stderr: String::new(),
        elapsed: Duration::ZERO,
    });
    let usage_text = format!(
        "{}
{}",
        usage.stdout, usage.stderr
    );
    let (supports_max_memory, supports_time_limit) = flags_from_usage(&usage_text);
    Ok(LustTool {
        bin: bin.to_path_buf(),
        version,
        supports_max_memory,
        supports_time_limit,
    })
}

/// `(--max-memory named, --time-limit named)` in lust's usage text. The usage
/// text is the first `usage:` line plus every line after it that is itself a
/// `usage:` line or starts with whitespace. With no `usage:` line, both are
/// false.
fn flags_from_usage(text: &str) -> (bool, bool) {
    let mut usage_text = String::new();
    let mut seen_usage = false;
    for line in text.lines() {
        let starts_usage = line.starts_with("usage:");
        if starts_usage {
            seen_usage = true;
        }
        if seen_usage && (starts_usage || line.starts_with(char::is_whitespace)) {
            usage_text.push_str(line);
            usage_text.push('\n');
        }
    }
    (
        usage_text.contains("--max-memory"),
        usage_text.contains("--time-limit"),
    )
}

/// `2s` when the duration is a whole number of seconds, otherwise `500ms`.
pub fn format_time_limit(limit: Duration) -> String {
    let ms = limit.as_millis();
    if ms >= 1000 && ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else if ms == 0 {
        "1ms".to_string()
    } else {
        format!("{ms}ms")
    }
}

pub fn check(tool: &LustTool, source: &Path, timeout: Duration) -> Result<CheckResult> {
    let result = tool_output(
        &tool.bin,
        &[
            "check",
            "--json",
            "--sandbox",
            &source.display().to_string(),
        ],
        timeout,
        &[("LUST_NO_DAEMON", "1")],
    )?;
    let text = format!("{}{}", result.stdout, result.stderr);
    let diagnostics = parse_diagnostics(&text);
    let ok = result.success() && diagnostics.iter().all(|d| d.code.is_empty());
    Ok(CheckResult { ok, diagnostics })
}

pub fn fix(tool: &LustTool, source: &Path, timeout: Duration) -> Result<ExecResult> {
    let with_sandbox = tool_output(
        &tool.bin,
        &["fix", "--all", "--sandbox", &source.display().to_string()],
        timeout,
        &[("LUST_NO_DAEMON", "1")],
    )?;
    if with_sandbox.success() {
        return Ok(with_sandbox);
    }
    let combined = format!("{}{}", with_sandbox.stderr, with_sandbox.stdout);
    let rejected = combined.contains("unexpected")
        || combined.contains("unrecognized")
        || combined.contains("unknown");
    if !rejected {
        return Ok(with_sandbox);
    }
    tool_output(
        &tool.bin,
        &["fix", "--all", &source.display().to_string()],
        timeout,
        &[("LUST_NO_DAEMON", "1")],
    )
}

pub fn build(
    tool: &LustTool,
    source: &Path,
    output: &Path,
    max_memory: u64,
    time_limit: Option<Duration>,
    timeout: Duration,
) -> Result<()> {
    let memory = max_memory.to_string();
    let mut args = vec!["build".to_string(), "--sandbox".to_string()];
    if tool.supports_max_memory {
        args.push("--max-memory".to_string());
        args.push(memory);
    }
    if tool.supports_time_limit {
        if let Some(limit) = time_limit {
            args.push("--time-limit".to_string());
            args.push(format_time_limit(limit));
        }
    }
    args.push("-o".to_string());
    args.push(output.display().to_string());
    args.push(source.display().to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = tool_output(&tool.bin, &arg_refs, timeout, &[("LUST_NO_DAEMON", "1")])?;
    if !result.success() {
        return Err(err(format!(
            "lust build failed ({:?}): {}",
            result.kind,
            tail(&format!("{}{}", result.stderr, result.stdout))
        )));
    }
    if !output.is_file() {
        return Err(err(format!(
            "lust build exited 0 but did not write {}",
            output.display()
        )));
    }
    Ok(())
}

/// Why rustc did not produce a binary.
///
/// `Rejected` is a compile error: the oracle records a divergence.
/// `NotOnPath`, `TimedOut`, and `Failed` are problems running the compiler,
/// and the caller returns them as errors.
#[derive(Debug)]
pub enum RustcError {
    Rejected { message: String },
    NotOnPath { message: String },
    TimedOut { message: String },
    Failed { message: String },
}

impl RustcError {
    pub fn message(&self) -> &str {
        match self {
            RustcError::Rejected { message }
            | RustcError::NotOnPath { message }
            | RustcError::TimedOut { message }
            | RustcError::Failed { message } => message,
        }
    }
}

impl fmt::Display for RustcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for RustcError {}

impl From<RustcError> for crate::error::Error {
    fn from(error: RustcError) -> Self {
        err(error.message())
    }
}

pub fn compile_rustc(
    source: &Path,
    output: &Path,
    edition: &str,
    timeout: Duration,
) -> std::result::Result<(), RustcError> {
    let rustc = which("rustc").ok_or_else(|| RustcError::NotOnPath {
        message: "rustc is not on PATH".to_string(),
    })?;
    let result = tool_output(
        &rustc,
        &[
            "--edition",
            edition,
            "-C",
            "opt-level=2",
            "-C",
            "debuginfo=0",
            "-C",
            "debug-assertions=on",
            "-C",
            "overflow-checks=on",
            "-o",
            &output.display().to_string(),
            &source.display().to_string(),
        ],
        timeout,
        &[],
    )
    .map_err(|error| RustcError::Failed {
        message: error.message,
    })?;
    match result.kind {
        StopKind::Exited(0) => Ok(()),
        StopKind::TimedOut => Err(RustcError::TimedOut {
            message: format!("rustc timed out after {}ms", timeout.as_millis()),
        }),
        StopKind::Exited(_) => Err(RustcError::Rejected {
            message: format!(
                "rustc failed: {}",
                tail(&format!("{}{}", result.stderr, result.stdout))
            ),
        }),
        StopKind::MemorySignal => Err(RustcError::Failed {
            message: "rustc was killed by the host memory limit".to_string(),
        }),
    }
}

/// Edition 2024, then 2021 only when rustc rejects the edition itself.
/// Returns the edition that worked.
pub fn compile_rustc_any_edition(
    source: &Path,
    output: &Path,
    timeout: Duration,
) -> std::result::Result<String, RustcError> {
    match compile_rustc(source, output, "2024", timeout) {
        Ok(()) => Ok("2024".to_string()),
        Err(problem) => {
            let edition_problem = match &problem {
                RustcError::Rejected { message } => {
                    let text = message.to_lowercase();
                    text.contains("edition")
                        && (text.contains("unknown")
                            || text.contains("invalid")
                            || text.contains("must be one of"))
                }
                _ => false,
            };
            if !edition_problem {
                return Err(problem);
            }
            compile_rustc(source, output, "2021", timeout)?;
            Ok("2021".to_string())
        }
    }
}

pub fn classify(result: &ExecResult) -> RunClass {
    if matches!(result.kind, StopKind::TimedOut) {
        return RunClass::TimedOut;
    }
    if matches!(result.kind, StopKind::MemorySignal) {
        return RunClass::MemoryHost;
    }
    let stderr = &result.stderr;
    if stderr.contains("error[L0533]") {
        return RunClass::TimeLust;
    }
    if stderr.contains("error[L0532]") && stderr.contains("max-memory") {
        return RunClass::MemoryLust;
    }
    if stderr.contains("error[L0532]") {
        return RunClass::SandboxDenied;
    }
    if stderr.contains("panicked at") {
        return RunClass::Panic;
    }
    if result.success() {
        RunClass::Clean
    } else {
        RunClass::Failed
    }
}

pub fn candidate_limits(wall: Duration, host_memory: u64) -> Limits {
    Limits {
        wall,
        memory_bytes: Some(host_memory),
        scrub_env: true,
    }
}

pub fn run_session(bin: &Path, limits: &Limits) -> Result<Session> {
    let cmd = Command::new(bin);
    Session::spawn(cmd, limits)
}

fn tool_output(
    bin: &Path,
    args: &[&str],
    timeout: Duration,
    env: &[(&str, &str)],
) -> Result<ExecResult> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    supervisor::run_capture(
        cmd,
        &Limits {
            wall: timeout,
            memory_bytes: None,
            scrub_env: false,
        },
        b"",
    )
}

fn parse_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        match serde_json::from_str::<DiagLine>(line) {
            Ok(value) => {
                if value.message.is_none() && value.code.is_none() && value.ok == Some(true) {
                    continue;
                }
                // `line` and `col` live under `span`. A missing field, or JSON
                // null, becomes an empty string or 0. `span.file` (the old fake
                // lust shape) is ignored; real lust puts the path at the top.
                let span = value.span.unwrap_or_default();
                out.push(Diagnostic {
                    code: value.code.unwrap_or_default(),
                    message: value.message.unwrap_or_default(),
                    severity: value.severity.unwrap_or_default(),
                    line: span.line.unwrap_or(0),
                    col: span.col.unwrap_or(0),
                    fix: value.fix.unwrap_or_default(),
                    auto_edits: value.edits.unwrap_or_default().len(),
                    raw: line.to_string(),
                });
            }
            // A field of the wrong type (a string `line`, an object `edits`)
            // fails the struct parse. Keep the text fields that are still
            // readable so the proposer hears about the error.
            Err(_) => {
                if let Some(diagnostic) = loose_diagnostic(line) {
                    out.push(diagnostic);
                }
            }
        }
    }
    out
}

#[derive(Deserialize)]
struct DiagLine {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    span: Option<DiagSpan>,
    #[serde(default)]
    fix: Option<String>,
    #[serde(default)]
    edits: Option<Vec<serde_json::Value>>,
}

#[derive(Default, Deserialize)]
struct DiagSpan {
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    col: Option<u32>,
}

/// Reads each field on its own after the strict parse fails. A number that
/// fits in `u32` is kept; anything else at `span.line` or `span.col` is 0.
/// `auto_edits` is the length of `edits` when that field is an array.
fn loose_diagnostic(line: &str) -> Option<Diagnostic> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let code = json_str(&value, "code");
    let message = json_str(&value, "message");
    if code.is_empty() && message.is_empty() {
        return None;
    }
    let span = value.get("span");
    Some(Diagnostic {
        code,
        message,
        severity: json_str(&value, "severity"),
        line: json_u32(span.and_then(|span| span.get("line"))),
        col: json_u32(span.and_then(|span| span.get("col"))),
        fix: json_str(&value, "fix"),
        auto_edits: value
            .get("edits")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        raw: line.to_string(),
    })
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn json_u32(value: Option<&serde_json::Value>) -> u32 {
    value
        .and_then(serde_json::Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(0)
}

fn first_line(stdout: &str, stderr: &str) -> String {
    stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn tail(text: &str) -> String {
    let text = text.trim();
    if text.len() <= 600 {
        text.to_string()
    } else {
        text[text.len() - 600..].to_string()
    }
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_lust_memory_and_timeout() {
        let memory = ExecResult {
            kind: StopKind::Exited(101),
            stdout: String::new(),
            stderr: "error[L0532]: this program was built with --max-memory 1048576 and tried to use more memory\n".into(),
            elapsed: Duration::from_millis(10),
        };
        assert_eq!(classify(&memory), RunClass::MemoryLust);
        let denied = ExecResult {
            kind: StopKind::Exited(101),
            stdout: String::new(),
            stderr: "error[L0532]: this program was built with --sandbox and tried to use fs\n"
                .into(),
            elapsed: Duration::from_millis(10),
        };
        assert_eq!(classify(&denied), RunClass::SandboxDenied);
        let timed = ExecResult {
            kind: StopKind::TimedOut,
            stdout: String::new(),
            stderr: String::new(),
            elapsed: Duration::from_millis(400),
        };
        assert_eq!(classify(&timed), RunClass::TimedOut);
        let guest = ExecResult {
            kind: StopKind::Exited(101),
            stdout: "before\n".into(),
            stderr: "error[L0533]: this program was built with --time-limit 300ms and tried to run longer\n".into(),
            elapsed: Duration::from_millis(300),
        };
        assert_eq!(classify(&guest), RunClass::TimeLust);
        let host = ExecResult {
            kind: StopKind::MemorySignal,
            stdout: String::new(),
            stderr: String::new(),
            elapsed: Duration::from_millis(20),
        };
        assert_eq!(classify(&host), RunClass::MemoryHost);
    }

    #[test]
    fn usage_without_the_flags_gives_false() {
        // The first two lines of `lust` with no arguments, lust 0.2.0.
        let text = "usage: lust <check|build|run> <file.rs> [--json] [--max-errors N] [--timings] [-o out] [-g] [--emit-c prefix] [-- args]
       lust <check|build|run> <file.rs> [--manifest-path <Cargo.toml>] [--package <name>] [--bin <name> | --lib] [--features <a,b>] [--no-default-features] [--all-features] [--target <triple>] [-C panic=unwind|abort] [--offline] [--locked] [--frozen]
";
        assert_eq!(flags_from_usage(text), (false, false));
    }

    #[test]
    fn an_error_naming_a_flag_is_not_usage() {
        assert_eq!(
            flags_from_usage("error: cannot read C:\\x\\--time-limit"),
            (false, false)
        );
    }

    #[test]
    fn usage_naming_the_flags_gives_true() {
        let text = "usage: lust <check|build|run> <file.rs> [--max-memory <size>]
       lust build <file.rs> [--time-limit <duration>]
";
        assert_eq!(flags_from_usage(text), (true, true));
    }

    #[test]
    fn parses_a_json_diagnostic() {
        let text = "{\"code\":\"L0102\",\"message\":\"cannot find value `BROKEN_TOKEN`\"}\n";
        let diags = parse_diagnostics(text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "L0102");
        assert_eq!(diags[0].severity, "");
        assert_eq!(diags[0].line, 0);
        assert_eq!(diags[0].col, 0);
        assert_eq!(diags[0].fix, "");
        assert_eq!(diags[0].auto_edits, 0);
    }

    #[test]
    fn parses_a_real_lust_diagnostic() {
        let text = r#"{"schema":1,"kind":"diagnostic","code":"L0110","severity":"error","message":"mismatched types: expected `u32`, found `&str`","file":"C:\\t\\bad.rs","span":{"start":29,"end":32,"line":2,"col":18,"end_line":2,"end_col":21},"labels":[],"fix":"Change the value to type `u32`, or change the expected type.","edits":[]}"#;
        let diags = parse_diagnostics(text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "L0110");
        assert_eq!(diags[0].line, 2);
        assert_eq!(diags[0].col, 18);
        assert!(diags[0].fix.starts_with("Change"));
    }

    #[test]
    fn parses_the_old_fake_lust_shape() {
        // `span.file`, no `severity`. The path is a JSON string, so a Windows
        // backslash arrives as `\\`.
        let text = r#"{"code":"L0102","message":"cannot find value `BROKEN_TOKEN` in this scope","span":{"file":"C:\\t\\broken.rs","line":1,"col":1},"fix":"replace BROKEN_TOKEN","edits":[{},{"kind":"replace"}]}"#;
        let diags = parse_diagnostics(text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "L0102");
        assert_eq!(diags[0].severity, "");
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].col, 1);
        assert_eq!(diags[0].fix, "replace BROKEN_TOKEN");
        assert_eq!(diags[0].auto_edits, 2);
    }

    #[test]
    fn keeps_code_and_message_when_a_field_has_the_wrong_type() {
        let text = "{\"schema\":1,\"ok\":true}\n\
{\"code\":\"L0110\",\"severity\":\"error\",\"message\":\"mismatched types\",\"span\":{\"line\":\"2\",\"col\":18},\"fix\":\"Change it\",\"edits\":{\"not\":\"a list\"}}\n\
{\"code\":\"L0101\",\"message\":\"cannot find value\",\"span\":{\"line\":3,\"col\":20},\"edits\":{}}\n";
        let diags = parse_diagnostics(text);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].code, "L0110");
        assert_eq!(diags[0].message, "mismatched types");
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].line, 0);
        assert_eq!(diags[0].col, 18);
        assert_eq!(diags[0].fix, "Change it");
        assert_eq!(diags[0].auto_edits, 0);
        assert!(diags[0].raw.contains("L0110"));
        assert_eq!(diags[1].code, "L0101");
        assert_eq!(diags[1].message, "cannot find value");
        assert_eq!(diags[1].line, 3);
        assert_eq!(diags[1].col, 20);
        assert_eq!(diags[1].auto_edits, 0);
    }
}
