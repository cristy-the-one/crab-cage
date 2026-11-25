use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crab_cage::error::{err, Result};
use crab_cage::evolve::{execute, format_summary, BackendChoice, EvalMode, RunOptions};
use crab_cage::journal::Journal;
use crab_cage::lust;
use crab_cage::models;
use crab_cage::nugget::{self, Objective};
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
    /// Locate lust and report its version and supported flags.
    LustProbe {
        /// lust binary (defaults to $CRAB_LUST_BIN or `lust` on PATH).
        #[arg(long)]
        lust_bin: Option<PathBuf>,
        /// Print probe results as one JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Inspect a nugget (metadata, objective, seed, template, sample case).
    Inspect(InspectArgs),
    /// Summarize a run's journal.jsonl (counts, top L-codes, best score, stage timings).
    Summary(SummaryArgs),
    /// Scaffold a new runnable nugget directory.
    Init(InitArgs),
    /// Evaluate one candidate against a nugget.
    Eval(EvalArgs),
    /// Evaluate the seed and folder snippets, and journal each verdict.
    ///
    /// `pass_all` exits 0 only when a proven candidate scores 0.
    /// `minimize` exits 0 when any candidate is proven.
    Run(RunArgs),
    /// Firmware delta kernels: the v0.03 oracle, the corpus, and a search.
    Delta(DeltaArgs),
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
struct InspectArgs {
    /// Directory with nugget.json, template.rs, and cases/.
    nugget_dir: PathBuf,
    /// Print the nugget briefing as one JSON object (ideal for LLM agents).
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Debug)]
struct SummaryArgs {
    /// Run output directory containing `journal.jsonl`, or path to a journal file.
    path: PathBuf,
    /// Print counts, top L-codes, best score, and stage timings as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Debug)]
struct InitArgs {
    /// Directory to create the nugget in.
    dir: PathBuf,
    /// Nugget name (defaults to the directory name).
    #[arg(long)]
    name: Option<String>,
    /// Objective: `pass_all`, `minimize`, or `fastest`.
    #[arg(long, default_value = "pass_all")]
    objective: String,
    /// Optional target score written to `nugget.json`.
    #[arg(long)]
    goal: Option<f64>,
    /// Optional "close enough" margin written to `nugget.json`.
    #[arg(long)]
    tolerance: Option<f64>,
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
    /// Evolutionary code and parameter breeder: UCB species selection,
    /// directional line search, same-species differential evolution / crossover,
    /// and sandbox-safe code micro-mutations. Defaults `--species` to 4.
    #[arg(long, visible_alias = "mutate")]
    breed: bool,
    /// Fast `--breed` candidates proposed per round-robin turn (defaults to 4
    /// when paired with `--backend local`, 1 otherwise).
    #[arg(long)]
    breed_burst: Option<usize>,
    /// Number of distinct proven code species champions to maintain and validate
    /// against `rustc` (defaults to 4 with `--breed`, 1 otherwise).
    #[arg(long)]
    species: Option<usize>,
    /// Target score at which the experiment succeeds and stops early.
    #[arg(long, allow_hyphen_values = true)]
    goal: Option<f64>,
    /// Additive "close enough" margin: stops when a proven candidate reaches
    /// `score <= goal + tolerance`.
    #[arg(long)]
    tolerance: Option<f64>,
    /// Stop early after this many evaluated candidates without improving the
    /// global best or advancing the species archive.
    #[arg(long)]
    patience: Option<usize>,
    /// `local` asks a loopback model for each region.
    #[arg(long)]
    backend: Option<String>,
    /// Loopback model URL. Default http://127.0.0.1:11434/v1.
    #[arg(long)]
    endpoint: Option<String>,
    /// Model name sent to that URL. Default qwen2.5-coder:14b.
    #[arg(long)]
    model: Option<String>,
    /// Maximum number of candidates to evaluate, including the seed.
    #[arg(long, default_value_t = 200)]
    budget: usize,
    /// Measure lust against rustc: ask rustc about every candidate lust
    /// refuses to compile, and compare every candidate that runs with its
    /// rustc build, not only a new best. A program rustc accepts and lust
    /// refuses is copied to `<out>/lust-only/`. Slower: one rustc run more
    /// for most candidates. Needs --lust-bin.
    #[arg(long)]
    coverage: bool,
    /// With `--backend local`, a candidate that does not compile is given
    /// to rustc, and the fixes rustc marks as safe to apply are applied and
    /// tried as a candidate named `repair`. This flag turns that off.
    #[arg(long)]
    no_repair: bool,
    /// Makes a run repeatable. `--tweak` draws its changes from it
    /// (default 1). With `--backend local`, each request to the model
    /// carries a seed counted up from it; without the flag the model
    /// server picks its own and two runs differ.
    #[arg(long)]
    seed: Option<u64>,
    /// Candidates evaluated at the same time. Each one gets its own slot
    /// file; the journal keeps the order the candidates were proposed in.
    #[arg(long, default_value_t = 1)]
    jobs: usize,
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

#[derive(Parser, Debug)]
struct DeltaArgs {
    #[command(subcommand)]
    command: DeltaCommand,
}

#[derive(Subcommand, Debug)]
enum DeltaCommand {
    /// Write `nuggets/delta-kernel/cases` from every `old.srec`/`new.srec` pair.
    SyncCases {
        #[arg(long, default_value = "corpus/pairs")]
        corpus: PathBuf,
        #[arg(long, default_value = "nuggets/delta-kernel")]
        nugget: PathBuf,
    },
    /// Score the host encoders and print JSON.
    Measure {
        #[arg(long, default_value = "corpus/pairs")]
        corpus: PathBuf,
    },
    /// Write the synthetic SREC pairs into the corpus directory.
    GenSynthetic {
        #[arg(long, default_value = "corpus/pairs")]
        corpus: PathBuf,
    },
    /// Run one kernel through the cage, then the reference decoder.
    Eval {
        #[arg(long, default_value = "nuggets/delta-kernel")]
        nugget: PathBuf,
        /// lust binary (defaults to $CRAB_LUST_BIN; without it, rustc runs the fast gates).
        #[arg(long)]
        lust_bin: Option<PathBuf>,
        /// Candidate file. The seed is `nuggets/delta-kernel/template.rs`'s region;
        /// pass a file of replacement lines, as with `crab-cage eval`.
        candidate: PathBuf,
    },
    /// Search the greedy parameter grid and print the winner.
    Search {
        #[arg(long, default_value = "corpus/pairs")]
        corpus: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Models) => {
            print!("{}", models::recommended());
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::LustProbe { lust_bin, json }) => {
            probe(lust_bin, json).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Inspect(args)) => inspect_cmd(args).map(|()| ExitCode::SUCCESS),
        Some(Command::Summary(args)) => summary_cmd(args).map(|()| ExitCode::SUCCESS),
        Some(Command::Init(args)) => init_cmd(args).map(|()| ExitCode::SUCCESS),
        Some(Command::Evolve(args)) => evolve(args).map(|()| ExitCode::SUCCESS),
        Some(Command::Eval(args)) => eval_cmd(args),
        Some(Command::Run(args)) => run_cmd(args),
        Some(Command::Delta(args)) => delta_cmd(args).map(|()| ExitCode::SUCCESS),
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
        lust_bin: resolve_lust_bin(
            args.lust_bin
                .clone()
                .or(file.and_then(|f| f.lust_bin.clone())),
        ),
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

fn delta_cmd(args: DeltaArgs) -> Result<()> {
    match args.command {
        DeltaCommand::SyncCases { corpus, nugget } => {
            crab_cage::delta::sync_cases(&corpus, &nugget)?;
            println!("wrote cases in {}", nugget.display());
        }
        DeltaCommand::GenSynthetic { corpus } => {
            crab_cage::delta::write_synthetic(&corpus)?;
            println!("wrote synthetic pairs in {}", corpus.display());
        }
        DeltaCommand::Measure { corpus } => {
            let pairs = crab_cage::delta::load_pairs(&corpus)?;
            let measured = crab_cage::delta::measure(&pairs)?;
            println!("{}", serde_json::to_string_pretty(&measured)?);
        }
        DeltaCommand::Search { corpus } => {
            let pairs = crab_cage::delta::load_pairs(&corpus)?;
            let images: Vec<_> = pairs
                .iter()
                .map(|pair| (pair.old.clone(), pair.new.clone()))
                .collect();
            let (params, total) = crab_cage::delta::search_family(&images)?;
            println!("bytes {total}");
            println!("{params:?}");
        }
        DeltaCommand::Eval {
            nugget,
            lust_bin,
            candidate,
        } => {
            let lust_bin = resolve_lust_bin(lust_bin);
            let report = crab_cage::delta::eval_candidate_with_lust(
                &nugget,
                &candidate,
                lust_bin.as_deref(),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.ok {
                return Err(crab_cage::error::err(if report.note.is_empty() {
                    "kernel has no score".to_string()
                } else {
                    report.note
                }));
            }
        }
    }
    Ok(())
}

fn run_cmd(args: RunArgs) -> Result<ExitCode> {
    let lust_bin = resolve_lust_bin(args.lust_bin);
    let species_cap = args
        .species
        .unwrap_or(if args.breed { 4 } else { 1 })
        .max(1);
    let breed_burst = args
        .breed_burst
        .unwrap_or(if args.breed && args.backend.is_some() {
            4
        } else {
            1
        });
    let archive = crab_cage::species::SpeciesArchive::new_handle(species_cap);
    let archive_ref = (species_cap > 1 || args.breed).then_some(&archive);

    let mut folder = args
        .candidates
        .as_ref()
        .map(|dir| FolderProposer::new(dir.as_path()));
    let mut proposers: Vec<&mut dyn Proposer> = Vec::new();
    if let Some(folder) = folder.as_mut() {
        proposers.push(folder);
    }
    let mut tweak = None;
    run::attach_tweak(args.tweak, args.seed, &mut tweak, &mut proposers);
    // Shared by the repair proposer, which writes it, and the model
    // proposer, which reads what became of its own regions.
    let repairs = crab_cage::repair::RepairLog::default();
    let mut model = None;
    run::attach_model_with_species(
        args.backend.as_deref(),
        args.endpoint.as_deref(),
        args.model.as_deref(),
        lust_bin.as_deref(),
        args.seed,
        Some(&args.out.join("prompts")),
        Some(&repairs),
        archive_ref,
        &mut model,
        &mut proposers,
    )?;
    let mut repair = None;
    run::attach_repair_with_lust(
        args.backend.is_some() && !args.no_repair,
        lust_bin.as_deref(),
        &repairs,
        &mut repair,
        &mut proposers,
    );
    let mut breed = None;
    run::attach_breed(
        args.breed,
        args.seed,
        Some(&archive),
        &mut breed,
        &mut proposers,
    );
    let output = run::execute_evolved(
        &run::RunOptions {
            nugget_dir: args.nugget_dir,
            budget: args.budget,
            jobs: args.jobs,
            out_dir: args.out,
            lust_daemon: lust_bin.is_some() && !args.no_lust_daemon,
            coverage: args.coverage,
            lust_bin,
        },
        &run::EvolutionOptions {
            species: species_cap,
            goal: args.goal,
            tolerance: args.tolerance,
            patience: args.patience,
            archive: Some(archive),
            breed_burst,
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
    let lust_bin = resolve_lust_bin(args.lust_bin);
    let mut lust = match &lust_bin {
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
            slot: 0,
        },
    )?;
    if args.json {
        crab_cage::console::line(&serde_json::to_string(&verdict)?);
    } else {
        crab_cage::console::print(&pipeline::format_verdict(&verdict));
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

fn resolve_lust_bin(flag: Option<PathBuf>) -> Option<PathBuf> {
    if flag.is_some() {
        return flag;
    }
    std::env::var_os("CRAB_LUST_BIN")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn probe(lust_bin: Option<PathBuf>, json: bool) -> Result<()> {
    let path = match resolve_lust_bin(lust_bin) {
        Some(path) => path,
        None => lust::which("lust").ok_or_else(|| {
            err("lust is not on PATH. Pass --lust-bin, set CRAB_LUST_BIN, or set lust_bin in crab-cage.toml.")
        })?,
    };
    let tool = lust::probe(&path, Duration::from_secs(10))?;
    if json {
        let payload = serde_json::json!({
            "version": tool.version,
            "build": tool.build,
            "path": tool.bin.display().to_string(),
            "supports_max_memory": tool.supports_max_memory,
            "supports_time_limit": tool.supports_time_limit,
            "supports_build_json": tool.supports_build_json,
            "supports_fix_sandbox": tool.supports_fix_sandbox,
            "supports_sandbox_summary": tool.supports_sandbox_summary,
            "supports_opt": tool.supports_opt,
        });
        crab_cage::console::line(&serde_json::to_string(&payload)?);
        return Ok(());
    }
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
    println!(
        "build --json: {}",
        if tool.supports_build_json {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "fix --sandbox: {}",
        if tool.supports_fix_sandbox {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "capabilities --sandbox --summary: {}",
        if tool.supports_sandbox_summary {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "build --opt: {}",
        if tool.supports_opt { "yes" } else { "no" }
    );
    Ok(())
}

const SAMPLE_CLIP: usize = 1000;

fn clip_sample(text: &str) -> String {
    let mut count = 0usize;
    for (byte, _) in text.char_indices() {
        if count == SAMPLE_CLIP {
            return format!("{}...(truncated)", &text[..byte]);
        }
        count += 1;
    }
    text.to_string()
}

fn objective_rule(objective: Objective) -> &'static str {
    match objective {
        Objective::PassAll => {
            "Lower is better; score is the number of failed cases (0 is solved)."
        }
        Objective::Minimize => {
            "Lower is better; each case prints `score: <number>` on its last stdout line and the score is the sum."
        }
        Objective::Fastest => {
            "Lower is better; every case must match its expected output, then the score is the sum of median run times in ms."
        }
    }
}

fn inspect_cmd(args: InspectArgs) -> Result<()> {
    let loaded = nugget::load(&args.nugget_dir)?;
    let goal_spec = nugget::load_goal(&args.nugget_dir)?;
    let case_ids: Vec<&str> = loaded.cases.iter().map(|c| c.id.as_str()).collect();
    let sample = loaded.cases.first().map(|case| {
        serde_json::json!({
            "id": case.id,
            "input": clip_sample(&case.input),
            "expected": case.expected.as_deref().map(clip_sample),
        })
    });
    if args.json {
        let payload = serde_json::json!({
            "name": loaded.name,
            "objective": loaded.objective.as_str(),
            "objective_rule": objective_rule(loaded.objective),
            "goal": goal_spec.map(|g| g.goal),
            "tolerance": goal_spec.map(|g| g.tolerance),
            "timeout_ms": loaded.timeout.as_millis() as u64,
            "max_memory": loaded.max_memory,
            "region_first_line": loaded.region_first_line,
            "seed": loaded.seed,
            "template": loaded.splice(&loaded.seed),
            "cases_count": loaded.cases.len(),
            "case_ids": case_ids,
            "sample_case": sample,
        });
        crab_cage::console::line(&serde_json::to_string(&payload)?);
        return Ok(());
    }
    println!("nugget: {}", loaded.name);
    println!("objective: {}", loaded.objective.as_str());
    println!("rule: {}", objective_rule(loaded.objective));
    if let Some(g) = goal_spec {
        println!(
            "goal: {} (tolerance {}, threshold <= {})",
            g.goal,
            g.tolerance,
            g.threshold()
        );
    }
    println!(
        "limits: timeout_ms={} max_memory={}",
        loaded.timeout.as_millis(),
        loaded.max_memory
    );
    println!("region_first_line: {}", loaded.region_first_line);
    println!("cases ({}): {}", loaded.cases.len(), case_ids.join(", "));
    println!("--- seed ---");
    print!("{}", loaded.seed);
    if !loaded.seed.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn summary_cmd(args: SummaryArgs) -> Result<()> {
    let (journal_path, out_dir) = if args.path.is_dir() {
        (args.path.join("journal.jsonl"), Some(args.path.as_path()))
    } else {
        (args.path.clone(), args.path.parent())
    };
    if !journal_path.is_file() {
        return Err(err(format!(
            "{}: journal file not found",
            journal_path.display()
        )));
    }
    let stats = parse_journal_stats(&journal_path)?;
    let species_manifest = out_dir.and_then(crab_cage::species::read_manifest_json);
    if args.json {
        let best_region = out_dir
            .map(|dir| dir.join("best.region.rs"))
            .filter(|path| path.is_file())
            .and_then(|path| fs::read_to_string(path).ok());
        let top_codes: Vec<serde_json::Value> = stats
            .top_codes
            .iter()
            .map(|(code, count)| serde_json::json!({ "code": code, "count": count }))
            .collect();
        let best = stats
            .best
            .as_ref()
            .map(|(score, hash)| serde_json::json!({ "score": score, "hash": hash }));
        let proposers: Vec<serde_json::Value> = stats
            .proposers
            .iter()
            .map(|p| {
                serde_json::json!({
                    "proposer": p.name,
                    "candidates": p.candidates,
                    "proven": p.proven,
                    "best_proven": p.best_proven,
                })
            })
            .collect();
        let payload = serde_json::json!({
            "journal": journal_path.display().to_string(),
            "candidates": stats.candidates,
            "status": {
                "rejected": stats.rejected,
                "scored": stats.scored,
                "proven": stats.proven,
                "diverged": stats.diverged,
            },
            "rejected_gates": {
                "prefilter": stats.gate_prefilter,
                "check": stats.gate_check,
                "build": stats.gate_build,
                "run": stats.gate_run,
                "oracle": stats.gate_oracle,
            },
            "top_codes": top_codes,
            "proposers": proposers,
            "species": species_manifest,
            "best": best,
            "ms_total": {
                "check": stats.ms_check,
                "build": stats.ms_build,
                "run": stats.ms_run,
                "opt": stats.ms_opt,
                "oracle": stats.ms_oracle,
            },
            "best_region": best_region,
        });
        crab_cage::console::line(&serde_json::to_string(&payload)?);
        return Ok(());
    }
    let text = Journal::summary(&journal_path)?;
    crab_cage::console::print(&text);
    if let Some(sp) = &species_manifest {
        if let Some(count) = sp.get("count").and_then(|v| v.as_u64()) {
            crab_cage::console::line(&format!("species proven={count}"));
        }
    }
    crab_cage::console::line(&format!(
        "ms total check={} build={} run={} opt={} oracle={}",
        stats.ms_check, stats.ms_build, stats.ms_run, stats.ms_opt, stats.ms_oracle
    ));
    Ok(())
}

struct ProposerStat {
    name: String,
    candidates: u64,
    proven: u64,
    best_proven: Option<f64>,
}

#[derive(Default)]
struct JournalStats {
    candidates: u64,
    rejected: u64,
    scored: u64,
    proven: u64,
    diverged: u64,
    gate_prefilter: u64,
    gate_check: u64,
    gate_build: u64,
    gate_run: u64,
    gate_oracle: u64,
    ms_check: u64,
    ms_build: u64,
    ms_run: u64,
    ms_opt: u64,
    ms_oracle: u64,
    top_codes: Vec<(String, u64)>,
    proposers: Vec<ProposerStat>,
    best: Option<(f64, String)>,
}

fn parse_journal_stats(path: &Path) -> Result<JournalStats> {
    let text = fs::read_to_string(path)?;
    let ends_with_newline = text.ends_with('\n');
    let segments: Vec<&str> = text.split('\n').collect();
    let mut stats = JournalStats::default();
    let mut codes: HashMap<String, u64> = HashMap::new();
    let mut proposer_map: HashMap<String, (u64, u64, Option<f64>)> = HashMap::new();
    for (index, raw) in segments.iter().enumerate() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let is_last = index + 1 == segments.len();
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) if is_last && !ends_with_newline => continue,
            Err(error) => {
                return Err(err(format!(
                    "{}: journal line {}: {error}",
                    path.display(),
                    index + 1
                )));
            }
        };
        stats.candidates += 1;
        let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let gate = value.get("gate").and_then(|v| v.as_str()).unwrap_or("");
        let proposer_name = value
            .get("proposer")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let p_entry = proposer_map
            .entry(proposer_name.to_string())
            .or_insert((0, 0, None));
        p_entry.0 += 1;
        if status == "proven" {
            p_entry.1 += 1;
            if let Some(sc) = value
                .get("score")
                .and_then(|v| v.as_f64())
                .filter(|s| s.is_finite())
            {
                p_entry.2 = Some(match p_entry.2 {
                    Some(prev) => prev.min(sc),
                    None => sc,
                });
            }
        }
        match status {
            "rejected" => {
                stats.rejected += 1;
                match gate {
                    "prefilter" => stats.gate_prefilter += 1,
                    "check" => stats.gate_check += 1,
                    "build" => stats.gate_build += 1,
                    "run" => stats.gate_run += 1,
                    "oracle" => stats.gate_oracle += 1,
                    _ => {}
                }
            }
            "scored" => stats.scored += 1,
            "proven" => stats.proven += 1,
            "diverged" => stats.diverged += 1,
            _ => {}
        }
        if let Some(diags) = value.get("diagnostics").and_then(|v| v.as_array()) {
            for diag in diags {
                if let Some(code) = diag.get("code").and_then(|v| v.as_str()) {
                    if !code.is_empty() {
                        *codes.entry(code.to_string()).or_default() += 1;
                    }
                }
            }
        }
        if let Some(ms) = value.get("ms") {
            stats.ms_check += ms.get("check").and_then(|v| v.as_u64()).unwrap_or(0);
            stats.ms_build += ms.get("build").and_then(|v| v.as_u64()).unwrap_or(0);
            stats.ms_run += ms.get("run").and_then(|v| v.as_u64()).unwrap_or(0);
            stats.ms_opt += ms.get("opt").and_then(|v| v.as_u64()).unwrap_or(0);
            stats.ms_oracle += ms.get("oracle").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        if matches!(status, "scored" | "proven") {
            if let (Some(score), Some(hash)) = (
                value
                    .get("score")
                    .and_then(|v| v.as_f64())
                    .filter(|s| s.is_finite()),
                value.get("hash").and_then(|v| v.as_str()),
            ) {
                let take = match &stats.best {
                    Some((best_score, _)) => score < *best_score,
                    None => true,
                };
                if take {
                    stats.best = Some((score, hash.to_string()));
                }
            }
        }
    }
    let mut ranked: Vec<(String, u64)> = codes.into_iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(10);
    stats.top_codes = ranked;
    let mut prop_list: Vec<ProposerStat> = proposer_map
        .into_iter()
        .map(|(name, (candidates, proven, best_proven))| ProposerStat {
            name,
            candidates,
            proven,
            best_proven,
        })
        .collect();
    prop_list.sort_by(|a, b| {
        b.candidates
            .cmp(&a.candidates)
            .then_with(|| a.name.cmp(&b.name))
    });
    stats.proposers = prop_list;
    Ok(stats)
}

fn init_cmd(args: InitArgs) -> Result<()> {
    let objective = match args.objective.as_str() {
        "pass_all" => Objective::PassAll,
        "minimize" => Objective::Minimize,
        "fastest" => Objective::Fastest,
        other => {
            return Err(err(format!(
                "unknown objective `{other}`; expected pass_all, minimize or fastest"
            )));
        }
    };
    let spec_path = args.dir.join("nugget.json");
    let template_path = args.dir.join("template.rs");
    if spec_path.exists() || template_path.exists() {
        return Err(err(format!(
            "{}: nugget files already exist; choose an empty directory",
            args.dir.display()
        )));
    }
    let name = match args.name {
        Some(name) if !name.trim().is_empty() => name,
        _ => args
            .dir
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("my-nugget")
            .to_string(),
    };
    if let Some(tol) = args.tolerance {
        if !tol.is_finite() || tol < 0.0 {
            return Err(err("--tolerance must be a non-negative finite number"));
        }
        if args.goal.is_none() {
            return Err(err("--tolerance needs --goal"));
        }
    }
    if let Some(goal) = args.goal {
        if !goal.is_finite() {
            return Err(err("--goal must be a finite number"));
        }
    }
    let cases_dir = args.dir.join("cases");
    fs::create_dir_all(&cases_dir)?;
    let mut spec = serde_json::json!({
        "name": name,
        "objective": objective.as_str(),
        "timeout_ms": 2000,
        "max_memory": "64M",
    });
    if let Some(obj) = spec.as_object_mut() {
        if let Some(goal) = args.goal {
            obj.insert("goal".to_string(), serde_json::Value::from(goal));
        }
        if let Some(tol) = args.tolerance {
            obj.insert("tolerance".to_string(), serde_json::Value::from(tol));
        }
    }
    fs::write(
        &spec_path,
        format!("{}\n", serde_json::to_string_pretty(&spec)?),
    )?;
    let (template, sample_in, sample_out) = match objective {
        Objective::PassAll | Objective::Fastest => (
            "\
use std::io::{self, Read};

fn solve(xs: &[i64]) -> i64 {
    // CRAB:BEGIN
    xs.iter().sum()
    // CRAB:END
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let xs: Vec<i64> = input
        .split_whitespace()
        .map(|s| s.parse::<i64>().unwrap())
        .collect();
    println!(\"{}\", solve(&xs));
}
",
            "1 2 3\n",
            Some("6\n"),
        ),
        Objective::Minimize => (
            "\
use std::io::{self, Read};

fn cost(xs: &[i64]) -> f64 {
    // CRAB:BEGIN
    xs.iter().map(|&x| (x * x) as f64).sum()
    // CRAB:END
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let xs: Vec<i64> = input
        .split_whitespace()
        .map(|s| s.parse::<i64>().unwrap())
        .collect();
    println!(\"score: {}\", cost(&xs));
}
",
            "1 2 3\n",
            None,
        ),
    };
    fs::write(&template_path, template)?;
    fs::write(cases_dir.join("01.in"), sample_in)?;
    if let Some(expected) = sample_out {
        fs::write(cases_dir.join("01.out"), expected)?;
    }
    println!(
        "created {} nugget in {}",
        objective.as_str(),
        args.dir.display()
    );
    Ok(())
}
