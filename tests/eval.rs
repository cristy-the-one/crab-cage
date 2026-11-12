//! The funnel with no lust binary: rustc runs the fast gates.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crab_cage::lust;
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
        slot: 0,
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
    let failure = verdict.failure.expect("the first failing case");
    assert_eq!(failure.id, "negatives");
    assert_eq!(failure.input, "-4\n5\n");
    assert_eq!(failure.expected, "41\n");
    assert_eq!(failure.actual, "1\n");
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
    assert!(verdict.failure.is_none());
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
fn an_endless_loop_stops_after_the_first_unclean_case() {
    // The first case exits clean. The second loops. The third must not run.
    // The host wall clock is the nugget timeout plus 500 ms, and one of
    // those limits is well under two. Either the host kill (`TimedOut`) or
    // lust's own deadline (`TimeLust`) is a reject at `run`.
    let timeout_ms = 500u64;
    let mut nugget = tiny(Objective::PassAll, timeout_ms, Some("ok\n"));
    nugget.cases = vec![
        Case {
            id: "a".to_string(),
            input: "0\n".to_string(),
            expected: Some("ok\n".to_string()),
        },
        Case {
            id: "b".to_string(),
            input: "1\n".to_string(),
            expected: Some("ok\n".to_string()),
        },
        Case {
            id: "c".to_string(),
            input: "1\n".to_string(),
            expected: Some("ok\n".to_string()),
        },
    ];
    let candidate = "\
use std::io::Read;
let mut input = String::new();
let _ = std::io::stdin().read_to_string(&mut input);
if input.contains('1') {
    loop {}
}
print!(\"ok\\n\");
";
    let verdict = evaluate(&nugget, candidate, &no_lust(), &options("loop")).unwrap();
    assert_eq!(verdict.status, Status::Rejected, "{}", verdict.note);
    assert_eq!(verdict.gate, Gate::Run);
    assert!(
        verdict.note == "TimedOut" || verdict.note == "TimeLust",
        "note {}",
        verdict.note
    );
    assert!(verdict.score.is_none());
    assert_eq!(verdict.total, 3);
    assert_eq!(verdict.passed, 1);
    let host_ms = timeout_ms + 500;
    assert!(
        verdict.ms.run < 2 * host_ms,
        "run ms {} is not under 2 x {host_ms}",
        verdict.ms.run
    );
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
        failure: None,
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

fn options_oracle(label: &str) -> EvalOptions {
    let mut opts = options(label);
    opts.oracle = true;
    opts
}

/// Stand-in lust whose `build` writes a copy of itself that prints one fixed
/// wrong line. Built once: the tests in this file run in parallel.
fn wrong_lust() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(build_wrong_lust).clone()
}

fn build_wrong_lust() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("crab-cage-eval-wrong-lust-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("wrong_lust.rs");
    fs::write(&source, WRONG_LUST).unwrap();
    let output = dir.join(if cfg!(windows) {
        "wrong_lust.exe"
    } else {
        "wrong_lust"
    });
    let status = Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
            "-C",
            "debuginfo=0",
            "-o",
        ])
        .arg(&output)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success(), "fake lust failed to compile");
    output
}

const WRONG_LUST: &str = r#"
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

fn main() {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("wrong-lust"));
    let mode_path = PathBuf::from(format!("{}.crabmode", exe.display()));
    if let Ok(mode) = fs::read_to_string(&mode_path) {
        if mode.trim() == "wrong" {
            println!("wrong");
        }
        return;
    }
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("lust 0.3.0-fake");
        }
        None => {
            eprintln!("usage: lust <check|build|run> <file.rs> [--sandbox] [--max-memory <size>] [--time-limit <duration>]");
            process::exit(2);
        }
        Some("check") => {
            if !args.iter().any(|arg| arg == "--sandbox") {
                eprintln!("expected --sandbox");
                process::exit(3);
            }
            if args.iter().any(|arg| arg == "--json") {
                println!("{{\"schema\":1,\"ok\":true}}");
            }
        }
        Some("build") => cmd_build(&args, &exe),
        _ => {
            eprintln!("fake lust: unknown command");
            process::exit(2);
        }
    }
}

