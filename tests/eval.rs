//! The funnel with no lust binary: rustc runs the fast gates.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crab_cage::nugget::{self, Case, Nugget, Objective};
use crab_cage::pipeline::{
    evaluate, format_verdict, Diagnostic, EvalOptions, Gate, Ms, Status, Toolchain, Verdict,
};

fn demo() -> Nugget {
    nugget::load(&demo_dir()).unwrap()
}

fn demo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("nuggets")
        .join("sum-squares")
}

fn work(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crab-cage-eval-{label}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    dir
}

fn options(label: &str) -> EvalOptions {
    EvalOptions {
        work_dir: work(label),
        host_memory: 1024 * 1024 * 1024,
        compile_timeout: Duration::from_secs(120),
        oracle: false,
    }
}

fn no_lust() -> Toolchain {
    Toolchain { lust: None }
}

fn crab_exe() -> PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let name = if cfg!(windows) {
        "crab-cage.exe"
    } else {
        "crab-cage"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(name)
}

fn tiny(objective: Objective, timeout_ms: u64, expected: Option<&str>) -> Nugget {
    Nugget {
        name: "probe".to_string(),
        objective,
        timeout: Duration::from_millis(timeout_ms),
        max_memory: 64 * 1024 * 1024,
        prefix: "fn main() {\n// CRAB:BEGIN\n".to_string(),
        seed: String::new(),
        suffix: "// CRAB:END\n}\n".to_string(),
        region_first_line: 3,
        cases: vec![Case {
            id: "only".to_string(),
            input: String::new(),
            expected: expected.map(str::to_string),
        }],
    }
}

#[test]
fn seed_scores_every_demo_case_as_a_failure() {
    let nugget = demo();
    let opts = options("seed");
    let verdict = evaluate(&nugget, &nugget.seed, &no_lust(), &opts).unwrap();
    assert_eq!(verdict.status, Status::Scored);
    assert_eq!(verdict.gate, Gate::Run);
    assert_eq!(verdict.score, Some(3.0));
    assert_eq!(verdict.passed, 0);
    assert_eq!(verdict.total, 3);
    assert_eq!(verdict.ms.check, 0);
    assert!(verdict.ms.build > 0, "build ms {}", verdict.ms.build);
    assert_eq!(verdict.hash.len(), 16);
    assert!(verdict.hash.chars().all(|c| c.is_ascii_hexdigit()));
    let source = opts.work_dir.join(format!("{}.rs", verdict.hash));
    let text = fs::read_to_string(source).unwrap();
    assert!(text.contains(&nugget.seed));
    assert!(text.contains("// CRAB:BEGIN"));
}

#[test]
fn correct_body_scores_zero() {
    let nugget = demo();
    let opts = options("correct");
    let candidate = "    xs.iter().map(|x| x * x).sum()\n";
    let verdict = evaluate(&nugget, candidate, &no_lust(), &opts).unwrap();
    assert_eq!(verdict.status, Status::Scored);
    assert_eq!(verdict.gate, Gate::Run);
    assert_eq!(verdict.score, Some(0.0));
    assert_eq!(verdict.passed, 3);
    assert_eq!(verdict.total, 3);
    assert!(verdict.note.is_empty(), "{}", verdict.note);
    let source = fs::read_to_string(opts.work_dir.join(format!("{}.rs", verdict.hash))).unwrap();
    assert!(source.contains("x * x"));
    let name = if cfg!(windows) {
        format!("{}.exe", verdict.hash)
    } else {
        verdict.hash.clone()
    };
    assert!(opts.work_dir.join(name).is_file());
}

#[test]
fn trailing_whitespace_still_matches_pass_all() {
    let nugget = tiny(Objective::PassAll, 2000, Some("ok\n"));
    let verdict = evaluate(
        &nugget,
        "print!(\"ok  \\n\");\n",
        &no_lust(),
        &options("trim"),
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Scored, "{}", verdict.note);
    assert_eq!(verdict.score, Some(0.0));
    assert_eq!(verdict.passed, 1);
}

#[test]
fn syntax_error_is_rejected_at_build() {
    let verdict = evaluate(
        &demo(),
        "    this is not rust\n",
        &no_lust(),
        &options("syntax"),
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.gate, Gate::Build);
    assert!(verdict.score.is_none());
    assert!(verdict.note.contains("rustc"), "{}", verdict.note);
}

#[test]
fn filesystem_is_rejected_at_prefilter_before_any_file_is_written() {
    let opts = options("fs");
    let verdict = evaluate(
        &demo(),
        "    let _ = std::fs::read(\"a\");\n",
        &no_lust(),
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.gate, Gate::Prefilter);
    assert!(verdict.note.contains("std::fs"), "{}", verdict.note);
    assert!(verdict.score.is_none());
    assert!(!opts.work_dir.exists());
}

#[test]
fn a_panic_is_rejected_at_run_with_the_class_name() {
    let verdict = evaluate(
        &demo(),
        "    panic!(\"nope\");\n",
        &no_lust(),
        &options("panic"),
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.gate, Gate::Run);
    assert_eq!(verdict.note, "Panic");
    assert!(verdict.score.is_none());
}

