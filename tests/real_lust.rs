//! Tests that talk to a real lust binary. They run only when
//! `CRAB_LUST_BIN` names one:
//! `CRAB_LUST_BIN=path/to/lust.exe cargo test --test real_lust`.
//! The first two tests never start lust's daemon. The daemon test does, and
//! stops it at the end; with `LUST_NO_DAEMON=1` set it still passes, cold.

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
        "{} max_memory={} time_limit={} build_json={} sandbox_summary={} fix_sandbox={} opt={}",
        tool.version,
        tool.supports_max_memory,
        tool.supports_time_limit,
        tool.supports_build_json,
        tool.supports_sandbox_summary,
        tool.supports_fix_sandbox,
        tool.supports_opt
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "probe took {elapsed:?}, limit 15 s"
    );
    assert!(tool.supports_max_memory, "expected --max-memory on lust");
    assert!(tool.supports_time_limit, "expected --time-limit on lust");
    assert!(
        tool.supports_build_json,
        "expected build --json on lust 0.3.1+ / 0.3.2"
    );
    assert!(
        tool.supports_sandbox_summary,
        "expected capabilities --sandbox --summary on lust 0.3.1+ / 0.3.2"
    );
    assert!(
        tool.supports_fix_sandbox,
        "expected fix --sandbox on lust 0.3.1+ / 0.3.2"
    );
    let summary = lust::capabilities_summary(&tool);
    assert!(
        summary.contains("--sandbox"),
        "expected sandbox capability summary, got: {summary}"
    );
}

