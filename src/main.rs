use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crab_cage::error::{err, Result};
use crab_cage::evolve::{execute, format_summary, BackendChoice, EvalMode, RunOptions};
use crab_cage::lust;
use crab_cage::models;
use crab_cage::nugget;
use crab_cage::pipeline::{self, Status};
use crab_cage::proposer::{FolderProposer, Proposer};
use crab_cage::run;
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
    /// Evaluate one candidate against a nugget.
    Eval(EvalArgs),
    /// Evaluate the seed and folder snippets, and journal each verdict.
    ///
    /// `pass_all` exits 0 only when a proven candidate scores 0.
    /// `minimize` exits 0 when any candidate is proven.
    Run(RunArgs),
}

#[derive(Parser, Clone, Debug, Default)]
struct EvolveArgs {
    /// Default 1.
    #[arg(long)]
    seed: Option<u64>,
    /// Default 16.
    #[arg(long)]
    population: Option<usize>,
    /// Default 12.
    #[arg(long)]
    generations: Option<usize>,
    /// `mock` (deterministic, no model server) or `local` (loopback HTTP). Default mock.
    #[arg(long)]
    backend: Option<String>,
    /// Default http://127.0.0.1:11434/v1.
    #[arg(long)]
    endpoint: Option<String>,
    /// Default qwen2.5-coder:14b.
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    lust_bin: Option<PathBuf>,
    /// `auto` (native search, sandboxed winner), `native`, or `sandbox`. Default auto.
    #[arg(long)]
    eval: Option<String>,
    /// Guest cap passed to `lust build --sandbox --max-memory` (lust 0.3.0). Default 128M.
    #[arg(long)]
    max_memory: Option<String>,
    /// Host backstop: RLIMIT_AS on Unix, job commit cap on Windows. Default 1G.
    #[arg(long)]
    host_memory: Option<String>,
    /// Default 5000.
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// Default crab-out.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Skip rustc / lust replay of the winner.
    #[arg(long)]
    no_verify: bool,
}

#[derive(Parser, Debug)]
struct EvalArgs {
    /// Directory with nugget.json, template.rs, and cases/.
    nugget_dir: PathBuf,
    /// Candidate file. `-` reads stdin.
    candidate: String,
    /// lust binary. Without it, rustc runs the fast gates.
    #[arg(long)]
    lust_bin: Option<PathBuf>,
    /// Directory for the spliced source and the built binary.
    #[arg(long)]
    work_dir: Option<PathBuf>,
    /// Print the verdict as one JSON line.
    #[arg(long)]
    json: bool,
    /// After a score, compile the same program with rustc and compare.
    #[arg(long)]
    oracle: bool,
    /// Use lust's warm daemon. Needs --lust-bin and --work-dir: the daemon
    /// is started in the work directory and stays warm for the next call
    /// that names the same one. It exits when idle, or when
    /// `.lust-daemon-*.sock` in that directory is deleted.
    #[arg(long)]
    lust_daemon: bool,
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Directory with nugget.json, template.rs, and cases/.
    nugget_dir: PathBuf,
    /// Directory of `*.rs` candidate snippets.
    #[arg(long)]
    candidates: Option<PathBuf>,
    /// Perturb one numeric literal of the best candidate on each proposal.
    /// Round-robin with `--candidates` when both are set.
    #[arg(long)]
    tweak: bool,
    /// Maximum number of candidates to evaluate, including the seed.
    #[arg(long, default_value_t = 200)]
    budget: usize,
    /// Journal, work files, and the best candidate.
    #[arg(long, default_value = "crab-out")]
    out: PathBuf,
    /// lust binary. Without it, rustc runs the fast gates.
    #[arg(long)]
    lust_bin: Option<PathBuf>,
    /// Start lust cold for every candidate. By default a run with
    /// --lust-bin keeps lust's daemon warm in `<out>/work` and stops it when
    /// the run ends.
    #[arg(long)]
    no_lust_daemon: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Models) => {
            print!("{}", models::recommended());
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::LustProbe { lust_bin }) => probe(lust_bin).map(|()| ExitCode::SUCCESS),
        Some(Command::Evolve(args)) => evolve(args).map(|()| ExitCode::SUCCESS),
        Some(Command::Eval(args)) => eval_cmd(args),
        Some(Command::Run(args)) => run_cmd(args),
        None => evolve(cli.evolve).map(|()| ExitCode::SUCCESS),
    };
    match result {
        Ok(code) => code,
        Err(problem) => {
            eprintln!("crab-cage: {}", problem.message);
            ExitCode::from(1)
        }
    }
}

fn evolve(args: EvolveArgs) -> Result<()> {
    let file = if let Some(path) = &args.config {
        Some(read_config(path)?)
    } else {
        let path = std::path::Path::new("crab-cage.toml");
        if path.is_file() {
            Some(read_config(path)?)
        } else {
            None
        }
    };
    let options = resolve_options(&args, file.as_ref())?;
    let output = execute(&options)?;
    print!("{}", format_summary(&output));
    Ok(())
}

fn parse_backend(value: &str) -> Result<BackendChoice> {
    match value {
        "mock" => Ok(BackendChoice::Mock),
        "local" | "openai" | "ollama" | "llama" => Ok(BackendChoice::Local),
        other => Err(err(format!(
            "unknown backend `{other}` (expected mock or local)"
        ))),
    }
}

fn parse_eval(value: &str) -> Result<EvalMode> {
    match value {
        "auto" => Ok(EvalMode::Auto),
        "native" => Ok(EvalMode::Native),
        "sandbox" => Ok(EvalMode::Sandbox),
        other => Err(err(format!(
            "unknown eval mode `{other}` (expected auto, native, or sandbox)"
        ))),
    }
}

