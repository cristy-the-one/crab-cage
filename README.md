# Crab Cage

Crab Cage tries many small changes to one marked region of a Rust program and keeps the best one that is proven correct.

- [lust](https://github.com/cristy-the-one/lust-lang) is the fast gate. It checks and builds a subset of Rust in milliseconds, and everything it accepts is valid Rust.
- `rustc` is the oracle. It is slow and always right, so it runs only on a candidate that beats the best so far.
- Crab Cage is the loop between them: propose, check, build, run, score, prove.

```text
proposer ──► prefilter            host, microseconds
         ──► lust check           diagnostics go back to the proposer
         ──► lust build + run     the score
         ──► rustc build + run    only for a new best; output must equal lust's
                equal     → proven
                different → diverged, saved as a lust bug report
```

The design notes are in [docs/DESIGN.md](docs/DESIGN.md), and the plan with its status is in [docs/WORK-PACKAGES.md](docs/WORK-PACKAGES.md).

Crab Cage is MIT OR Apache-2.0. It does not phone home. The only network client is the optional model backend, and it refuses every host that is not `127.0.0.1`, `localhost`, or `::1`.

## What you need

- Rust (the 1.74 toolchain or anything newer) and `rustc` on `PATH`. Linux and Windows are the tested hosts.
- Optional: a lust binary, built from lust-lang `next` at `f2dc9a8b` or later. Without lust, `rustc` does the fast gates too: slower, same results.
- Optional, for `--backend local`: Ollama or llama.cpp with a coding model. See [A local model](#a-local-model).

## Build

The first build fetches crates from crates.io. After that, `--offline` works.

```sh
cargo build --release
cargo test --locked
```

The binary is `target/release/crab-cage` (`crab-cage.exe` on Windows).

## A nugget

A nugget is a directory that states one problem. This repo has a ladder of four, easy to hard: `nuggets/sum-squares`, `nuggets/run-lengths`, `nuggets/merge-ranges` and `nuggets/normalize-path` (each of the last three has a `reference.rs` beside it that is a solution), one real problem, `nuggets/delta-kernel`, and one speed problem, `nuggets/fast-sum`, whose seed is right but slow.

```text
nuggets/sum-squares/
  nugget.json      name, objective, timeout_ms, max_memory
  template.rs      a complete program with one marked region
  cases/seven.in   stdin for one case
  cases/seven.out  the stdout that case must produce
```

`template.rs` is plain Rust with exactly one `// CRAB:BEGIN` line and one `// CRAB:END` line. The lines between them are the seed. A **candidate** is replacement text for those lines.

```rust
fn sum_squares(xs: &[i64]) -> i64 {
    // CRAB:BEGIN
    xs.iter().sum()
    // CRAB:END
}
```

`nugget.json`:

```json
{ "name": "sum-squares", "objective": "pass_all", "timeout_ms": 2000, "max_memory": "64M" }
```

| `objective` | Score (lower is better) | Needs |
|---|---|---|
| `pass_all` | the number of cases whose stdout is wrong; 0 is solved | a `.out` file for every case |
| `minimize` | the sum over the cases of the number on the program's last stdout line, written `score: <number>` | nothing more |
| `fastest` | when every case prints its `.out`: the sum over the cases of the median of three run times, in milliseconds. Otherwise 1e9 plus the number of wrong cases, so a wrong candidate never beats a right one | a `.out` file for every case |

For `fastest`, lust times its `--opt` build (the same C compiled with `zig cc -O2`, `cc`, or `LUST_CC`), and that binary must print the same outputs, or the verdict is `diverged`. With no such compiler, or when the opt build fails, the tcc build is timed and `note` says so (`opt build unavailable, timed the tcc build`). Without lust the rustc build is timed. `ms.opt` is the time spent on the opt build and the timed runs. `nuggets/fast-sum` is the example: a correct but slow seed, and a fast `reference.rs`. Times from runs with `--jobs` above 1 are taken while other candidates run, so they are noisier.

## Evaluate one candidate

`crab-cage eval` takes one candidate through the funnel and prints a verdict. Any agent or script can call it.

```sh
crab-cage eval nuggets/sum-squares candidate.rs --json --oracle
crab-cage eval nuggets/sum-squares candidate.rs --json --oracle --lust-bin /path/to/lust
crab-cage eval nuggets/sum-squares - --json < candidate.rs     # the candidate on stdin
```

With a candidate file that holds `    xs.iter().map(|x| x * x).sum()`:

```json
{"hash":"fb78602b10932674","gate":"oracle","status":"proven","score":0.0,"passed":3,"total":3,"diagnostics":[],"ms":{"check":109,"build":410,"run":108,"oracle":398},"note":""}
```

| Field | Meaning |
|---|---|
| `gate` | the last gate reached: `prefilter`, `check`, `build`, `run`, `oracle` |
| `status` | `rejected` (stopped at a gate), `scored` (ran every case, oracle not asked for), `proven` (rustc agrees), `diverged` (rustc disagrees) |
| `score` | as in the table above; `null` when rejected before the cases ran |
| `diagnostics` | lust's errors, with `line` counted from the first line of the candidate |
| `note` | why a run was rejected: `TimeLust`, `TimedOut`, `MemoryLust`, `Panic`, `SandboxDenied`, `nondeterministic output`, … |

The exit code is 0 for `scored` and `proven`, and 1 otherwise. Without `--json` the verdict is three lines of text.

A candidate may not use `unsafe`, `async`, the file system, the network, the environment, processes or threads. The prefilter refuses those words, and with lust the program is built in lust's sandbox with a memory limit and a time limit.

## Search

`crab-cage run` evaluates the seed, then asks its proposers for candidates in turn until the budget is spent, the problem is solved, or the proposers have nothing new.

```sh
crab-cage run nuggets/sum-squares --candidates my-snippets --lust-bin /path/to/lust
crab-cage run my-nugget --tweak --budget 120 --jobs 4 --lust-bin /path/to/lust
crab-cage run my-nugget --backend local --model qwen2.5-coder:7b --lust-bin /path/to/lust
```

| Flag | Proposer |
|---|---|
| `--candidates DIR` | every `*.rs` file in `DIR`, once, in name order. Files added during the run are picked up. Any model or script can feed a run this way. |
| `--tweak` | changes one numeric literal of the best candidate per proposal: a float by about ±20%, an integer by 1 or 2. No model needed. |
| `--backend local` | asks a local model for the region, and sends lust's diagnostics back to it when a reply is rejected. `--endpoint` and `--model` choose the server. |
| (with `--backend local`) | `repair`: when a candidate does not compile, rustc is asked about it, and the fixes rustc marks as safe to apply (`MachineApplicable`, such as a missing `*` in a comparison) are applied and tried as a candidate of its own. No model involved. `--no-repair` turns it off. |

Several can be given; they take turns.

The run prints one line per candidate (its number, status, last gate, score and proposer) and a summary. Here the seed fails all three cases, one snippet has a type error, one passes two cases, and one is right:

```text
1 proven oracle 3 seed
2 rejected check - folder
3 proven oracle 1 folder
4 proven oracle 0 folder
candidates 4
status rejected=1 scored=0 proven=3 diverged=0
rejected prefilter=0 check=1 build=0 run=0 oracle=0
codes L0110=1
best 0 fb78602b10932674
```

and writes, under `--out` (default `crab-out`):

```text
journal.jsonl     one verdict per line, with the proposer's name
best.rs           the whole program with the best proven candidate in place
best.region.rs    the candidate text alone
work/             every candidate as <hash>.rs, its binary, and divergences/
prompts/          with --backend local: each request to the model and its reply, 0001.txt onwards
```

- **Only `proven` counts.** A candidate becomes the best only after `rustc` has built it and printed the same output as the fast build. A candidate whose `rustc` build disagrees with itself between two runs is rejected as nondeterministic.
- **Exit code.** For `pass_all`, 0 only when a proven candidate has score 0. For `minimize` and `fastest`, 0 when any candidate is proven; neither stops early.
- **Resume.** Run again with the same `--out`. Candidates already in the journal are skipped, and the best is restored.
- **`--coverage`** measures lust against rustc; it is not for finding the best candidate fast. rustc is asked about every candidate lust refuses to compile, and a program rustc accepts is copied to `<out>/lust-only/` with what lust said: a gap in lust. Every candidate that runs is compared with its rustc build, not only a new best, so every one ends `proven`, `diverged` or rejected by rustc. The summary gains a line `lust-only N`. It costs one rustc run for most candidates and needs `--lust-bin`. `nuggets/coverage/` holds 26 small tasks written for it, each with a `reference.rs`.
- **`--seed N`** makes a run repeatable. `--tweak` draws its changes from it (default 1). With `--backend local`, each request to the model carries a seed counted up from N, and a server that honours seeds (Ollama does) then gives the same replies again. Without it the server picks its own seed and two runs differ.
- **`--budget N`** is the number of candidates to journal, the seed included (default 200). A run also stops after 50 proposals in a row that add nothing new.
- **`--jobs N`** evaluates up to `N` candidates at the same time. The journal keeps the order the candidates were proposed in.
- **`work/divergences/<hash>/`** holds the source, the input, and both outputs whenever lust and `rustc` disagree. Each one is a lust bug report.

## Lust

Lust is an external binary. This repo does not contain it.

```sh
crab-cage lust-probe --lust-bin "$HOME/lust/bin/lust"
```

prints the version, the path, and whether `--max-memory` and `--time-limit` exist. Crab Cage passes each flag only when lust's usage text names it.

A candidate is checked with `lust check --json --sandbox` and built with `lust build --sandbox --max-memory <nugget's> --time-limit <nugget's>`. A program that runs past the limit stops itself with `L0533`; one that allocates past the cap stops with `L0532`. The host keeps its own backstops, because lust's limits do not cover everything: a wall clock half a second later than lust's, and an address-space cap (Unix) or a job-object commit cap (Windows), 1 GiB by default.

### The warm daemon

lust can keep a daemon running that answers a small edit in milliseconds.

- `crab-cage run` with `--lust-bin` uses it without being asked. It starts lust in `<out>/work` and removes the daemon's socket file when the run ends, so no daemon is left behind. `--no-lust-daemon` starts lust cold for every candidate instead.
- `crab-cage eval` leaves it off, because one call cannot know whether another will follow. `--lust-daemon` turns it on and needs `--lust-bin` and `--work-dir`; the daemon then stays warm for the next call that names the same work directory. It stops after 30 minutes idle, or at once when `<work>/.lust-daemon-*.sock` is deleted.

lust keeps one warm session per source path, so with the daemon on every candidate is compiled as `work/slot.rs` (with `--jobs N`, `slot.rs`, `slot-1.rs`, …). The candidate is still saved as `work/<hash>.rs`, and that file is what `rustc` compiles.

Measured on a Windows laptop (Ryzen 7 8840HS), demo nugget, medians per candidate:

| | `lust check` | `lust build` | 40 candidates, one job | four jobs |
|---|---:|---:|---:|---:|
| daemon off | 108 ms | 378 ms | 33.8 s | 15.2 s |
| daemon on | 18 ms | 35 ms | 11.1 s | 6.8 s |

More than four jobs did not help there: lust's builds slow down as they overlap. With a lust older than `f2dc9a8b` nothing hangs, but each lust call waits an extra half second, so use `--no-lust-daemon` with one.

## A local model

`--backend local` posts to an OpenAI-style chat endpoint on this machine. Ollama and llama.cpp's `llama-server` both serve one. `https://…` and any non-loopback host are refused, and there is no API key.

```sh
ollama pull qwen2.5-coder:7b
crab-cage run my-nugget --backend local --model qwen2.5-coder:7b --lust-bin /path/to/lust
```

```sh
llama-server -m qwen2.5-coder-14b-instruct-q5_k_m.gguf -ngl 99 -c 8192 --host 127.0.0.1 --port 8080
crab-cage run my-nugget --backend local --endpoint http://127.0.0.1:8080/v1 --model qwen2.5-coder
```

The default endpoint is `http://127.0.0.1:11434/v1` (Ollama's) and the default model name is `qwen2.5-coder:14b`.

Which model: the loop asks for many short function bodies and lets lust and `rustc` throw away the bad ones, so a model that answers quickly is worth more than a larger one that answers slowly. On a 16 GB GPU, Qwen2.5-Coder-14B (Q5_K_M, about 10.5 GB) fits with an 8k context; `crab-cage models` prints those notes. Without a discrete GPU, the 7B build is the place to start. The proposer has been run against one real model so far: a 7.6B Qwen-family coder at Q4_K_M under Ollama, on a laptop processor with no discrete GPU. It answered in about two seconds and solved the demo nugget in one request, ten runs out of ten; when its first reply was forced to be a fragment, it recovered on the next request, six runs out of six. Use `--jobs 1` with a model.

The same model did not solve `nuggets/normalize-path`. In 19 requests (about 17 s each) it wrote five different candidates, all of which lust accepted, took the seed from 8 failing cases to 6, and then went round the same three variants until the proposer stopped. The details are in `docs/WORK-PACKAGES.md`.

`experiments/model-ladder/run.ps1` runs a list of models over those nuggets several times each and writes one table: which model solved what, seconds per candidate, where candidates were rejected, and which lust error codes came up. It is made to be run on another machine and sent back; see [experiments/model-ladder/README.md](experiments/model-ladder/README.md).

## Example: controller search

The first thing built here was a search for process controllers, and it is still in the tree as `crab-cage evolve`. It does not use nuggets: the candidate is a typed genome (a control law and seventeen numbers), a genetic algorithm tunes the numbers, and a model or a fixed schedule picks the law.

```sh
cargo run --release -- evolve
cargo run --release -- evolve --lust-bin /path/to/lust
```

That is the jacketed-tank simulator: six measured inputs, a heater, a coolant valve, and a one-step temperature prediction. The run prints one line per generation and writes

```text
crab-out/best_controller.rs    one file, no dependencies
crab-out/report.md
crab-out/report.json
```

The winner is built by `rustc`, and by lust when it is given, and replayed against the recorded input; the report says whether the two builds printed identical lines (`oracle agrees`).

```sh
crab-cage evolve --seed 2 --population 16 --generations 12 --out-dir crab-out
crab-cage evolve --eval native --no-verify     # search only
crab-cage models                               # model notes for a 16 GB GPU
```

Copy `crab-cage.toml.example` to `crab-cage.toml` to set `evolve`'s options without flags. A flag wins over the file, and the file wins over the built-in default.

## Delta kernels

`docs/delta-kernels.md` is the measured comparison of v0.03 firmware delta kernels. The nugget is `nuggets/delta-kernel`. The corpus is `corpus/pairs`: compiled images plus synthetic pairs. Drop `old.srec` and `new.srec` into a new directory and run `crab-cage delta sync-cases`. A kernel's byte count counts only after `crab-cage delta eval` packs the transcript and the reference decoder accepts it.

```sh
crab-cage delta eval nuggets/delta-kernel/candidates/06-optimal.rs
crab-cage run nuggets/delta-kernel --candidates nuggets/delta-kernel/candidates
crab-cage run nuggets/delta-kernel --backend local --model qwen2.5-coder:14b
```

## Tests

```sh
cargo test --locked
```

No lust and no model are needed: the suite uses `rustc`, a fake lust, and a mock model. It covers the funnel and its verdicts, the oracle and its divergence reports, the journal and resuming, the three proposers, parallel jobs giving the same journal as one, the host limits (wall clock, memory cap, a child that leaves a grandchild holding its pipes), and the controller search.

`tests/real_lust.rs` runs against a real lust. It does nothing unless `CRAB_LUST_BIN` is set:

```sh
CRAB_LUST_BIN=/path/to/lust cargo test --test real_lust
```

PowerShell: `$env:CRAB_LUST_BIN = "C:\path\to\lust.exe"; cargo test --test real_lust`. Those tests start lust's daemon and stop it again.

CI runs the suite on Linux and Windows, plus `cargo fmt --check` and `cargo deny`. `deny.toml` allows MIT, Apache-2.0, BSD, ISC, Zlib, and the Unicode licenses. GPL and LGPL are not allowed. Dependencies come from crates.io only.

## Layout

```text
src/nugget.rs      load a nugget, splice a candidate into its template
src/pipeline.rs    the funnel for one candidate, and the verdict
src/run.rs         the search loop: batches, the oracle on a new best, resume
src/proposer.rs    the folder, literal-tweak and local-model proposers
src/journal.rs     journal.jsonl: append, dedup, summary
src/lust.rs        probe, check, build, the daemon switch, rustc as the oracle
src/supervisor.rs  wall clock, memory cap, output capture for a child
src/spawn.rs       starting a child with three pipes (a direct call on Windows)
src/llm.rs, http.rs    the loopback chat client
src/evolve.rs, ga.rs, genome.rs, plant.rs, …   the controller search
nuggets/           example nuggets, including delta-kernel
tests/             the suite; tests/probes/ are small programs the tests compile
docs/              DESIGN.md, STRATEGY.md, WORK-PACKAGES.md, delta-kernels.md, briefs/
```
