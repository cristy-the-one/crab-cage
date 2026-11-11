//! Tests that talk to a real lust binary. They run only when
//! `CRAB_LUST_BIN` names one:
//! `CRAB_LUST_BIN=path/to/lust.exe cargo test --test real_lust`.
//! Set `LUST_NO_DAEMON=1` as well so lust leaves no background daemon.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crab_cage::evolve::{execute, EvalMode, RunOptions};
use crab_cage::lust;

fn real_lust() -> Option<PathBuf> {
    match std::env::var_os("CRAB_LUST_BIN") {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => {
            println!("skipped: CRAB_LUST_BIN not set");
            None
        }
    }
}

#[test]
fn real_lust_runs_the_default_evolution_and_checks_the_winner() {
    let Some(bin) = real_lust() else { return };
    let out_dir = std::env::temp_dir().join(format!("crab-cage-real-lust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    let options = RunOptions {
        lust_bin: Some(bin),
        eval: EvalMode::Auto,
        out_dir: out_dir.clone(),
        ..RunOptions::default()
    };
    let started = Instant::now();
    let output = execute(&options).expect("execute with a real lust");
    let elapsed = started.elapsed();
    assert!(
        output.report.verification.lust_check.starts_with("pass"),
        "lust check was: {}",
        output.report.verification.lust_check
    );
    assert!(
        elapsed < Duration::from_secs(120),
        "took {elapsed:?}, limit 120 s"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn real_lust_probe_returns_quickly() {
    let Some(bin) = real_lust() else { return };
    let started = Instant::now();
    let tool = lust::probe(&bin, Duration::from_secs(10)).expect("probe a real lust");
    let elapsed = started.elapsed();
    println!(
        "{} max_memory={} time_limit={}",
        tool.version, tool.supports_max_memory, tool.supports_time_limit
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "probe took {elapsed:?}, limit 15 s"
    );
}