/// Each option: the command-line flag, then the config file, then the
/// built-in default from `RunOptions::default()`.
fn resolve_options(args: &EvolveArgs, file: Option<&FileConfig>) -> Result<RunOptions> {
    let defaults = RunOptions::default();
    let backend = match args
        .backend
        .as_deref()
        .or(file.and_then(|f| f.backend.as_deref()))
    {
        Some(value) => parse_backend(value)?,
        None => defaults.backend,
    };
    let eval = match args
        .eval
        .as_deref()
        .or(file.and_then(|f| f.eval.as_deref()))
    {
        Some(value) => parse_eval(value)?,
        None => defaults.eval,
    };
    let max_memory = match args
        .max_memory
        .as_deref()
        .or(file.and_then(|f| f.max_memory.as_deref()))
    {
        Some(value) => supervisor::parse_memory(value)?,
        None => defaults.max_memory,
    };
    let host_memory = match args
        .host_memory
        .as_deref()
        .or(file.and_then(|f| f.host_memory.as_deref()))
    {
        Some(value) => supervisor::parse_memory(value)?,
        None => defaults.host_memory,
    };
    Ok(RunOptions {
        seed: args
            .seed
            .or(file.and_then(|f| f.seed))
            .unwrap_or(defaults.seed),
        population: args
            .population
            .or(file.and_then(|f| f.population))
            .unwrap_or(defaults.population),
        generations: args
            .generations
            .or(file.and_then(|f| f.generations))
            .unwrap_or(defaults.generations),
        backend,
        endpoint: args
            .endpoint
            .clone()
            .or(file.and_then(|f| f.endpoint.clone()))
            .unwrap_or(defaults.endpoint),
        model: args
            .model
            .clone()
            .or(file.and_then(|f| f.model.clone()))
            .unwrap_or(defaults.model),
        llm_timeout: Duration::from_secs(180),
        lust_bin: args
            .lust_bin
            .clone()
            .or(file.and_then(|f| f.lust_bin.clone())),
        eval,
        max_memory,
        host_memory,
        run_timeout: args
            .timeout_ms
            .or(file.and_then(|f| f.timeout_ms))
            .map(Duration::from_millis)
            .unwrap_or(defaults.run_timeout),
        compile_timeout: Duration::from_secs(180),
        out_dir: args
            .out_dir
            .clone()
            .or(file.and_then(|f| f.out_dir.clone()))
            .unwrap_or(defaults.out_dir),
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
    // A bad value in the file is an error even when a flag overrides it.
    if let Some(value) = &file.backend {
        parse_backend(value).map_err(|e| err(format!("{}: {}", path.display(), e.message)))?;
    }
    if let Some(value) = &file.eval {
        parse_eval(value).map_err(|e| err(format!("{}: {}", path.display(), e.message)))?;
    }
    for value in [&file.max_memory, &file.host_memory].into_iter().flatten() {
        supervisor::parse_memory(value)
            .map_err(|e| err(format!("{}: {}", path.display(), e.message)))?;
    }
    Ok(file)
}

fn run_cmd(args: RunArgs) -> Result<ExitCode> {
    let mut folder = args
        .candidates
        .as_ref()
        .map(|dir| FolderProposer::new(dir.as_path()));
    let mut proposers: Vec<&mut dyn Proposer> = Vec::new();
    if let Some(folder) = folder.as_mut() {
        proposers.push(folder);
    }
    let mut tweak = None;
    run::attach_tweak(args.tweak, &mut tweak, &mut proposers);
    let output = run::execute(
        &run::RunOptions {
            nugget_dir: args.nugget_dir,
            budget: args.budget,
            out_dir: args.out,
            lust_daemon: args.lust_bin.is_some() && !args.no_lust_daemon,
            lust_bin: args.lust_bin,
        },
        &mut proposers,
    )?;
    if output.solved {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn eval_cmd(args: EvalArgs) -> Result<ExitCode> {
    let nugget = nugget::load(&args.nugget_dir)?;
    let candidate = read_candidate(&args.candidate)?;
    let mut lust = match &args.lust_bin {
        Some(path) => Some(lust::probe(path, Duration::from_secs(10))?),
        None => None,
    };
    if args.lust_daemon {
        let Some(tool) = lust.as_mut() else {
            return Err(err("--lust-daemon needs --lust-bin"));
        };
        if args.work_dir.is_none() {
            return Err(err(
                "--lust-daemon needs --work-dir: the daemon is warm only for calls that share one",
            ));
        }
        tool.daemon = true;
    }
    let work_dir = match args.work_dir {
        Some(path) => path,
        None => fresh_work_dir(),
    };
    let verdict = pipeline::evaluate(
        &nugget,
        &candidate,
        &pipeline::Toolchain { lust },
        &pipeline::EvalOptions {
            work_dir,
            host_memory: 1024 * 1024 * 1024,
            compile_timeout: Duration::from_secs(180),
            oracle: args.oracle,
        },
    )?;
    if args.json {
        println!("{}", serde_json::to_string(&verdict)?);
    } else {
        print!("{}", pipeline::format_verdict(&verdict));
    }
    if verdict.status == Status::Scored || verdict.status == Status::Proven {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn read_candidate(path: &str) -> Result<String> {
    let text = if path == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        fs::read_to_string(path).map_err(|error| err(format!("{path}: {error}")))?
    };
    Ok(text.replace("\r\n", "\n"))
}

fn fresh_work_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("crab-cage-eval-{}-{nanos}", std::process::id()))
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
