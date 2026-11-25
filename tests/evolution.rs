//! End-to-end tests for evolutionary code breeding (`--breed`), multi-species
//! `rustc` oracle promotion (`--species`), and experiment goal / tolerance
//! stopping (`--goal`, `--tolerance`, `--patience`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crab-cage-evo-{label}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn journal_records(out: &Path) -> Vec<serde_json::Value> {
    let text = fs::read_to_string(out.join("journal.jsonl")).unwrap();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn breed_evolves_candidate_to_goal_within_tolerance_and_stops_early() {
    let root = scratch("breed-goal");
    let nugget_dir = root.join("nugget");
    fs::create_dir_all(nugget_dir.join("cases")).unwrap();
    fs::write(
        nugget_dir.join("nugget.json"),
        r#"{"name":"tune-target","objective":"minimize","timeout_ms":2000,"max_memory":"64M"}"#,
    )
    .unwrap();
    // Optimal constant `k` is 32 (gives score 0). Seed starts at k = 8 (score 576).
    // With `--goal 0 --tolerance 4`, any proven `k` in 30..=34 (`(k - 32)^2 <= 4`)
    // reaches "close enough" and stops the experiment well before `--budget 60`.
    let template = "\
use std::io::{self, Read};

fn estimate(x: i64) -> i64 {
    // CRAB:BEGIN
    let k: i64 = 8;
    let err = x - k;
    err * err
    // CRAB:END
}

fn main() {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s).unwrap();
    let x: i64 = s.trim().parse().unwrap();
    println!(\"score: {}\", estimate(x));
}
";
    fs::write(nugget_dir.join("template.rs"), template).unwrap();
    fs::write(nugget_dir.join("cases").join("01.in"), "32\n").unwrap();

    let out = root.join("out");
    let output = Command::new(crab_exe())
        .env_remove("CRAB_LUST_BIN")
        .args([
            "run",
            nugget_dir.to_str().unwrap(),
            "--breed",
            "--seed",
            "3",
            "--goal",
            "0",
            "--tolerance",
            "4",
            "--budget",
            "60",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected exit 0 when goal+tolerance is reached\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("goal reached: proven score "), "{stdout}");

    let records = journal_records(&out);
    assert!(
        records.len() < 40,
        "expected early stop well before budget 60, took {} candidates:\n{stdout}",
        records.len()
    );
    let last_proven = records
        .iter()
        .filter(|r| r["status"] == "proven")
        .last()
        .expect("proven winner");
    let best_score = last_proven["score"].as_f64().unwrap();
    assert!(best_score <= 4.0, "best_score={best_score}");
    assert!(out.join("species").join("manifest.json").is_file());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn species_archive_promotes_best_of_species_and_specialists_to_rustc_oracle() {
    let root = scratch("species-oracle");
    let lust_src = root.join("standin_lust.rs");
    fs::write(
        &lust_src,
        r##"
use std::env;
use std::process::{self, Command};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("capabilities") if args.get(1).map(String::as_str) == Some("--json") => {
            println!("{}", r#"{"interface":{"version":"0.3.2","build":"test","commands":["check","build","capabilities"],"flags":{"build":["--json","--sandbox","--max-memory","--time-limit"],"capabilities":["--json","--summary","--sandbox"]}}}"#);
        }
        Some("build") => {
            let out = args.iter().position(|a| a == "-o").and_then(|i| args.get(i + 1)).cloned().unwrap();
            let file = args.iter().rev().find(|a| a.ends_with(".rs")).cloned().unwrap();
            let status = Command::new("rustc")
                .args(["--edition", "2021", "-C", "opt-level=2", "-C", "debug-assertions=on", "-C", "overflow-checks=on", "-o", &out, &file])
                .status()
                .unwrap();
            if !status.success() {
                process::exit(1);
            }
        }
        _ => process::exit(2),
    }
}
"##,
    )
    .unwrap();
    let lust_bin = root.join(if cfg!(windows) {
        "standin_lust.exe"
    } else {
        "standin_lust"
    });
    assert!(Command::new("rustc")
        .args(["--edition", "2021", "-o"])
        .arg(&lust_bin)
        .arg(&lust_src)
        .status()
        .unwrap()
        .success());

    let nugget_dir = root.join("nugget");
    fs::create_dir_all(nugget_dir.join("cases")).unwrap();
    fs::write(
        nugget_dir.join("nugget.json"),
        r#"{"name":"two-cases","objective":"minimize","timeout_ms":2000,"max_memory":"64M"}"#,
    )
    .unwrap();
    let template = "\
use std::io::{self, Read};

fn cost(case_id: i64) -> i64 {
    // CRAB:BEGIN
    if case_id == 1 { 50 } else { 50 }
    // CRAB:END
}

fn main() {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s).unwrap();
    let id: i64 = s.trim().parse().unwrap();
    println!(\"score: {}\", cost(id));
}
";
    fs::write(nugget_dir.join("template.rs"), template).unwrap();
    fs::write(nugget_dir.join("cases").join("a.in"), "1\n").unwrap();
    fs::write(nugget_dir.join("cases").join("b.in"), "2\n").unwrap();

    let cands = root.join("cands");
    fs::create_dir_all(&cands).unwrap();
    // 01: same species as seed (`if v0 == # { # } else { # }`), worse score (120 > 100)
    //     -> filtered by lust score, MUST NOT run rustc oracle (`scored`, not `proven`).
    fs::write(
        cands.join("01_same_species_worse.rs"),
        "    if case_id == 1 { 60 } else { 60 }\n",
    )
    .unwrap();
    // 02: distinct structural species (`match`), total score 110 (worse than seed's 100!),
    //     wins case `b` (20 vs 50) and fills species slot #2 -> MUST be proven by rustc!
    fs::write(
        cands.join("02_new_species_specialist.rs"),
        "    match case_id { 1 => 90, _ => 20 }\n",
    )
    .unwrap();
    // 03: same `match` species as 02, improved total score (95 < 110, AND new global best!)
    //     -> MUST be proven by rustc!
    fs::write(
        cands.join("03_match_species_improved.rs"),
        "    match case_id { 1 => 80, _ => 15 }\n",
    )
    .unwrap();
    // 04: third species (`while`), worse than both on every case (total 160: 80 + 80)
    //     with `--species 2` full -> MUST stay `scored` (no oracle call wasted!).
    fs::write(
        cands.join("04_third_species_dominated.rs"),
        "    let mut acc = 80;\n    while acc < 80 { acc += case_id; }\n    acc\n",
    )
    .unwrap();

    let out = root.join("out");
    let output = Command::new(crab_exe())
        .env_remove("CRAB_LUST_BIN")
        .args([
            "run",
            nugget_dir.to_str().unwrap(),
            "--candidates",
            cands.to_str().unwrap(),
            "--species",
            "2",
            "--lust-bin",
            lust_bin.to_str().unwrap(),
            "--no-lust-daemon",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("species proven=2 observed=3"), "{stdout}");

    let records = journal_records(&out);
    assert_eq!(records.len(), 5);
    assert_eq!(records[0]["status"], "proven"); // seed (100)
    assert_eq!(records[1]["status"], "scored"); // same species worse (120) -> skipped oracle!
    assert_eq!(records[2]["status"], "proven"); // new species #2 (110) -> proven!
    assert_eq!(records[3]["status"], "proven"); // species #2 improved (95) -> proven!
    assert_eq!(records[4]["status"], "scored"); // dominated 3rd species (160) -> skipped oracle!

    // Check `summary --json` includes both `species` and `proposers`.
    let sum_out = Command::new(crab_exe())
        .args(["summary", out.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(sum_out.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&sum_out.stdout).unwrap();
    assert_eq!(summary["species"]["count"], 2);
    assert_eq!(summary["species"]["champions"][0]["score"], 95.0);
    assert!(!summary["proposers"].as_array().unwrap().is_empty());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn init_and_inspect_support_goal_tolerance_and_unmet_goal_exits_nonzero() {
    let root = scratch("init-goal");
    let nug = root.join("min-goal");
    let init = Command::new(crab_exe())
        .args([
            "init",
            nug.to_str().unwrap(),
            "--objective",
            "minimize",
            "--goal",
            "5",
            "--tolerance",
            "0.5",
        ])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let inspect = Command::new(crab_exe())
        .args(["inspect", nug.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(inspect.status.success());
    let info: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(info["goal"], 5.0);
    assert_eq!(info["tolerance"], 0.5);

    // Scaffolded minimize seed scores 1^2 + 2^2 + 3^2 = 14 > 5.5, so budget 1 exits 1!
    let out = root.join("out");
    let ran = Command::new(crab_exe())
        .env_remove("CRAB_LUST_BIN")
        .args([
            "run",
            nug.to_str().unwrap(),
            "--budget",
            "1",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        ran.status.code(),
        Some(1),
        "unmet goal must exit 1: {}",
        String::from_utf8_lossy(&ran.stdout)
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn real_lust_warm_daemon_breeds_and_proves_species_against_rustc() {
    let Some(lust_bin) = std::env::var_os("CRAB_LUST_BIN").filter(|v| !v.is_empty()) else {
        eprintln!("skipped: CRAB_LUST_BIN not set");
        return;
    };
    let root = scratch("real-lust-breed");
    let nugget_dir = root.join("nugget");
    fs::create_dir_all(nugget_dir.join("cases")).unwrap();
    fs::write(
        nugget_dir.join("nugget.json"),
        r#"{"name":"lust-breed","objective":"minimize","goal":0.0,"tolerance":1.0,"timeout_ms":2000,"max_memory":"64M"}"#,
    )
    .unwrap();
    // Seed uses `<` and constant `12`; optimal is `16`. `lust` warm daemon
    // validates mutations in milliseconds, and `rustc` proves the winners.
    let template = "\
use std::io::{self, Read};

fn loss(target: i64) -> i64 {
    // CRAB:BEGIN
    let guess: i64 = 12;
    let d = target - guess;
    d * d
    // CRAB:END
}

fn main() {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s).unwrap();
    let t: i64 = s.trim().parse().unwrap();
    println!(\"score: {}\", loss(t));
}
";
    fs::write(nugget_dir.join("template.rs"), template).unwrap();
    fs::write(nugget_dir.join("cases").join("01.in"), "16\n").unwrap();

    let out = root.join("out");
    let output = Command::new(crab_exe())
        .env("CRAB_LUST_BIN", &lust_bin)
        .args([
            "run",
            nugget_dir.to_str().unwrap(),
            "--breed",
            "--species",
            "3",
            "--seed",
            "2",
            "--budget",
            "50",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "real lust breed failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("goal reached: proven score "), "{stdout}");
    assert!(out.join("best.rs").is_file());
    assert!(out.join("species").join("manifest.json").is_file());

    // Ensure the warm daemon socket was cleaned up on exit.
    let work = out.join("work");
    let lingering: Vec<_> = fs::read_dir(&work)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".lust-daemon-") && n.ends_with(".sock"))
        .collect();
    assert!(lingering.is_empty(), "lingering sockets: {lingering:?}");

    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = fs::remove_dir_all(&root);
}