fn cmd_build(args: &[String], exe: &std::path::Path) {
    if !args.iter().any(|arg| arg == "--sandbox")
        || !args.iter().any(|arg| arg == "--max-memory")
        || !args.iter().any(|arg| arg == "--time-limit")
    {
        eprintln!("build requires --sandbox, --max-memory and --time-limit");
        process::exit(3);
    }
    let out = match flag(args, "-o") {
        Some(path) => path,
        None => {
            eprintln!("missing -o");
            process::exit(3);
        }
    };
    if let Err(err) = fs::copy(exe, &out) {
        eprintln!("copy failed: {err}");
        process::exit(4);
    }
    if let Err(err) = fs::write(format!("{out}.crabmode"), "wrong\n") {
        eprintln!("mode failed: {err}");
        process::exit(4);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
"#;

fn oracle_name(hash: &str) -> String {
    if cfg!(windows) {
        format!("{hash}.oracle.exe")
    } else {
        format!("{hash}.oracle")
    }
}

#[test]
fn oracle_without_lust_proves_a_correct_body() {
    let nugget = demo();
    let opts = options_oracle("oracle-proven");
    let verdict = evaluate(
        &nugget,
        "    xs.iter().map(|x| x * x).sum()\n",
        &no_lust(),
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Proven, "{}", verdict.note);
    assert_eq!(verdict.gate, Gate::Oracle);
    assert_eq!(verdict.score, Some(0.0));
    assert_eq!(verdict.passed, 3);
    assert_eq!(verdict.total, 3);
    assert!(verdict.note.is_empty(), "{}", verdict.note);
    assert!(!opts.work_dir.join("divergences").exists());
    assert!(
        !opts.work_dir.join(oracle_name(&verdict.hash)).exists(),
        "no lust means the fast build is already rustc"
    );
}

#[test]
fn oracle_with_the_fake_lust_diverges_and_writes_the_case() {
    let nugget = demo();
    let tool = lust::probe(&wrong_lust(), Duration::from_secs(30)).unwrap();
    let opts = options_oracle("oracle-diverged");
    let verdict = evaluate(
        &nugget,
        "    xs.iter().map(|x| x * x).sum()\n",
        &Toolchain { lust: Some(tool) },
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Diverged, "{}", verdict.note);
    assert_eq!(verdict.gate, Gate::Oracle);
    assert_eq!(verdict.score, Some(3.0));
    assert_eq!(verdict.passed, 0);
    assert_eq!(verdict.total, 3);
    assert!(verdict.ms.oracle > 0, "oracle ms {}", verdict.ms.oracle);
    let div = opts.work_dir.join("divergences").join(&verdict.hash);
    assert!(div.is_dir(), "missing {}", div.display());
    let source = fs::read_to_string(div.join("source.rs")).unwrap();
    assert!(source.contains("x * x"), "{source}");
    assert_eq!(
        fs::read_to_string(div.join("case.in")).unwrap(),
        nugget.cases[0].input
    );
    assert_eq!(
        fs::read_to_string(div.join("fast.stdout")).unwrap(),
        "wrong\n"
    );
    assert_eq!(
        fs::read_to_string(div.join("oracle.stdout")).unwrap(),
        nugget.cases[0].expected.as_deref().unwrap()
    );
    let readme = fs::read_to_string(div.join("README.txt")).unwrap();
    assert!(readme.contains("lust version: lust 0.3.0-fake"), "{readme}");
    assert!(readme.contains("build --sandbox"), "{readme}");
    assert!(
        readme.contains("rustc --edition 2024") || readme.contains("rustc --edition 2021"),
        "{readme}"
    );
    assert!(readme.contains("-C debug-assertions=on"), "{readme}");
    assert!(readme.contains("-C overflow-checks=on"), "{readme}");
    assert!(
        readme.contains(&format!("case: {}", nugget.cases[0].id)),
        "{readme}"
    );
}

#[test]
fn oracle_diverges_when_rustc_rejects_what_lust_built() {
    let tool = lust::probe(&wrong_lust(), Duration::from_secs(30)).unwrap();
    let opts = options_oracle("oracle-rejected");
    let verdict = evaluate(
        &demo(),
        "    this is not rust\n",
        &Toolchain { lust: Some(tool) },
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Diverged);
    assert_eq!(verdict.gate, Gate::Oracle);
    assert_eq!(verdict.note, "lust accepted, rustc rejected");
    let div = opts.work_dir.join("divergences").join(&verdict.hash);
    assert!(div.is_dir(), "missing {}", div.display());
    let stderr = fs::read_to_string(div.join("rustc.stderr")).unwrap();
    assert!(stderr.contains("rustc"), "{stderr}");
    let source = fs::read_to_string(div.join("source.rs")).unwrap();
    assert!(source.contains("this is not rust"), "{source}");
    let readme = fs::read_to_string(div.join("README.txt")).unwrap();
    assert!(readme.contains("lust version: lust 0.3.0-fake"), "{readme}");
    assert!(readme.contains("rustc --edition 2024"), "{readme}");
    assert!(readme.contains("-C debug-assertions=on"), "{readme}");
    assert!(readme.contains("-C overflow-checks=on"), "{readme}");
    assert!(!div.join("case.in").exists());
}

#[test]
fn rustc_keeps_debug_assertions_and_overflow_checks() {
    let debug = "\
    let base: i64 = xs.iter().map(|x| x * x).sum();
    if cfg!(debug_assertions) { base } else { base + 1 }
";
    let verdict = evaluate(&demo(), debug, &no_lust(), &options_oracle("oracle-debug")).unwrap();
    assert_eq!(verdict.status, Status::Proven, "{}", verdict.note);
    assert_eq!(verdict.score, Some(0.0), "{}", verdict.note);
    assert_eq!(verdict.passed, 3);

    let overflow = "\
    let value = std::hint::black_box(i64::MAX);
    let _ = value + 1;
    xs.iter().map(|x| x * x).sum()
";
    let verdict = evaluate(&demo(), overflow, &no_lust(), &options("oracle-overflow")).unwrap();
    assert_eq!(verdict.status, Status::Rejected, "{}", verdict.note);
    assert_eq!(verdict.gate, Gate::Run);
    assert_eq!(verdict.note, "Panic");
}

#[test]
fn oracle_rejects_output_that_changes_between_runs() {
    let tool = lust::probe(&wrong_lust(), Duration::from_secs(30)).unwrap();
    let opts = options_oracle("oracle-unstable");
    let candidate = "\
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0)
";
    let verdict = evaluate(&demo(), candidate, &Toolchain { lust: Some(tool) }, &opts).unwrap();
    assert_eq!(verdict.status, Status::Rejected, "{}", verdict.note);
    assert_eq!(verdict.gate, Gate::Oracle);
    assert_eq!(verdict.note, "nondeterministic output");
    assert!(!opts.work_dir.join("divergences").exists());
}

#[test]
fn oracle_returns_an_error_when_rustc_is_missing_or_times_out() {
    let source_dir = work("oracle-timeout-src");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("slow.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let output = source_dir.join(if cfg!(windows) { "slow.exe" } else { "slow" });
    let timed_out = lust::compile_rustc_any_edition(&source, &output, Duration::from_millis(1));
    match timed_out {
        Err(lust::RustcError::TimedOut { message }) => {
            assert!(message.contains("timed out"), "{message}");
        }
        other => panic!("expected a timeout, got {other:?}"),
    }

    let exe = crab_exe();
    let dir = work("oracle-no-rustc");
    fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.rs");
    fs::write(&good, "    xs.iter().map(|x| x * x).sum()\n").unwrap();
    let work_dir = dir.join("work");
    let output = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg(&good)
        .arg("--oracle")
        .arg("--lust-bin")
        .arg(wrong_lust())
        .arg("--work-dir")
        .arg(&work_dir)
        .env("PATH", &dir)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {}\nstderr {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rustc is not on PATH"), "{stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("diverged"), "{stdout}");
    assert!(
        !stdout.contains("lust accepted, rustc rejected"),
        "{stdout}"
    );
    assert!(!work_dir.join("divergences").exists());
}

#[test]
fn cli_oracle_proves_without_lust_and_diverges_with_the_fake() {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    let dir = work("cli-oracle");
    fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.rs");
    fs::write(&good, "    xs.iter().map(|x| x * x).sum()\n").unwrap();

    let good_work = dir.join("good-work");
    let output = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg(&good)
        .arg("--json")
        .arg("--oracle")
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
    assert!(stdout.contains("\"status\":\"proven\""), "{stdout}");
    assert!(stdout.contains("\"gate\":\"oracle\""), "{stdout}");
    assert!(stdout.contains("\"score\":0.0"), "{stdout}");

    let fake_work = dir.join("fake-work");
    let output = Command::new(&exe)
        .arg("eval")
        .arg(demo_dir())
        .arg(&good)
        .arg("--json")
        .arg("--oracle")
        .arg("--lust-bin")
        .arg(wrong_lust())
        .arg("--work-dir")
        .arg(&fake_work)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"status\":\"diverged\""), "{stdout}");
    assert!(stdout.contains("\"gate\":\"oracle\""), "{stdout}");
    let hash = json_string_field(&stdout, "hash");
    assert!(fake_work.join("divergences").join(hash).is_dir());
}

fn json_string_field(stdout: &str, key: &str) -> String {
    let pattern = format!("\"{key}\":\"");
    let start = stdout
        .find(&pattern)
        .unwrap_or_else(|| panic!("no {key} in {stdout}"))
        + pattern.len();
    let rest = &stdout[start..];
    let end = rest
        .find('"')
        .unwrap_or_else(|| panic!("unterminated {key} in {stdout}"));
    rest[..end].to_string()
}

fn fast_sum_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("nuggets")
        .join("fast-sum")
}

fn fast_sum_reference() -> String {
    fs::read_to_string(fast_sum_dir().join("reference.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

/// Timing is noisy on a loaded machine, so this asserts only the order, and
/// only after the gap is large: the seed does about n * n / 2 steps (5e7 for
/// the largest case) and the reference does a constant few. A run that is
/// too close to call is repeated, up to three times, before it fails.
#[test]
fn fastest_seed_is_slower_than_the_reference() {
    let nugget = nugget::load(&fast_sum_dir()).unwrap();
    assert_eq!(nugget.objective, Objective::Fastest);
    let reference = fast_sum_reference();
    let mut last = (0.0, 0.0);
    for attempt in 0..3 {
        let seed = evaluate(
            &nugget,
            &nugget.seed,
            &no_lust(),
            &options(&format!("fast-seed-{attempt}")),
        )
        .unwrap();
        assert_eq!(seed.status, Status::Scored, "{seed:?}");
        assert_eq!(seed.passed, 3);
        assert!(seed.note.is_empty(), "{}", seed.note);
        let fast = evaluate(
            &nugget,
            &reference,
            &no_lust(),
            &options(&format!("fast-reference-{attempt}")),
        )
        .unwrap();
        assert_eq!(fast.status, Status::Scored, "{fast:?}");
        assert_eq!(fast.passed, 3);
        let seed_ms = seed.score.unwrap();
        let fast_ms = fast.score.unwrap();
        assert!(seed_ms > 0.0 && seed_ms < 1e9, "seed {seed_ms}");
        assert!(fast_ms > 0.0 && fast_ms < 1e9, "reference {fast_ms}");
        assert!(seed.ms.opt > 0, "seed opt ms {}", seed.ms.opt);
        last = (seed_ms, fast_ms);
        if fast_ms < seed_ms {
            return;
        }
    }
    panic!(
        "the reference ({} ms) was never faster than the seed ({} ms)",
        last.1, last.0
    );
}

#[test]
fn fastest_wrong_body_scores_at_least_a_billion() {
    let nugget = nugget::load(&fast_sum_dir()).unwrap();
    let verdict = evaluate(&nugget, "    n\n", &no_lust(), &options("fast-wrong")).unwrap();
    assert_eq!(verdict.status, Status::Scored, "{verdict:?}");
    assert_eq!(verdict.passed, 0);
    assert_eq!(verdict.score, Some(1e9 + 3.0));
    assert!(verdict.score.unwrap() >= 1e9);
    assert_eq!(verdict.ms.opt, 0, "a wrong candidate is not timed");
    let failure = verdict.failure.expect("the first failing case");
    assert_eq!(failure.id, "large");
}

#[test]
fn fastest_with_the_oracle_and_no_lust_is_proven_with_a_time() {
    let nugget = nugget::load(&fast_sum_dir()).unwrap();
    let verdict = evaluate(
        &nugget,
        &fast_sum_reference(),
        &no_lust(),
        &options_oracle("fast-oracle"),
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Proven, "{verdict:?}");
    let score = verdict.score.unwrap();
    assert!(score > 0.0 && score < 1e9, "{score}");
}

#[test]
fn an_old_journal_line_without_ms_opt_still_parses() {
    let line = r#"{"hash":"aaaaaaaaaaaaaaaa","gate":"run","status":"scored","score":1.0,"passed":1,"total":2,"diagnostics":[],"ms":{"check":0,"build":1,"run":2,"oracle":0},"note":""}"#;
    let verdict: Verdict = serde_json::from_str(line).unwrap();
    assert_eq!(verdict.ms.opt, 0);
    assert_eq!(verdict.ms.run, 2);
    let encoded = serde_json::to_string(&verdict).unwrap();
    assert!(encoded.contains("\"opt\":0"), "{encoded}");
}

/// Stand-in lust whose `build` copies itself as a binary that prints the
/// fast-sum answer for the number on stdin, and whose `build --opt` fails
/// the way lust 0.3.0 does on Windows without zig. Built once.
fn no_cc_lust() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(build_no_cc_lust).clone()
}

fn build_no_cc_lust() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("crab-cage-eval-no-cc-lust-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("no_cc_lust.rs");
    fs::write(&source, NO_CC_LUST).unwrap();
    let output = dir.join(if cfg!(windows) {
        "no_cc_lust.exe"
    } else {
        "no_cc_lust"
    });
    let status = Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
            "-C",
            "debuginfo=0",
            "-o",
        ])
        .arg(&output)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success(), "fake lust failed to compile");
    output
}

