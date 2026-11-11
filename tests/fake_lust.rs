use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crab_cage::lust::{self, RunClass};
use crab_cage::supervisor::{self, Limits, StopKind};

fn fake_lust() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/probes/fake_lust.rs");
    let out_dir = root.join("target/probes");
    fs::create_dir_all(&out_dir).unwrap();
    let output = out_dir.join(if cfg!(windows) {
        "fake_lust.exe"
    } else {
        "fake_lust"
    });
    let source_newer = fs::metadata(&source)
        .and_then(|src| {
            fs::metadata(&output).map(|bin| {
                src.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    > bin.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
        })
        .unwrap_or(true);
    if !output.is_file() || source_newer {
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
        assert!(status.success());
    }
    output
}

#[test]
fn runner_passes_sandbox_and_max_memory_and_classifies_the_guest_stop() {
    let bin = fake_lust();
    let dir = std::env::temp_dir().join(format!("crab-cage-fake-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log.txt");
    let source = dir.join("candidate.rs");
    fs::write(&source, "fn main() { let _ = MODE_MEMORY; }\n").unwrap();

    let _ = log;
    let tool = lust::probe(&bin, Duration::from_secs(10)).unwrap();
    assert!(
        tool.supports_max_memory,
        "help text should advertise --max-memory"
    );
    assert!(
        tool.supports_time_limit,
        "help text should advertise --time-limit"
    );
    assert!(tool.version.contains("0.3.0"));

    // A backslash is legal in a Unix file name. On Windows every absolute
    // path already contains one. Either way the diagnostic must stay valid JSON.
    let broken_dir = if cfg!(windows) {
        dir.clone()
    } else {
        dir.join("win\\path")
    };
    fs::create_dir_all(&broken_dir).unwrap();
    let broken = broken_dir.join("broken.rs");
    fs::write(&broken, "fn main() { let _ = BROKEN_TOKEN; }\n").unwrap();
    let checked = lust::check(&tool, &broken, Duration::from_secs(10)).unwrap();
    assert!(!checked.ok);
    assert!(checked.diagnostics.iter().any(|d| d.code == "L0102"));
    lust::fix(&tool, &broken, Duration::from_secs(10)).unwrap();
    let fixed = fs::read_to_string(&broken).unwrap();
    assert!(!fixed.contains("BROKEN_TOKEN"));
    assert!(
        lust::check(&tool, &broken, Duration::from_secs(10))
            .unwrap()
            .ok
    );

    let built = dir.join(if cfg!(windows) { "guest.exe" } else { "guest" });
    lust::build(
        &tool,
        &source,
        &built,
        1024 * 1024,
        Some(Duration::from_millis(500)),
        Duration::from_secs(10),
    )
    .unwrap();
    let ran = supervisor::run_capture(
        Command::new(&built),
        &Limits {
            wall: Duration::from_secs(5),
            memory_bytes: Some(256 * 1024 * 1024),
            scrub_env: true,
        },
        b"",
    )
    .unwrap();
    assert_eq!(lust::classify(&ran), RunClass::MemoryLust);
    assert!(matches!(ran.kind, StopKind::Exited(101)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn evolve_runs_rustc_and_reports_a_lust_disagreement() {
    use crab_cage::evolve::{execute, BackendChoice, EvalMode, RunOptions};

    // The fake lust build echoes one fixed reply, so it cannot print the same
    // lines as the rustc build of the winner. The oracle comparison must
    // catch that instead of returning early after the lust build.
    let bin = fake_lust();
    let dir = std::env::temp_dir().join(format!("crab-cage-oracle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let result = execute(&RunOptions {
        seed: 1,
        population: 8,
        generations: 2,
        backend: BackendChoice::Mock,
        eval: EvalMode::Auto,
        verify: true,
        lust_bin: Some(bin),
        out_dir: dir.join("run"),
        run_timeout: Duration::from_secs(30),
        ..RunOptions::default()
    });
    let message = match result {
        Ok(_) => panic!("a stand-in lust must not agree with rustc"),
        Err(problem) => problem.to_string(),
    };
    assert!(
        message.contains("lust and rustc disagree; this is a lust bug"),
        "{message}"
    );
    let _ = fs::remove_dir_all(&dir);
}
