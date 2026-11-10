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