/// lust 0.3.2 accepts `f64::clamp` with non-literal bounds (lust-lang #44).
#[test]
fn real_lust_032_supports_float_clamp_with_variable_bounds() {
    use crab_cage::nugget::{Case, Nugget, Objective};
    use crab_cage::pipeline::{evaluate, EvalOptions, Status, Toolchain};

    let Some(bin) = real_lust() else { return };
    let tool = lust::probe(&bin, Duration::from_secs(15)).expect("probe");
    let work = std::env::temp_dir().join(format!("crab-cage-real-clamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let nugget = Nugget {
        name: "clamp-vars".to_string(),
        objective: Objective::PassAll,
        timeout: Duration::from_secs(2),
        max_memory: 64 * 1024 * 1024,
        prefix: "fn clamp_val(x: f64, lo: f64, hi: f64) -> f64 {\n// CRAB:BEGIN\n".to_string(),
        seed: "    x\n".to_string(),
        suffix: "// CRAB:END\n}\nfn main() {\n    println!(\"{} {} {}\", clamp_val(-2.0, -1.0, 3.0), clamp_val(1.5, -1.0, 3.0), clamp_val(9.0, -1.0, 3.0));\n}\n".to_string(),
        region_first_line: 3,
        cases: vec![Case {
            id: "one".to_string(),
            input: String::new(),
            expected: Some("-1 1.5 3\n".to_string()),
        }],
    };
    let verdict = evaluate(
        &nugget,
        "    x.clamp(lo, hi)\n",
        &Toolchain { lust: Some(tool) },
        &EvalOptions {
            work_dir: work.clone(),
            host_memory: 1024 * 1024 * 1024,
            compile_timeout: Duration::from_secs(60),
            oracle: true,
            slot: 0,
        },
    )
    .expect("evaluate clamp with variable bounds");
    assert_eq!(verdict.status, Status::Proven, "{verdict:?}");
    assert_eq!(verdict.score, Some(0.0));
    let _ = std::fs::remove_dir_all(&work);
}

/// With the daemon on, every candidate is compiled as `slot.rs` in the work
/// directory, lust runs there, and the verdicts are the same as without it.
/// Run without `LUST_NO_DAEMON` in the environment, or lust ignores the daemon.
#[test]
fn real_lust_daemon_gives_the_same_verdicts_from_one_slot_file() {
    use crab_cage::nugget;
    use crab_cage::pipeline::{evaluate, EvalOptions, Gate, Status, Toolchain, SLOT_FILE};

    let Some(bin) = real_lust() else { return };
    let demo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("nuggets/sum-squares");
    let nugget = nugget::load(&demo).expect("load the demo nugget");
    let work = std::env::temp_dir().join(format!("crab-cage-real-daemon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let mut tool = lust::probe(&bin, Duration::from_secs(15)).expect("probe");
    tool.daemon = true;
    let tools = Toolchain { lust: Some(tool) };
    let opts = EvalOptions {
        work_dir: work.clone(),
        host_memory: 1024 * 1024 * 1024,
        compile_timeout: Duration::from_secs(120),
        oracle: false,
        slot: 0,
    };

    let good = evaluate(
        &nugget,
        "    xs.iter().map(|x| x * x).sum()
",
        &tools,
        &opts,
    )
    .unwrap();
    assert_eq!(good.status, Status::Scored, "{good:?}");
    assert_eq!(good.score, Some(0.0));
    let wrong = evaluate(
        &nugget,
        "    xs.iter().sum()
",
        &tools,
        &opts,
    )
    .unwrap();
    assert_eq!(wrong.status, Status::Scored, "{wrong:?}");
    assert_eq!(wrong.score, Some(3.0));
    let broken = evaluate(
        &nugget,
        "    let v: i64 = \"no\";
    v
",
        &tools,
        &opts,
    )
    .unwrap();
    assert_eq!(broken.gate, Gate::Check, "{broken:?}");
    assert_eq!(broken.diagnostics[0].line, 1);
    // A second good candidate after a rejected one: the slot must not keep
    // the rejected text.
    let again = evaluate(
        &nugget,
        "    xs.iter().map(|x| x * x).sum::<i64>()
",
        &tools,
        &opts,
    )
    .unwrap();
    assert_eq!(again.score, Some(0.0), "{again:?}");

    assert!(work.join(SLOT_FILE).is_file());
    assert!(work.join(format!("{}.rs", good.hash)).is_file());
    println!(
        "daemon ms: first check {} build {}, fourth check {} build {}",
        good.ms.check, good.ms.build, again.ms.check, again.ms.build
    );
    lust::stop_daemon(&work);
    std::thread::sleep(Duration::from_millis(500));
    let _ = std::fs::remove_dir_all(&work);
}

/// `crab-cage run` with the daemon: candidates go through `slot.rs`, the
/// right one is proven, and no daemon socket is left when the run ends.
#[test]
fn real_lust_run_uses_the_daemon_and_stops_it() {
    use crab_cage::pipeline::SLOT_FILE;
    use crab_cage::proposer::{FolderProposer, Proposer};
    use crab_cage::run;

    let Some(bin) = real_lust() else { return };
    let root = std::env::temp_dir().join(format!("crab-cage-real-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let candidates = root.join("candidates");
    std::fs::create_dir_all(&candidates).unwrap();
    std::fs::write(
        candidates.join("a_wrong.rs"),
        "    49
",
    )
    .unwrap();
    std::fs::write(
        candidates.join("b_broken.rs"),
        "    let v: i64 = \"no\";
    v
",
    )
    .unwrap();
    std::fs::write(
        candidates.join("c_right.rs"),
        "    xs.iter().map(|x| x * x).sum()
",
    )
    .unwrap();
    let out = root.join("out");
    let mut folder = FolderProposer::new(candidates.as_path());
    let mut proposers: Vec<&mut dyn Proposer> = vec![&mut folder];
    let output = run::execute(
        &run::RunOptions {
            nugget_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("nuggets/sum-squares"),
            budget: 20,
            // Three jobs: the three candidates go through three slot files
            // at the same time, served by one daemon.
            jobs: 3,
            out_dir: out.clone(),
            lust_bin: Some(bin),
            lust_daemon: true,
            coverage: false,
        },
        &mut proposers,
    )
    .expect("run with a real lust");
    assert!(output.solved);
    let work = out.join("work");
    assert!(work.join(SLOT_FILE).is_file());
    assert!(work.join(crab_cage::pipeline::slot_file(1)).is_file());
    assert!(work.join(crab_cage::pipeline::slot_file(2)).is_file());
    let region = std::fs::read_to_string(out.join("best.region.rs")).unwrap();
    assert!(region.contains("x * x"), "{region}");
    let sockets: Vec<_> = std::fs::read_dir(&work)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".lust-daemon-") && name.ends_with(".sock"))
        .collect();
    assert!(sockets.is_empty(), "daemon socket left behind: {sockets:?}");
    std::thread::sleep(Duration::from_millis(500));
    let _ = std::fs::remove_dir_all(&root);
}