const NO_CC_LUST: &str = r#"
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process;

fn main() {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("no-cc-lust"));
    let mode_path = PathBuf::from(format!("{}.crabmode", exe.display()));
    if let Ok(mode) = fs::read_to_string(&mode_path) {
        if mode.trim() == "wrong" {
            println!("wrong");
            return;
        }
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let n: u64 = input.trim().parse().unwrap_or(0);
        let mut sum = 0u64;
        for i in 1..=n {
            sum = (sum + i * i % 1_000_000_007) % 1_000_000_007;
        }
        println!("{sum}");
        return;
    }
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => println!("lust 0.3.0-no-cc"),
        None => {
            eprintln!("usage: lust <check|build|run> <file.rs> [--sandbox] [--max-memory <size>] [--time-limit <duration>] [--opt]");
            process::exit(2);
        }
        Some("check") => {
            if args.iter().any(|arg| arg == "--json") {
                println!("{{\"schema\":1,\"ok\":true}}");
            }
        }
        Some("build") => {
            let opt = args.iter().any(|arg| arg == "--opt");
            // A copy named `wrong_opt_lust` has a C compiler, but its opt
            // binary prints the wrong line.
            let wrong_opt = exe
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy().contains("wrong_opt"));
            if opt && !wrong_opt {
                eprintln!("error: no C compiler for target `x86_64-pc-windows-msvc`: install zig (zig cc cross-compiles it) or set LUST_CC to a cross compiler command");
                process::exit(3);
            }
            let Some(out) = args
                .iter()
                .position(|arg| arg == "-o")
                .and_then(|index| args.get(index + 1))
            else {
                eprintln!("missing -o");
                process::exit(3);
            };
            if let Err(err) = fs::copy(&exe, out) {
                eprintln!("copy failed: {err}");
                process::exit(4);
            }
            let mode = if opt { "wrong\n" } else { "answer\n" };
            if let Err(err) = fs::write(format!("{out}.crabmode"), mode) {
                eprintln!("mode failed: {err}");
                process::exit(4);
            }
        }
        _ => {
            eprintln!("fake lust: unknown command");
            process::exit(2);
        }
    }
}
"#;

