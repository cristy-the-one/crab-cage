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
