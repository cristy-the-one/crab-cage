//! `crab-cage run` with the folder proposer. No lust binary: rustc is the fast path.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crab_cage::nugget;

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

fn demo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("nuggets")
        .join("sum-squares")
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crab-cage-run-{label}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(out: &PathBuf, args: &[&str]) -> std::process::Output {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    Command::new(exe)
        .current_dir(out)
        .args(args)
        .output()
        .unwrap()
}

fn journal_records(out: &PathBuf) -> Vec<serde_json::Value> {
    let text = fs::read_to_string(out.join("journal.jsonl")).unwrap();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn demo_folder_run_proves_the_right_snippet() {
    let out = scratch("demo");
    let candidates = scratch("demo-cands");
    let wrong = "    0\n";
    let right = "    xs.iter().map(|x| x * x).sum()\n";
    fs::write(candidates.join("a-wrong.rs"), wrong).unwrap();
    fs::write(candidates.join("b-right.rs"), right).unwrap();
    // Sorted after the right snippet. A score of 0 on pass_all stops first,
    // so this file must not become a fourth journal line.
    fs::write(candidates.join("c-later.rs"), "    std::fs::read(\"x\");\n").unwrap();

    let output = run(
        &out,
        &[
            "run",
            demo_dir().to_str().unwrap(),
            "--candidates",
            candidates.to_str().unwrap(),
            "--budget",
            "20",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?} stdout {stdout} stderr {stderr}",
        output.status.code()
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() >= 3, "{stdout}");
    assert_eq!(lines[0], "1 proven oracle 3 seed");
    assert_eq!(lines[1], "2 scored run 3 folder");
    assert_eq!(lines[2], "3 proven oracle 0 folder");
    assert!(stdout.contains("candidates 3\n"), "{stdout}");
    assert!(stdout.contains("proven=2"), "{stdout}");
    assert!(stdout.contains("scored=1"), "{stdout}");
    assert!(stdout.contains("best 0 "), "{stdout}");

    let nugget = nugget::load(&demo_dir()).unwrap();
    let region = fs::read_to_string(out.join("best.region.rs")).unwrap();
    assert_eq!(region, right);
    let best = fs::read_to_string(out.join("best.rs")).unwrap();
    assert_eq!(best, nugget.splice(&region));
    assert!(best.contains("fn main()"));

    let records = journal_records(&out);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["proposer"], "seed");
    assert_eq!(records[0]["status"], "proven");
    assert_eq!(records[0]["gate"], "oracle");
    assert_eq!(records[0]["score"], 3.0);
    assert_eq!(records[0]["n"], 1);
    assert_eq!(records[1]["proposer"], "folder");
    assert_eq!(records[1]["status"], "scored");
    assert_eq!(records[1]["gate"], "run");
    assert_eq!(records[1]["score"], 3.0);
    assert_eq!(records[1]["n"], 2);
    assert_eq!(records[2]["proposer"], "folder");
    assert_eq!(records[2]["status"], "proven");
    assert_eq!(records[2]["score"], 0.0);
    assert_eq!(records[2]["n"], 3);
    assert_ne!(records[0]["hash"], records[1]["hash"]);
    assert_ne!(records[1]["hash"], records[2]["hash"]);
}

#[test]
fn budget_counts_the_seed_before_a_snippet() {
    let out = scratch("budget");
    let candidates = scratch("budget-cands");
    fs::write(
        candidates.join("right.rs"),
        "    xs.iter().map(|x| x * x).sum()\n",
    )
    .unwrap();
    let output = run(
        &out,
        &[
            "run",
            demo_dir().to_str().unwrap(),
            "--candidates",
            candidates.to_str().unwrap(),
            "--budget",
            "1",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a pass_all run without a proven zero exits 1\nstdout {stdout} stderr {stderr}"
    );
    assert!(
        stdout.lines().next().unwrap().ends_with(" seed"),
        "{stdout}"
    );
    assert!(stdout.contains("candidates 1\n"), "{stdout}");
    let records = journal_records(&out);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["proposer"], "seed");
    assert_eq!(records[0]["status"], "proven");
    let region = fs::read_to_string(out.join("best.region.rs")).unwrap();
    assert_eq!(region, "    xs.iter().sum()\n");
    assert!(!region.contains("x * x"));
}