#[test]
fn fastest_without_a_c_compiler_times_the_tcc_build_and_says_so() {
    let nugget = nugget::load(&fast_sum_dir()).unwrap();
    let tool = lust::probe(&no_cc_lust(), Duration::from_secs(30)).unwrap();
    let opts = options("fast-no-cc");
    let verdict = evaluate(
        &nugget,
        &fast_sum_reference(),
        &Toolchain { lust: Some(tool) },
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Scored, "{verdict:?}");
    assert_eq!(verdict.gate, Gate::Run);
    assert_eq!(verdict.note, "opt build unavailable, timed the tcc build");
    assert_eq!(verdict.passed, 3);
    let score = verdict.score.unwrap();
    assert!(score > 0.0 && score < 1e9, "{score}");
    assert!(verdict.ms.opt > 0, "opt ms {}", verdict.ms.opt);
    let opt_bin = if cfg!(windows) {
        format!("{}.opt.exe", verdict.hash)
    } else {
        format!("{}.opt", verdict.hash)
    };
    assert!(!opts.work_dir.join(opt_bin).exists());
    assert!(!opts.work_dir.join("divergences").exists());
}

/// The same stand-in under the name `wrong_opt_lust`: its `--opt` build
/// succeeds and prints a wrong line. Copied once.
fn wrong_opt_lust() -> PathBuf {
    static COPIED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    COPIED
        .get_or_init(|| {
            let original = no_cc_lust();
            let name = if cfg!(windows) {
                "wrong_opt_lust.exe"
            } else {
                "wrong_opt_lust"
            };
            let copy = original.with_file_name(name);
            fs::copy(&original, &copy).unwrap();
            // On Linux a file cannot be executed while any process holds it
            // open for writing ("Text file busy"). Tests run on parallel
            // threads, and a child that another thread forked during the
            // copy keeps the copy's descriptor until its own exec. Wait here
            // until the copy can be started, so no test sees that window.
            for _ in 0..250 {
                match std::process::Command::new(&copy).arg("--version").output() {
                    Ok(_) => break,
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
            copy
        })
        .clone()
}

#[test]
fn fastest_opt_build_with_a_different_output_diverges_and_writes_the_case() {
    let nugget = nugget::load(&fast_sum_dir()).unwrap();
    let tool = lust::probe(&wrong_opt_lust(), Duration::from_secs(30)).unwrap();
    let opts = options("fast-wrong-opt");
    let verdict = evaluate(
        &nugget,
        &fast_sum_reference(),
        &Toolchain { lust: Some(tool) },
        &opts,
    )
    .unwrap();
    assert_eq!(verdict.status, Status::Diverged, "{verdict:?}");
    assert_eq!(verdict.passed, 3, "the tcc build printed every answer");
    assert!(verdict.score.is_none());
    let first = &nugget.cases[0];
    assert!(verdict.note.contains(&first.id), "{}", verdict.note);
    let div = opts.work_dir.join("divergences").join(&verdict.hash);
    assert_eq!(
        fs::read_to_string(div.join("case.in")).unwrap(),
        first.input
    );
    assert_eq!(
        fs::read_to_string(div.join("opt.stdout")).unwrap(),
        "wrong\n"
    );
    assert_eq!(
        fs::read_to_string(div.join("fast.stdout")).unwrap(),
        first.expected.as_deref().unwrap()
    );
    let readme = fs::read_to_string(div.join("README.txt")).unwrap();
    assert!(readme.contains("build --sandbox --opt"), "{readme}");
    assert!(readme.contains(&format!("case: {}", first.id)), "{readme}");
}

/// Each example nugget with a `reference.rs` is consistent: its seed fails
/// the number of cases recorded here, and the reference passes them all.
#[test]
fn example_nuggets_have_a_wrong_seed_and_a_right_reference() {
    for (name, cases, seed_fails) in [
        ("run-lengths", 8u32, 7u32),
        ("merge-ranges", 9, 7),
        ("normalize-path", 10, 8),
    ] {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("nuggets")
            .join(name);
        let nugget = nugget::load(&dir).unwrap();
        assert_eq!(nugget.cases.len(), cases as usize, "{name}");

        let seed = evaluate(
            &nugget,
            &nugget.seed,
            &no_lust(),
            &options(&format!("{name}-seed")),
        )
        .unwrap();
        assert_eq!(seed.status, Status::Scored, "{name}: {seed:?}");
        assert_eq!(seed.score, Some(f64::from(seed_fails)), "{name}");
        assert_eq!(seed.passed, cases - seed_fails, "{name}");

        let reference = fs::read_to_string(dir.join("reference.rs"))
            .unwrap()
            .replace("\r\n", "\n");
        let solved = evaluate(
            &nugget,
            &reference,
            &no_lust(),
            &options(&format!("{name}-reference")),
        )
        .unwrap();
        assert_eq!(solved.status, Status::Scored, "{name}: {solved:?}");
        assert_eq!(solved.score, Some(0.0), "{name}");
        assert_eq!(solved.passed, cases, "{name}");
    }
}
