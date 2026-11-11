//! Talks to an external `lust` binary. Crab Cage does not vendor lust.
//!
//! A candidate is checked with `lust check --json --sandbox`, optionally
//! repaired with `lust fix`, then built with
//! `lust build --sandbox --max-memory <bytes> --time-limit <duration> -o <bin>`.
//! `--time-limit` is passed when the binary advertises it. The host still
//! kills on the wall clock: a straight-line body is not probed, and a lust
//! that predates the flag has no guest deadline.

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
    let help = tool_output(
        bin,
        &["build", "--help"],
        timeout,
        &[("LUST_NO_DAEMON", "1")],
    )
    .unwrap_or(ExecResult {
        kind: StopKind::Exited(0),
        stdout: String::new(),
        stderr: String::new(),
        elapsed: Duration::ZERO,
    });
    // `lust build --help` is not a help page on a real binary: the client
    // treats `--help` as a file name. A limit flag with no value is a usage
    // error and prints the usage line, which names every flag.
    let usage = tool_output(
        bin,
        &["build", "--time-limit"],
        timeout,
        &[("LUST_NO_DAEMON", "1")],
    )
    .unwrap_or(ExecResult {
        kind: StopKind::Exited(2),
        stdout: String::new(),
        stderr: String::new(),
        elapsed: Duration::ZERO,
    });
    let help_text = format!(
        "{}{}{}{}",
        help.stdout, help.stderr, usage.stdout, usage.stderr
    );
    Ok(LustTool {
        bin: bin.to_path_buf(),
        version,
        supports_max_memory: help_text.contains("--max-memory"),
        supports_time_limit: help_text.contains("--time-limit"),
    })
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

pub fn compile_rustc(source: &Path, output: &Path, timeout: Duration) -> Result<()> {
    let rustc = which("rustc").ok_or_else(|| err("rustc is not on PATH"))?;
    let result = tool_output(
        &rustc,
        &[
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
            "-C",
            "debuginfo=0",
            "-o",
            &output.display().to_string(),
            &source.display().to_string(),
        ],
        timeout,
        &[],
    )?;
    if !result.success() {
        return Err(err(format!(
            "rustc failed: {}",
            tail(&format!("{}{}", result.stderr, result.stdout))
        )));
    }
    Ok(())
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
        if let Ok(value) = serde_json::from_str::<DiagLine>(line) {
            if value.message.is_none() && value.code.is_none() && value.ok == Some(true) {
                continue;
            }
            out.push(Diagnostic {
                code: value.code.unwrap_or_default(),
                message: value.message.unwrap_or_default(),
                raw: line.to_string(),
            });
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
    fn parses_a_json_diagnostic() {
        let text = "{\"code\":\"L0102\",\"message\":\"cannot find value `BROKEN_TOKEN`\"}\n";
        let diags = parse_diagnostics(text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "L0102");
    }
}
