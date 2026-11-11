//! Host wall-clock and memory backstops. These do not need a lust binary:
//! lust does not limit time, and the host cap is the backstop for the guest
//! allocator check.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use crab_cage::lust::{self, RunClass};
use crab_cage::supervisor::{self, Limits, StopKind};

fn compile_probe(name: &str) -> PathBuf {
    static LOCK: OnceLock<()> = OnceLock::new();
    let _ = LOCK.get_or_init(|| ());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/probes").join(format!("{name}.rs"));
    let out_dir = root.join("target").join("probes");
    fs::create_dir_all(&out_dir).unwrap();
    let output = out_dir.join(bin_name(name));
    if !output.is_file() {
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
            .expect("rustc");
        assert!(status.success(), "rustc {name} failed");
    }
    output
}

fn bin_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

#[test]
fn host_kills_a_candidate_that_overruns_the_wall_clock() {
    let sleeper = compile_probe("sleeper");
    let cmd = Command::new(&sleeper);
    let result = supervisor::run_capture(
        cmd,
        &Limits {
            wall: Duration::from_millis(400),
            memory_bytes: None,
            scrub_env: true,
        },
        b"",
    )
    .unwrap();
    assert_eq!(result.kind, StopKind::TimedOut);
    assert!(
        result.elapsed < Duration::from_secs(8),
        "kill took {:?}",
        result.elapsed
    );
    assert_eq!(lust::classify(&result), RunClass::TimedOut);
}

#[test]
fn run_capture_returns_when_a_grandchild_keeps_the_pipes() {
    let lingerer = compile_probe("lingerer");
    let started = std::time::Instant::now();
    let result = supervisor::run_capture(
        Command::new(&lingerer),
        &Limits {
            wall: Duration::from_secs(2),
            memory_bytes: None,
            scrub_env: false,
        },
        b"",
    )
    .unwrap();
    let took = started.elapsed();
    // Best effort: do not leave the 30 s grandchild behind.
    if let Some(pid) = result
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("child "))
    {
        let pid = pid.trim();
        let _ = if cfg!(windows) {
            Command::new("taskkill").args(["/F", "/PID", pid]).output()
        } else {
            Command::new("kill").args(["-9", pid]).output()
        };
    }
    eprintln!("lingerer: run_capture took {took:?}");
    assert!(took < Duration::from_secs(6), "run_capture took {took:?}");
    assert_eq!(result.kind, StopKind::Exited(0), "stderr={}", result.stderr);
    assert!(
        result.stdout.contains("parent done"),
        "stdout={:?}",
        result.stdout
    );
}

#[test]
fn run_capture_keeps_all_output_of_a_normal_exit() {
    let talker = compile_probe("talker");
    let result = supervisor::run_capture(
        Command::new(&talker),
        &Limits {
            wall: Duration::from_secs(20),
            memory_bytes: None,
            scrub_env: false,
        },
        b"",
    )
    .unwrap();
    assert_eq!(result.kind, StopKind::Exited(0));
    assert_eq!(result.stdout.len(), 400_000);
    assert_eq!(result.stderr.len(), 100_000);
    for (i, line) in result.stdout.lines().enumerate() {
        assert_eq!(
            line,
            format!("{i:08} {}", "x".repeat(90)),
            "stdout line {i}"
        );
    }
    for (i, line) in result.stderr.lines().enumerate() {
        assert_eq!(
            line,
            format!("{i:08} {}", "e".repeat(90)),
            "stderr line {i}"
        );
    }
}

#[test]
fn host_memory_cap_stops_a_large_allocation_and_allows_a_small_one() {
    let allocator = compile_probe("allocator");

    let mut tight = Command::new(&allocator);
    tight.env("PROBE_MB", "160");
    let denied = supervisor::run_capture(
        tight,
        &Limits {
            wall: Duration::from_secs(20),
            memory_bytes: Some(64 * 1024 * 1024),
            scrub_env: false,
        },
        b"x",
    )
    .unwrap();
    assert!(
        !denied.success(),
        "a 160 MiB allocation survived a 64 MiB cap: {:?} stdout={} stderr={}",
        denied.kind,
        denied.stdout,
        denied.stderr
    );

    let mut wide = Command::new(&allocator);
    wide.env("PROBE_MB", "32");
    let allowed = supervisor::run_capture(
        wide,
        &Limits {
            wall: Duration::from_secs(20),
            memory_bytes: Some(1024 * 1024 * 1024),
            scrub_env: false,
        },
        b"x",
    )
    .unwrap();
    assert!(
        allowed.success(),
        "a 32 MiB allocation died under a 1 GiB cap: {:?} stderr={}",
        allowed.kind,
        allowed.stderr
    );
    assert!(allowed.stdout.contains("32"));
}
