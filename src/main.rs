use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crab_cage::error::{err, Result};
use crab_cage::evolve::{execute, format_summary, BackendChoice, EvalMode, RunOptions};
use crab_cage::lust;
use crab_cage::models;
use crab_cage::supervisor;

#[derive(Parser)]
#[command(
    name = "crab-cage",
    version,
    about = "Evolve Rust process controllers offline and run them in the lust sandbox."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    evolve: EvolveArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Evolve a controller for the built-in jacketed-tank demo.
    Evolve(EvolveArgs),
    /// Print local coding models that fit a 16 GB GPU.
    Models,
    /// Locate lust and report whether --max-memory is available.
    LustProbe {
        #[arg(long)]
        lust_bin: Option<PathBuf>,
    },
}

#[derive(Parser, Clone, Debug, Default)]
struct EvolveArgs {
    #[arg(long, default_value_t = 1)]
    seed: u64,
    #[arg(long, default_value_t = 16)]
    population: usize,
    #[arg(long, default_value_t = 12)]
    generations: usize,
    /// `mock` (deterministic, no model server) or `local` (loopback HTTP).
    #[arg(long, default_value = "mock")]
    backend: String,
    #[arg(long, default_value = "http://127.0.0.1:11434/v1")]
    endpoint: String,
    #[arg(long, default_value = "qwen2.5-coder:14b")]
    model: String,
    #[arg(long)]
    lust_bin: Option<PathBuf>,
    /// `auto` (native search, sandboxed winner), `native`, or `sandbox`.
    #[arg(long, default_value = "auto")]
    eval: String,
    /// Guest cap passed to `lust build --sandbox --max-memory` (lust 0.3.0).
    #[arg(long, default_value = "128M")]
    max_memory: String,
    /// Host backstop: RLIMIT_AS on Unix, job commit cap on Windows.
    #[arg(long, default_value = "1G")]
    host_memory: String,
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,
    #[arg(long, default_value = "crab-out")]
    out_dir: PathBuf,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Skip rustc / lust replay of the winner.
    #[arg(long)]
    no_verify: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Models) => {
            print!("{}", models::recommended());
            Ok(())
        }
        Some(Command::LustProbe { lust_bin }) => probe(lust_bin),
        Some(Command::Evolve(args)) => evolve(args),
        None => evolve(cli.evolve),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("crab-cage: {}", problem.message);
            ExitCode::from(1)
        }
    }
}

fn evolve(args: EvolveArgs) -> Result<()> {
    let mut options = options_from_args(&args)?;
    if let Some(path) = &args.config {
        let file = read_config(path)?;
        merge_unset(&mut options, &file, &args);
    } else {
        let path = std::path::Path::new("crab-cage.toml");
        if path.is_file() {
            let file = read_config(path)?;
            merge_unset(&mut options, &file, &args);
        }
    }
    let output = execute(&options)?;
    print!("{}", format_summary(&output));
    Ok(())
}

fn options_from_args(args: &EvolveArgs) -> Result<RunOptions> {
    let backend = match args.backend.as_str() {
        "mock" => BackendChoice::Mock,
        "local" | "openai" | "ollama" | "llama" => BackendChoice::Local,
        other => {
            return Err(err(format!(
                "unknown backend `{other}` (expected mock or local)"
            )))
        }
    };
    let eval = match args.eval.as_str() {
        "auto" => EvalMode::Auto,
        "native" => EvalMode::Native,
        "sandbox" => EvalMode::Sandbox,
        other => {
            return Err(err(format!(
                "unknown eval mode `{other}` (expected auto, native, or sandbox)"
            )))
        }
    };
    Ok(RunOptions {
        seed: args.seed,
        population: args.population,
        generations: args.generations,
        backend,
        endpoint: args.endpoint.clone(),
        model: args.model.clone(),
        llm_timeout: Duration::from_secs(180),
        lust_bin: args.lust_bin.clone(),
        eval,
        max_memory: supervisor::parse_memory(&args.max_memory)?,
        host_memory: supervisor::parse_memory(&args.host_memory)?,
        run_timeout: Duration::from_millis(args.timeout_ms),
        compile_timeout: Duration::from_secs(180),
        out_dir: args.out_dir.clone(),
        verify: !args.no_verify,
    })
}

struct FileConfig {
    lust_bin: Option<PathBuf>,
    backend: Option<String>,
    endpoint: Option<String>,
    model: Option<String>,
    max_memory: Option<String>,
    host_memory: Option<String>,
    timeout_ms: Option<u64>,
    population: Option<usize>,
    generations: Option<usize>,
    seed: Option<u64>,
    eval: Option<String>,
    out_dir: Option<PathBuf>,
}

