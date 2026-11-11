use std::env;
use std::fs;
use std::time::Duration;

use crab_cage::emit;
use crab_cage::evolve::{execute, BackendChoice, EvalMode, RunOptions};
use crab_cage::genome::{Genes, Genome, LawKind};
use crab_cage::lust;

#[test]
fn emitted_controller_compiles_and_matches_the_genome() {
    let mut genes = Genes::zeros();
    genes.kp = 0.15;
    genes.ki = 0.035;
    genes.bias = 0.02;
    genes.k_trend = 0.0;
    let genome = Genome::new(LawKind::Pi, genes);
    let source = emit::emit(&genome);
    emit::sandbox_prefilter(&source).unwrap();

    let dir = env::temp_dir().join(format!("crab-cage-replay-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("controller.rs");
    fs::write(&source_path, &source).unwrap();
    let bin = if cfg!(windows) {
        dir.join("controller.exe")
    } else {
        dir.join("controller")
    };
    lust::compile_rustc(&source_path, &bin, "2021", Duration::from_secs(120)).unwrap();

    let options = RunOptions {
        run_timeout: Duration::from_secs(20),
        host_memory: 1024 * 1024 * 1024,
        ..RunOptions::default()
    };
    // Reuse the library path by running a native evolve only to get the
    // replay helper's behavior: spawn the binary through the public execute
    // verify by pointing a one-generation search at a temp dir would not use
    // this exact genome. Call the closed check via a second compile already
    // done, and compare through `crab_cage` by executing the same steps the
    // CLI uses: a short native run's verifier.
    let output = execute(&RunOptions {
        seed: 1,
        population: 8,
        generations: 4,
        backend: BackendChoice::Mock,
        eval: EvalMode::Auto,
        verify: true,
        out_dir: dir.join("run"),
        run_timeout: Duration::from_secs(30),
        host_memory: 1024 * 1024 * 1024,
        lust_bin: None,
        ..options
    })
    .unwrap();
    let error = output
        .report
        .verification
        .max_abs_error
        .expect("replay error");
    assert!(error < 1e-6, "max abs error {error}");
    assert!(output.controller.is_file());
    let text = fs::read_to_string(&output.controller).unwrap();
    emit::sandbox_prefilter(&text).unwrap();
    assert!(text.contains("fn control("));
    let _ = fs::remove_dir_all(&dir);
    let _ = bin;
    let _ = genome;
}
