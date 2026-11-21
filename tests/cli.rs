use std::fs;
use std::path::PathBuf;
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

#[test]
fn cli_evolve_writes_the_controller_and_models_are_local() {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    let dir = std::env::temp_dir().join(format!("crab-cage-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = Command::new(exe.clone())
        .current_dir(&dir)
        .args([
            "evolve",
            "--backend",
            "mock",
            "--eval",
            "native",
            "--no-verify",
            "--generations",
            "6",
            "--population",
            "12",
            "--seed",
            "1",
            "--out-dir",
            "out",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("best fitness"), "{stdout}");
    let controller = fs::read_to_string(dir.join("out/best_controller.rs")).unwrap();
    assert!(controller.contains("fn control("));
    assert!(controller.contains("fn main()"));
    assert!(dir.join("out/report.md").is_file());
    assert!(dir.join("out/report.json").is_file());

    let models = Command::new(&exe).arg("models").output().unwrap();
    let text = String::from_utf8_lossy(&models.stdout);
    assert!(text.contains("Qwen2.5-Coder-14B"));
    assert!(text.contains("127.0.0.1"));
    assert!(text.contains("16 GB"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn command_line_flags_win_over_the_config_file() {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    let dir = std::env::temp_dir().join(format!("crab-cage-cli-merge-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("crab.toml");
    fs::write(&config, "seed = 7\ngenerations = 3\n").unwrap();
    let output = Command::new(&exe)
        .current_dir(&dir)
        .args([
            "evolve",
            "--seed",
            "1",
            "--eval",
            "native",
            "--out-dir",
            "out",
        ])
        .arg("--config")
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(dir.join("out/report.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(value["seed"], 1, "the flag must beat the file: {report}");
    assert_eq!(
        value["generations"], 3,
        "the file must beat the default: {report}"
    );

    let bad = dir.join("bad.toml");
    fs::write(&bad, "max_memory = \"lots\"\n").unwrap();
    let output = Command::new(&exe)
        .current_dir(&dir)
        .args([
            "evolve",
            "--eval",
            "native",
            "--no-verify",
            "--out-dir",
            "out2",
        ])
        .arg("--config")
        .arg(&bad)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a bad max_memory must be an error"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_inspect_init_summary_and_crab_lust_bin_env() {
    let exe = crab_exe();
    assert!(exe.is_file(), "missing binary {}", exe.display());
    let root = std::env::temp_dir().join(format!("crab-cage-cli-ux-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    // 1. `inspect --json` on an existing nugget gives a complete LLM briefing.
    let demo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("nuggets/sum-squares");
    let inspected = Command::new(&exe)
        .args(["inspect", demo.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&inspected.stdout).expect("inspect JSON");
    assert_eq!(info["name"], "sum-squares");
    assert_eq!(info["objective"], "pass_all");
    assert_eq!(info["cases_count"], 3);
    assert!(info["seed"].as_str().unwrap().contains("xs.iter().sum()"));
    assert!(info["template"].as_str().unwrap().contains("// CRAB:BEGIN"));
    assert_eq!(info["sample_case"]["id"], "negatives");

    // 2. `init` scaffolds a runnable nugget and refuses to overwrite.
    let scaffold = root.join("my-nugget");
    let init = Command::new(&exe)
        .args([
            "init",
            scaffold.to_str().unwrap(),
            "--objective",
            "pass_all",
        ])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let again = Command::new(&exe)
        .args(["init", scaffold.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!again.status.success(), "init must not overwrite");

    // 3. The scaffolded nugget runs cleanly and `summary --json` summarizes it.
    let out_dir = root.join("run-out");
    let ran = Command::new(&exe)
        .args([
            "run",
            scaffold.to_str().unwrap(),
            "--budget",
            "1",
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "scaffolded nugget seed should solve: stderr {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let summary = Command::new(&exe)
        .args(["summary", out_dir.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        summary.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&summary.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&summary.stdout).expect("summary JSON");
    assert_eq!(stats["candidates"], 1);
    assert_eq!(stats["status"]["proven"], 1);
    assert_eq!(stats["best"]["score"], 0.0);
    assert!(stats["best_region"]
        .as_str()
        .unwrap()
        .contains("xs.iter().sum()"));

    // 4. `lust-probe --json` reads CRAB_LUST_BIN without needing --lust-bin.
    let fake_src = root.join("fake_probe.rs");
    fs::write(
        &fake_src,
        r#"
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("lust 0.3.2-probe");
    } else {
        eprintln!("usage: lust build <file.rs> [--sandbox] [--max-memory <size>] [--time-limit <duration>]");
        std::process::exit(2);
    }
}
"#,
    )
    .unwrap();
    let fake_bin = root.join(if cfg!(windows) {
        "fake_probe.exe"
    } else {
        "fake_probe"
    });
    let compiled = Command::new("rustc")
        .args(["--edition", "2021", "-o"])
        .arg(&fake_bin)
        .arg(&fake_src)
        .status()
        .unwrap();
    assert!(compiled.success());
    let probed = Command::new(&exe)
        .env("CRAB_LUST_BIN", &fake_bin)
        .args(["lust-probe", "--json"])
        .output()
        .unwrap();
    assert!(
        probed.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&probed.stderr)
    );
    let probe_json: serde_json::Value =
        serde_json::from_slice(&probed.stdout).expect("lust-probe JSON");
    assert_eq!(probe_json["version"], "lust 0.3.2-probe");
    assert_eq!(probe_json["supports_max_memory"], true);
    assert_eq!(probe_json["supports_time_limit"], true);

    let _ = fs::remove_dir_all(&root);
}