fn read_config(path: &std::path::Path) -> Result<FileConfig> {
    let text = fs::read_to_string(path)?;
    let mut file = FileConfig {
        lust_bin: None,
        backend: None,
        endpoint: None,
        model: None,
        max_memory: None,
        host_memory: None,
        timeout_ms: None,
        population: None,
        generations: None,
        seed: None,
        eval: None,
        out_dir: None,
    };
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(err(format!(
                "{}:{}: expected key = value",
                path.display(),
                index + 1
            )));
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key {
            "lust_bin" => file.lust_bin = Some(PathBuf::from(value)),
            "backend" => file.backend = Some(value.to_string()),
            "endpoint" => file.endpoint = Some(value.to_string()),
            "model" => file.model = Some(value.to_string()),
            "max_memory" => file.max_memory = Some(value.to_string()),
            "host_memory" => file.host_memory = Some(value.to_string()),
            "timeout_ms" => {
                file.timeout_ms = Some(
                    value
                        .parse()
                        .map_err(|_| err(format!("{}: bad timeout_ms", path.display())))?,
                )
            }
            "population" => {
                file.population = Some(
                    value
                        .parse()
                        .map_err(|_| err(format!("{}: bad population", path.display())))?,
                )
            }
            "generations" => {
                file.generations = Some(
                    value
                        .parse()
                        .map_err(|_| err(format!("{}: bad generations", path.display())))?,
                )
            }
            "seed" => {
                file.seed = Some(
                    value
                        .parse()
                        .map_err(|_| err(format!("{}: bad seed", path.display())))?,
                )
            }
            "eval" => file.eval = Some(value.to_string()),
            "out_dir" => file.out_dir = Some(PathBuf::from(value)),
            other => return Err(err(format!("{}: unknown key `{other}`", path.display()))),
        }
    }
    Ok(file)
}

fn merge_unset(options: &mut RunOptions, file: &FileConfig, args: &EvolveArgs) {
    // Clap always has defaults, so a flag the user did not pass is
    // indistinguishable from the default. The config file overrides those
    // defaults, and a non-default flag wins because we only replace a field
    // when the parsed args still hold the default.
    if args.seed == 1 {
        if let Some(seed) = file.seed {
            options.seed = seed;
        }
    }
    if args.population == 16 {
        if let Some(population) = file.population {
            options.population = population;
        }
    }
    if args.generations == 12 {
        if let Some(generations) = file.generations {
            options.generations = generations;
        }
    }
    if args.backend == "mock" {
        if let Some(backend) = &file.backend {
            if backend == "local" {
                options.backend = BackendChoice::Local;
            }
        }
    }
    if args.endpoint == "http://127.0.0.1:11434/v1" {
        if let Some(endpoint) = &file.endpoint {
            options.endpoint = endpoint.clone();
        }
    }
    if args.model == "qwen2.5-coder:14b" {
        if let Some(model) = &file.model {
            options.model = model.clone();
        }
    }
    if args.lust_bin.is_none() {
        if let Some(bin) = &file.lust_bin {
            options.lust_bin = Some(bin.clone());
        }
    }
    if args.max_memory == "128M" {
        if let Some(value) = &file.max_memory {
            if let Ok(bytes) = supervisor::parse_memory(value) {
                options.max_memory = bytes;
            }
        }
    }
    if args.host_memory == "1G" {
        if let Some(value) = &file.host_memory {
            if let Ok(bytes) = supervisor::parse_memory(value) {
                options.host_memory = bytes;
            }
        }
    }
    if args.timeout_ms == 5000 {
        if let Some(timeout) = file.timeout_ms {
            options.run_timeout = Duration::from_millis(timeout);
        }
    }
    if args.eval == "auto" {
        if let Some(eval) = &file.eval {
            options.eval = match eval.as_str() {
                "native" => EvalMode::Native,
                "sandbox" => EvalMode::Sandbox,
                _ => EvalMode::Auto,
            };
        }
    }
    if args.out_dir == PathBuf::from("crab-out") {
        if let Some(dir) = &file.out_dir {
            options.out_dir = dir.clone();
        }
    }
}

fn probe(lust_bin: Option<PathBuf>) -> Result<()> {
    let path = match lust_bin {
        Some(path) => path,
        None => lust::which("lust").ok_or_else(|| {
            err("lust is not on PATH. Pass --lust-bin, or set lust_bin in crab-cage.toml. A 0.2.0 build has no --max-memory; that flag is in lust-lang v0.3.0 / main.")
        })?,
    };
    let tool = lust::probe(&path, Duration::from_secs(10))?;
    println!("{}", tool.version);
    println!("path: {}", tool.bin.display());
    if tool.supports_max_memory {
        println!("--max-memory: yes");
    } else {
        println!("--max-memory: no (host memory backstop still applies)");
    }
    if tool.supports_time_limit {
        println!("--time-limit: yes");
    } else {
        println!("--time-limit: no (host wall clock still applies)");
    }
    Ok(())
}