#[test]
fn a_rejected_seed_exits_nonzero_and_writes_no_best() {
    let out = scratch("reject");
    let nugget_dir = scratch("reject-nugget");
    fs::create_dir_all(nugget_dir.join("cases")).unwrap();
    fs::write(
        nugget_dir.join("nugget.json"),
        r#"{"name":"probe","objective":"pass_all"}"#,
    )
    .unwrap();
    fs::write(
        nugget_dir.join("template.rs"),
        "// CRAB:BEGIN\n    std::fs::read(\"a\");\n// CRAB:END\n",
    )
    .unwrap();
    fs::write(nugget_dir.join("cases").join("only.in"), "").unwrap();
    fs::write(nugget_dir.join("cases").join("only.out"), "ok\n").unwrap();

    let output = run(
        &out,
        &[
            "run",
            nugget_dir.to_str().unwrap(),
            "--budget",
            "5",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert!(
        stdout
            .lines()
            .next()
            .unwrap()
            .starts_with("1 rejected prefilter - seed"),
        "{stdout}"
    );
    assert!(stdout.contains("proven=0"), "{stdout}");
    assert!(!out.join("best.rs").exists());
    assert!(!out.join("best.region.rs").exists());
    let records = journal_records(&out);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "rejected");
    assert_eq!(records[0]["gate"], "prefilter");
    assert!(records[0]["score"].is_null());
}

#[test]
fn a_zero_budget_records_nothing() {
    let out = scratch("zero");
    let output = run(
        &out,
        &[
            "run",
            demo_dir().to_str().unwrap(),
            "--budget",
            "0",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert!(stdout.contains("candidates 0\n"), "{stdout}");
    assert!(stdout.contains("proven=0"), "{stdout}");
    assert!(stdout.contains("best none\n"), "{stdout}");
    assert_eq!(journal_records(&out).len(), 0);
    assert!(!out.join("best.rs").exists());
}

#[test]
fn a_resumed_run_does_not_replace_the_best_with_a_worse_candidate() {
    let out = scratch("resume");
    let candidates = scratch("resume-cands");
    let better = "    if xs.len() == 4 { 0 } else { xs.iter().map(|x| x * x).sum() }\n";
    let worse = "    if xs.len() == 1 { 49 } else { 0 }\n";
    fs::write(candidates.join("a-better.rs"), better).unwrap();

    let first = run(
        &out,
        &[
            "run",
            demo_dir().to_str().unwrap(),
            "--candidates",
            candidates.to_str().unwrap(),
            "--budget",
            "20",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&first.stdout);
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert_eq!(
        first.status.code(),
        Some(1),
        "score 1 is not solved\nstdout {stdout} stderr {stderr}"
    );
    let region_before = fs::read_to_string(out.join("best.region.rs")).unwrap();
    let best_before = fs::read_to_string(out.join("best.rs")).unwrap();
    assert_eq!(region_before, better);
    let before = journal_records(&out);
    assert_eq!(before.len(), 2);
    assert_eq!(before[1]["status"], "proven");
    assert_eq!(before[1]["score"], 1.0);

    fs::write(candidates.join("b-worse.rs"), worse).unwrap();
    let second = run(
        &out,
        &[
            "run",
            demo_dir().to_str().unwrap(),
            "--candidates",
            candidates.to_str().unwrap(),
            "--budget",
            "20",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert_eq!(
        second.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert_eq!(
        fs::read_to_string(out.join("best.region.rs")).unwrap(),
        region_before
    );
    assert_eq!(
        fs::read_to_string(out.join("best.rs")).unwrap(),
        best_before
    );
    let after = journal_records(&out);
    assert_eq!(after.len(), 3);
    assert_eq!(after[2]["status"], "scored");
    assert_eq!(after[2]["score"], 2.0);
    assert_eq!(after[2]["proposer"], "folder");
    assert!(stdout.contains("best 1 "), "{stdout}");
}

#[test]
fn fifty_seen_proposals_stop_the_run_before_another_file() {
    let out = scratch("dry-stop");
    let nugget_dir = scratch("dry-stop-nugget");
    let candidates = scratch("dry-stop-cands");
    let seed = "    std::fs::read(\"a\");\n";
    write_probe(&nugget_dir, "pass_all", seed, Some("ok\n"));
    for index in 0..50 {
        fs::write(candidates.join(format!("{index:02}.rs")), seed).unwrap();
    }
    fs::write(candidates.join("50.rs"), "    std::env::var(\"X\");\n").unwrap();

    let output = run_nugget(&out, &nugget_dir, Some(&candidates), "80");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert!(
        stdout.contains("stopped: 50 proposals in a row left no journal line"),
        "{stdout}"
    );
    let records = journal_records(&out);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["proposer"], "seed");
    assert!(
        records[0]["note"].as_str().unwrap().contains("std::fs"),
        "{records:?}"
    );
}

#[test]
fn forty_nine_seen_proposals_still_journal_the_next_new_candidate() {
    let out = scratch("dry-continue");
    let nugget_dir = scratch("dry-continue-nugget");
    let candidates = scratch("dry-continue-cands");
    let seed = "    std::fs::read(\"a\");\n";
    write_probe(&nugget_dir, "pass_all", seed, Some("ok\n"));
    for index in 0..49 {
        fs::write(candidates.join(format!("{index:02}.rs")), seed).unwrap();
    }
    fs::write(candidates.join("49.rs"), "    std::env::var(\"X\");\n").unwrap();

    let output = run_nugget(&out, &nugget_dir, Some(&candidates), "80");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert!(!stdout.contains("stopped:"), "{stdout}");
    let records = journal_records(&out);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["status"], "rejected");
    assert_eq!(records[1]["gate"], "prefilter");
    assert!(
        records[1]["note"].as_str().unwrap().contains("std::env"),
        "{records:?}"
    );
}

#[test]
fn minimize_exits_zero_when_any_candidate_is_proven() {
    let out = scratch("minimize");
    let nugget_dir = scratch("minimize-nugget");
    write_probe(&nugget_dir, "minimize", "println!(\"score: 4\");\n", None);
    let output = run_nugget(&out, &nugget_dir, None, "5");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout {stdout} stderr {stderr}");
    assert!(stdout.contains("proven"), "{stdout}");
    let records = journal_records(&out);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "proven");
    assert_eq!(records[0]["score"], 4.0);
    let region = fs::read_to_string(out.join("best.region.rs")).unwrap();
    assert_eq!(region, "println!(\"score: 4\");\n");
}

#[test]
fn tweak_on_one_literal_stops_after_fifty_repeats() {
    let out = scratch("tweak-stop");
    let nugget_dir = scratch("tweak-stop-nugget");
    // `std::fs` dies in the prefilter, so a repeat never reaches rustc.
    // The only literal is `1`, whose legal steps are 0, 2, and 3.
    write_probe(
        &nugget_dir,
        "pass_all",
        "    std::fs::read(\"a\");\n    let n = 1;\n",
        Some("ok\n"),
    );
    let started = Instant::now();
    let output = run(
        &out,
        &[
            "run",
            nugget_dir.to_str().unwrap(),
            "--tweak",
            "--budget",
            "2000",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let elapsed = started.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout {stdout} stderr {stderr}"
    );
    assert!(
        stdout.contains("stopped: 50 proposals in a row left no journal line"),
        "{stdout}"
    );
    let records = journal_records(&out);
    eprintln!(
        "tweak-stop journal_lines={} elapsed_ms={}",
        records.len(),
        elapsed.as_millis()
    );
    // Seed 1 reaches every legal step of `1` (0, 2, 3) before fifty repeats.
    assert_eq!(records.len(), 4, "{stdout}");
    assert_eq!(records[0]["proposer"], "seed");
    assert!(records[1..]
        .iter()
        .all(|record| record["proposer"] == "tweak"));
    for record in &records {
        assert_eq!(record["status"], "rejected");
        assert_eq!(record["gate"], "prefilter");
        assert_eq!(record["ms"]["build"], 0);
        assert_eq!(record["ms"]["run"], 0);
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "tweak stop spun: {elapsed:?}, journal lines {}",
        records.len()
    );
}

fn write_probe(dir: &std::path::Path, objective: &str, seed: &str, expected: Option<&str>) {
    fs::create_dir_all(dir.join("cases")).unwrap();
    fs::write(
        dir.join("nugget.json"),
        format!(r#"{{"name":"probe","objective":"{objective}"}}"#),
    )
    .unwrap();
    fs::write(
        dir.join("template.rs"),
        format!("fn main() {{\n// CRAB:BEGIN\n{seed}// CRAB:END\n}}\n"),
    )
    .unwrap();
    fs::write(dir.join("cases").join("only.in"), "").unwrap();
    if let Some(expected) = expected {
        fs::write(dir.join("cases").join("only.out"), expected).unwrap();
    }
}

fn run_nugget(
    out: &PathBuf,
    nugget_dir: &PathBuf,
    candidates: Option<&PathBuf>,
    budget: &str,
) -> std::process::Output {
    let mut args = vec![
        "run".to_string(),
        nugget_dir.to_str().unwrap().to_string(),
        "--budget".to_string(),
        budget.to_string(),
        "--out".to_string(),
        out.to_str().unwrap().to_string(),
    ];
    if let Some(candidates) = candidates {
        args.push("--candidates".to_string());
        args.push(candidates.to_str().unwrap().to_string());
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run(out, &borrowed)
}