#[test]
fn an_endless_loop_is_rejected_at_run_by_either_clock() {
    // No lust here, so the host wall clock is the clock that can fire.
    // A real lust may instead stop the guest with L0533 (`TimeLust`).
    // Either class is a reject at `run`; this test accepts both.
    let nugget = tiny(Objective::PassAll, 500, Some("ok\n"));
    let verdict = evaluate(&nugget, "loop {}\n", &no_lust(), &options("loop")).unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.gate, Gate::Run);
    assert!(
        verdict.note == "TimedOut" || verdict.note == "TimeLust",
        "note {}",
        verdict.note
    );
    assert!(verdict.ms.run < 8_000, "run ms {}", verdict.ms.run);
}

#[test]
fn minimize_sums_score_lines_and_rejects_a_missing_or_non_finite_one() {
    let nugget = Nugget {
        name: "min".to_string(),
        objective: Objective::Minimize,
        timeout: Duration::from_millis(2000),
        max_memory: 64 * 1024 * 1024,
        prefix: "fn main() {\n// CRAB:BEGIN\n".to_string(),
        seed: String::new(),
        suffix: "// CRAB:END\n}\n".to_string(),
        region_first_line: 3,
        cases: vec![
            Case {
                id: "a".to_string(),
                input: String::new(),
                expected: None,
            },
            Case {
                id: "b".to_string(),
                input: String::new(),
                expected: None,
            },
        ],
    };
    let scored = evaluate(
        &nugget,
        "println!(\"score: 1.25\");\n",
        &no_lust(),
        &options("min-ok"),
    )
    .unwrap();
    assert_eq!(scored.status, Status::Scored, "{}", scored.note);
    assert_eq!(scored.score, Some(2.5));
    assert_eq!(scored.passed, 2);

    let missing = evaluate(
        &nugget,
        "println!(\"nope\");\n",
        &no_lust(),
        &options("min-miss"),
    )
    .unwrap();
    assert_eq!(missing.status, Status::Rejected);
    assert_eq!(missing.gate, Gate::Run);
    assert_eq!(missing.note, "missing score");

    let infinite = evaluate(
        &nugget,
        "println!(\"score: inf\");\n",
        &no_lust(),
        &options("min-inf"),
    )
    .unwrap();
    assert_eq!(infinite.status, Status::Rejected);
    assert_eq!(infinite.gate, Gate::Run);
    assert_eq!(infinite.note, "non-finite score");
}

#[test]
fn human_verdict_is_three_lines() {
    let rejected = Verdict {
        hash: "0123456789abcdef".to_string(),
        gate: Gate::Build,
        status: Status::Rejected,
        score: None,
        passed: 0,
        total: 3,
        diagnostics: Vec::new(),
        ms: Ms::default(),
        note: "rustc failed: first\nsecond".to_string(),
    };
    let text = format_verdict(&rejected);
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(
        text.starts_with("rejected build 0123456789abcdef\n"),
        "{text}"
    );
    assert!(text.contains("passed 0/3 score -\n"), "{text}");
    assert!(text.contains("first second"), "{text}");

    let diagnosed = Verdict {
        note: String::new(),
        gate: Gate::Check,
        diagnostics: vec![Diagnostic {
            code: "L0110".to_string(),
            message: "mismatched types".to_string(),
            line: 1,
            col: 9,
            fix: "Change".to_string(),
        }],
        ..rejected
    };
    let text = format_verdict(&diagnosed);
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("L0110:1:9: mismatched types"), "{text}");
}

#[test]
fn cli_json_exits_zero_only_when_scored_and_human_output_is_three_lines() {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    let dir = work("cli");
    fs::create_dir_all(&dir).unwrap();

    let good = dir.join("good.rs");
    fs::write(&good, "    xs.iter().map(|x| x * x).sum()\n").unwrap();
    let good_work = dir.join("good-work");
    let output = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg(&good)
        .arg("--json")
        .arg("--work-dir")
        .arg(&good_work)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("\"status\":\"scored\""), "{stdout}");
    assert!(stdout.contains("\"gate\":\"run\""), "{stdout}");
    assert!(stdout.contains("\"score\":0.0"), "{stdout}");
    assert!(stdout.contains("\"oracle\":0"), "{stdout}");

    let bad = dir.join("bad.rs");
    fs::write(&bad, "let _ = std::fs::read(\"a\");\n").unwrap();
    let bad_work = dir.join("bad-work");
    let output = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg(&bad)
        .arg("--work-dir")
        .arg(&bad_work)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert!(lines[0].contains("rejected"), "{stdout}");
    assert!(lines[0].contains("prefilter"), "{stdout}");
    assert!(stdout.contains("std::fs"), "{stdout}");
    assert!(!bad_work.exists());

    let stdin_work = dir.join("stdin-work");
    let mut child = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg("-")
        .arg("--json")
        .arg("--work-dir")
        .arg(&stdin_work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"std::fs::read(\"x\");\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"gate\":\"prefilter\""), "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
}
