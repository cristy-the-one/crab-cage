# Crab Cage

Crab Cage is an offline workbench that searches for process controllers. A local model proposes a control law. A genetic algorithm tunes its numbers. Each candidate is valid Rust. When a `lust` binary is on `PATH`, that Rust is type-checked and built inside lust's sandbox, then run under a host wall-clock limit. The search result is one standalone `.rs` file plus a report.

The product belongs to Marius, who maintains [lust-lang](https://github.com/cristy-the-one/lust-lang). Crab Cage does not vendor lust and does not patch it. Anything lust cannot do yet is listed at the end as a proposed lust change.

## The job

A process has many measured signals and a few things worth driving or predicting. The mapping depends on time: disturbances repeat, and operating points drift. The user describes

- the measured inputs,
- the actuator outputs and the predicted outputs,
- the objective and the constraints,
- a time series, or a simulator that stands in for one.

Crab Cage then

1. Asks a local model for a candidate controller written as Rust that lust can accept.
2. Evolves a population. Numeric parameters move by mutation and crossover. The model is the structural mutation operator: it changes which terms of the law are live.
3. Scores a candidate by running it against the simulator or the series, inside the sandbox, with a memory cap and a host timeout.
4. Feeds lust diagnostics back so a broken candidate can be repaired (`lust check`, `lust fix`, then the model).
5. Writes the best controller as a file `rustc` compiles on its own, and a report: the fitness curve, the Pareto archive, prediction against the plant, and the trend-shift scenario.

## What lust is, here

Lust compiles a subset of Rust. A program it accepts is valid Rust and, on that subset, matches `rustc`. Crab Cage relies on the surface in lust-lang v0.3.0 / main (pull request 29, merged: lust fix, reject rewrites, `--max-memory`, and the release setup) and on `docs/sandbox.md` there.

| Command | What Crab Cage uses it for |
|---|---|
| `lust check --json --sandbox file.rs` | Type-check and borrow-check a candidate. JSON lines carry `code`, `message`, `span`, `fix`, `edits`. |
| `lust fix file.rs --all` | Apply the edits those diagnostics carry, then check again. `--sandbox` is passed; if that build of lust rejects the flag, Crab Cage retries without it. |
| `lust build --sandbox --max-memory <bytes> -o bin file.rs` | Build the guest. `--max-memory` is the cap in lust-lang v0.3.0 / main. It requires `--sandbox`. |
| `lust check --rustc` | Second opinion from real `rustc`. Not the default path. `--rustc` refuses `--sandbox`. |
| `lust watch check` | Editor loop, kept for a later UI. The search loop calls `check` itself. |
| `lust capabilities --json` | The subset the prompt should stay inside. |

A warm daemon answers a one-file body edit in a handful of milliseconds (the lust notes measure 6–8 ms when nothing changed; a body edit on a large crate is tens of milliseconds). Crab Cage keeps the harness outside `fn control` stable so a later edit of that one body is the case the daemon is built for. `LUST_NO_DAEMON=1` is set on `build` and `fix`, so a timed-out compile does not leave a background daemon holding the candidate. `check` may use the daemon.

The sandbox (`lust check|build|run --sandbox`) allows stdin, stdout, stderr, memory, math, formatting, and `std::time`. It refuses filesystem, network, environment, process spawn, and threads at check time (`L0532`). Codegen plants `lust_sandbox_deny` in front of any other OS intrinsic, including ones the check cannot see (`Path::exists`). Over the memory cap the guest prints

```text
error[L0532]: this program was built with --max-memory <bytes> and tried to use more memory
```

and exits 101. The same code and exit are used when a sandboxed program touches a refused family. `--max-memory` reads the OS peak (resident set on Linux and macOS, peak commit on Windows) after an allocation, about once per 64th of the limit. One request larger than the whole limit is refused before it is made.

Lust does not limit time. `docs/sandbox.md` says the host kills a program that runs too long. That kill is Crab Cage's job.

## Demo process

The MVP ships one simulator, a jacketed tank. It is deterministic for a given scenario id.

State is temperature `T` (°C). Actuators are heater duty `u ∈ [0, 1]` and coolant-valve opening `c ∈ [0, 1]`. The discrete step (`dt = 1`) is

```text
ΔT = 1.8 u
   − 0.9 c (T − inlet)
   − 0.02 (T − ambient)
   − 0.06 flow (T − inlet)
   + 0.45 leak
```

The controller does not see those coefficients. It sees one sample of six inputs:

| Input | Meaning |
|---|---|
| `measured_temperature` | Previous true temperature, plus Gaussian noise of scale 0.2 |
| `setpoint` | Where the temperature should be |
| `ambient` | Room or outdoor temperature |
| `coolant_flow` | Disturbance on the jacket |
| `inlet_temperature` | Coolant inlet |
| `leak` | A sensor correlated with an extra heat leak |

It returns three numbers: `heater`, `coolant_valve`, `predicted_temperature` (its guess of the next true temperature).

Two scenarios, 280 steps each:

- **Nominal.** Setpoint holds at 58, then 70, then 62. Ambient swings slowly. Coolant flow oscillates with a period of 30 steps. The leak oscillates with a period of 37.
- **Trend shift.** Setpoint stays at 64 while ambient, flow, inlet, and leak all drift upward. This is the stability check.

The initial temperature is 28 °C, so a controller that never heats scores a tracking error of hundreds of degrees squared. A proportional law with a small gain still misses. Integral action removes the offset. Feedforward and a one-step linear predictor are available once the law turns them on; they do not affect a law that has not enabled them.

## Architecture

```text
crab-cage
  CLI  ── crab-cage.toml / flags
    │
    ├─ LLM backend
    │     mock      deterministic structural proposals, used by tests
    │     local     HTTP/1.1 JSON to 127.0.0.1, localhost, or ::1 only
    │
    ├─ GA engine
    │     genome = law + numeric genes
    │     operators: gaussian mutation, blend crossover, LLM structural edit
    │     archive: Pareto on four objectives
    │
    ├─ fitness
    │     native: the genome interpreted in-process (same arithmetic as the .rs file)
    │     sandbox: the emitted .rs compiled and spoken to over stdin/stdout
    │
    ├─ lust runner
    │     check / fix / build --sandbox --max-memory
    │     falls back to rustc when lust is absent or has no --max-memory
    │
    └─ supervisor
          wall-clock kill
          RLIMIT_AS (Unix) or a job-object commit cap (Windows)
          secret-like environment variables stripped; on Unix the candidate
          starts from a cleared environment plus PATH
```

There is one crate. No network service, no account, no telemetry, no database. The only sockets Crab Cage opens are the ones the user pointed at a loopback model server.

### LLM backend

`LlmBackend` has two methods.

- `mutate` looks at the current law, the genes, the fitness, the emitted source, and any diagnostics. It returns a new law and a list of gene edits.
- `repair` looks at source plus diagnostics and may return replacement source.

The mock backend is a fixed schedule, so a seed repeats on Linux and Windows:

| When | Proposal |
|---|---|
| generation ≥ 1 and the elite is proportional | switch to PI, `ki` at least 0.02 |
| generation ≥ 5 and the elite is PI | switch to PI + feedforward, with gains near `loss/heat`, `flow/heat`, and `−leak/heat`, plus a coolant valve |
| generation ≥ 8 and the elite is feedforward | switch on the one-step predictor; `mix` stays 0 so feedback is unchanged |

The local backend posts to `{endpoint}/chat/completions` with `temperature` 0.2. The body is the OpenAI chat shape that Ollama and `llama-server` both speak. The URL must be `http` (not `https`) and the host must be loopback. Redirects are not followed. The response is capped at 1 MiB. The model is asked for one JSON object (`law`, `genes`, `note`). A fenced block is stripped. A repair request asks for Rust source and is rejected by the host prefilter if it contains `unsafe`, `async`, `std::fs`, `std::net`, `std::env`, `std::process`, `std::thread`, or `include!`.

The model never sees a path to the open internet from Crab Cage. Downloading weights is a separate step the user does once, with the model's own tool, before disconnecting.

### GA engine

A `(μ + λ)` loop with elitism.

- Population 16, 12 generations, in the default demo. Both are flags.
- The random stream is a portable PCG. The plant noise uses another stream of the same generator. Nothing in the score depends on `rand` or on the OS.
- Initial genomes are proportional, `kp` in `[0, 0.03]`, a tiny bias, every other gene zero.
- Each generation keeps the two best unchanged.
- If the model proposed a new law, one child is the elite with that proposal applied and no extra noise, so the structural edit is actually evaluated.
- Other children are blend crossovers (or copies) plus gaussian mutation. Sigma starts at 0.18 of each gene's range and shrinks by 0.94 per generation. Genes are clamped to the bounds in `genome::BOUNDS`.
- The proposal is applied only to children whose law is still the elite's law, after numeric mutation, so a floor such as `ki ≥ 0.02` is not immediately wiped out.

Fitness is deterministic, so elites are not re-evaluated for noise. An individual that does not answer, or answers with non-finite numbers, scores as a failure (`f64::MAX / 4`).

The archive holds up to eight non-dominated individuals on `(nominal tracking, trend-shift tracking, prediction error, effort)`.

### Genome

The genome is a law plus seventeen finite numbers. The law decides which numbers are live. A gene that belongs to a law the candidate has not reached does not affect the output, so mutating it early is harmless and it is already in place when the law turns on.

| Law | Live terms |
|---|---|
| `hold` | constant bias |
| `proportional` | bias + `kp` · error |
| `pi` | proportional + `ki` · integral |
| `pid` | PI + `kd` · derivative + coolant valve on positive error |
| `pi_feedforward` | PI + ambient, flow, and leak feedforward + coolant valve |
| `pi_feedforward_predict` | feedforward + a one-step linear model of `ΔT` + a window slope and a single Fourier bin, mixed into the error by `mix` |

The integral is clamped to ±120 and is frozen while the heater command is saturated against the error (anti-windup). The window is 2 to 16 samples. The Fourier bin is period 30, the nominal flow cycle. Slope is `(newest − oldest) / (n − 1)`.

This is a typed genome, not a free AST. The model proposes a law and numbers; Crab Cage lowers them to Rust. A free-form source replacement exists on the repair path and is not what the demo search evaluates. Letting the model edit statements inside `fn control`, with lust as the constraint, is a later phase. The trait already has a place for that (`repair` returns source).

### What is emitted

`best_controller.rs` is one file, no dependencies, edition 2021. It is valid Rust. It stays inside the lust sandbox subset:

- `fn control(...)` holds every coefficient as a literal, so a change is an edit of one function body.
- `main` speaks a line protocol on stdin/stdout and flushes every reply. `reset` clears the memory. Six finite floats in, three floats out.
- No `unsafe`, no `async`, no filesystem, network, environment, process, or thread API. The host prefilter rejects those tokens outside comments. `lust check --sandbox` is the real check when lust is present.

`rustc --edition 2021` compiles the file. The replay test drives the binary with the same input trace the in-process controller saw and requires the three outputs to match within `1e-6`. The literals use Rust's round-trip debug format, so the numbers in the file are the numbers in the genome.

### Fitness

Lower is better. For each scenario the rollout records

- tracking MSE of true temperature against the setpoint,
- effort, the mean square of heater and valve increments,
- prediction MSE of the controller's one-step guess against the next true temperature,
- overshoot, the mean square of temperature above `setpoint + 1.5`.

The scalar used for selection is

```text
1.00 · tracking_nominal
+ 0.65 · tracking_trend
+ 4.00 · effort_nominal
+ 0.35 · prediction_nominal
+ 0.20 · prediction_trend
+ 0.40 · overshoot_nominal
+ 0.25 · overshoot_trend
```

Tracking dominates. Effort and overshoot keep a bang-bang law from winning on tracking alone. Prediction is a separate objective: with `mix = 0` the predictor does not move the actuators, so a model can improve the prediction term without undoing a good PI. The trend-shift weight is what asks the controller to survive a drift it was not tuned on as a separate scenario. Both scenarios are always evaluated. There is no held-out series in the MVP; the trend shift is the stress case inside the score, which means the search can overfit it. A later phase splits a series the fitness function does not see.

Closed-loop evaluation is interactive when the candidate is a binary. The host owns the plant. Each step it writes one input line and reads one output line. The candidate cannot see the plant coefficients or the score. A short or non-numeric reply fails the individual.

The default search (`--eval auto`) evaluates the genome in-process, because that arithmetic is the meaning of the emitted file, and the file is checked by replay. `--eval sandbox` compiles the winner and re-scores the closed loop under the host limits, and refuses the run if that score disagrees with the genome by more than `1e-2`. `--eval native` skips both lust and `rustc`; tests that only care about the search use it.

### Supervisor

Every candidate binary is a child of Crab Cage.

- **Time.** A thread reads stdout line by line. The parent kills the process group when the wall clock (`--timeout-ms`, default 5 s) expires. On Unix the child calls `setpgid(0, 0)` before `exec`, and the kill is `kill(-pid, SIGKILL)` plus `child.kill()`. Sleeping, spinning, and blocking on a pipe all die. The test starts a probe that sleeps for 30 s with a 400 ms budget.
- **Memory, guest.** When lust advertises `--max-memory` (0.3.0), `lust build --sandbox --max-memory N` bakes the cap in. The runner classifies exit 101 plus the `max-memory` line as a lust memory stop, and the same code with a different `L0532` line as a sandbox violation (`fs`, `net`, …).
- **Memory, host.** This is the backstop lust's own note asks for: the guest check runs on allocation, so a reservation that is touched later is not seen, and the stack is not part of the guest cap. Unix sets `RLIMIT_AS` to `--host-memory` (default 1 GiB) in the child before `exec`. Windows assigns the process to a job object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY` (commit charge, the same quantity lust reads on Windows) before any stdin byte is written. The candidate blocks on stdin until that assignment has happened. The test allocates 160 MiB under a 64 MiB cap (must die) and 32 MiB under a 1 GiB cap (must live).

On Unix the candidate's environment is cleared except `PATH`. On Windows, variables whose names contain `SECRET`, `TOKEN`, `PASSWORD`, `CREDENTIAL`, or end in `_KEY` are removed; `SystemRoot` stays, because the process will not start without it. Lust and `rustc` themselves are trusted tools and keep the parent environment.

Stdout and stderr captured from a run are capped at 2 MiB.

### Report

`crab-out/report.json` (schema 1) and `crab-out/report.md` contain the scalar, the per-scenario breakdown, the per-generation curve, the Pareto archive, a sliced nominal trace (temperature, setpoint, heater, prediction, next true temperature), and what the verifier did: `rustc` replay error, lust version, whether `--max-memory` exists, and the check result.

## Models for this laptop

RTX 5080 Laptop, 16 GB VRAM, 64 GB RAM. One resident GGUF. No second copy, no cloud fallback.

| Role | Model | Why |
|---|---|---|
| Default mutation operator | Qwen2.5-Coder-14B-Instruct, GGUF Q5_K_M (about 10.5 GB of weights) | Apache-2.0. With an 8k context and FP16 KV the working set is about 12–13 GB, which fits in 16 GB with the desktop up. |
| More context | same family, Q4_K_M (about 9 GB) or Q5_K_M with a Q8 KV cache | 16k tokens otherwise crowds the card. |
| Headroom for another process | Qwen2.5-Coder-7B-Instruct, Q6_K or Q8_0, about 6–8 GB | Apache-2.0. Prefer the 14B when it is the only model loaded. |
| One-shot rewrite of a winner | Qwen2.5-Coder-32B-Instruct, Q4_K_M | Weights are about 18–19 GB. They do not fit in 16 GB. Part of the model sits in the 64 GB of system RAM. Usable for a single pass, not for a population. Q3_K_M (about 15 GB) is tight once KV is added and is not the default. |

Runtime, after the weights are on disk:

```text
OLLAMA_HOST=127.0.0.1:11434 ollama serve
crab-cage evolve --backend local --endpoint http://127.0.0.1:11434/v1 --model qwen2.5-coder:14b
```

or llama.cpp (MIT):

```text
llama-server -m qwen2.5-coder-14b-instruct-q5_k_m.gguf -ngl 99 -c 8192 --host 127.0.0.1 --port 8080
crab-cage evolve --backend local --endpoint http://127.0.0.1:8080/v1 --model qwen2.5-coder
```

Context 8192 covers the plant description, the current controller, and the genome schema. Codestral is not recommended; its license is not a permissive one. Crab Cage does not ship weights and does not download them.

## Desktop UI

Not in this tree. The report is the contract a UI would read. A later UI is a Tauri shell (MIT / Apache-2.0, already used on other projects here) with a TypeScript front end:

- the fitness curve and the Pareto archive from `report.json`,
- the nominal and trend-shift traces,
- the emitted source,
- lust diagnostics next to the lines they cite,
- a backend picker that only offers loopback endpoints.

The shell starts the same `crab-cage` library. It does not grow a second search implementation. Tauri's system webview is the window; there is no bundled Chromium.

## Safety boundaries

- Candidates run as separate processes. The host owns the plant and the score.
- The guest, when built by lust 0.3.0, cannot open files, sockets, or the environment. The host cap and the wall clock still apply, because lust does not cover time, stack, or a touch of previously reserved pages.
- `rustc` replay checks numerical agreement with the genome. It does not enforce the sandbox. A host that only has `rustc` is told so in the report (`host backstop only`).
- The model endpoint is rejected unless it is loopback HTTP. There is no API key, no telemetry sink, and no fallback host.
- Emitted source is filtered before it is executed. Lust's checker is the authority when the binary exists.

## Roadmap

Sizes are the amount of code and the surface that changes. They are not a schedule.

| Phase | Size | What it adds |
|---|---|---|
| 0, this tree | one crate, about twenty modules, one demo | Typed genome, mock and local backends, native search, lust runner, host limits, report, Linux and Windows CI |
| 1 | the runner plus a warm work directory | Every offspring built by lust, body-patch checks, diagnostics fed back into `mutate` on failures. Sandbox becomes the default search when lust 0.3.0 is on `PATH` |
| 2 | a statement-level genome inside `fn control` | The model edits the body. Crossover swaps statement blocks. `lust check --sandbox` is the constraint. `syn` (Apache-2.0 / MIT) or a smaller parser |
| 3 | a Tauri crate beside this one | Curves, traces, diagnostics, loopback backend picker. Reads `report.json` |
| 4 | a CSV reader and a second simulator | User series replace the tank. The protocol stays the line protocol. A split the fitness function does not see |
| 5 | one optimizer module | Full-covariance CMA-ES on the numeric genes. The structural operator stays the model |

## What this tree implements

- `crab-cage evolve` runs the tank demo with the mock backend, prints the curve and the best fitness, and writes `best_controller.rs`, `report.md`, and `report.json`.
- `--backend local` talks to a loopback model server and refuses anything else.
- With lust 0.3.0 on `PATH` or `--lust-bin`, the winner is `lust check --sandbox` and, when the flags are advertised, `lust build --sandbox --max-memory --time-limit`.
- Without lust, the winner is compiled with `rustc` and replayed under the host memory cap and the wall clock. The report says lust was absent.
- Tests cover: the search improves fitness on several seeds and repeats bit-for-bit; a hand PI beats a weak proportional; the emitted file compiles with `rustc` and matches the genome; the host kills a sleeper; the host memory cap stops a large allocation and allows a small one; a fake lust is invoked with `--sandbox`, `--max-memory` and `--time-limit`, and `L0532` / `L0533` lines are classified as a guest memory stop and a guest time stop.

## Gaps in lust

These are proposals for lust. Crab Cage works around the ones that are still open. The time limit is the exception: the implementation is `docs/lust-time-limit.patch`, against lust-lang v0.3.0 / main at `cbc913832b6f`. Apply it there with `git am`. This environment has no credentials for `github.com/cristy-the-one/lust-lang`, so the patch is the copy of that commit. Crab Cage passes the flag when the binary advertises it.

1. **Time limit.** `lust build` and `lust run` take `--sandbox --time-limit <duration>`. A bare number is seconds; `500ms`, `2s` and `1m` name the unit. The deadline is monotonic and set in `main`. A loop checks it on every back-edge, a function entry checks it every 64th call, and `thread::sleep` does not sleep past it. The stop is `error[L0533]`, exit 101, stdout flushed first. It is not another meaning of `L0532`. `check` does not take the flag. A straight-line body that never loops, calls, or sleeps is not probed, and a recursion that fills the stack first is still the stack-overflow stop, so the host wall clock stays the backstop. Crab Cage passes the configured run timeout when `lust build --help` or the usage text contains `--time-limit`.

2. **`--max-memory` is in lust-lang v0.3.0 / main.** v0.2.0 does not have the flag. Crab Cage detects it from the binary's help or usage text and otherwise uses only the host cap.

3. **The guest cap misses some memory.** It samples on allocation. Memory reserved and touched later, with no further allocation, is not seen. The stack is excluded (`fatal runtime error: stack overflow` is a different stop). A sampler thread cannot be the fix: the sandbox refuses threads. Proposal: a back-edge probe in codegen, and an optional stack size on the sandbox profile. Until then the host `RLIMIT_AS` / job object is the backstop, which is what `docs/sandbox.md` already tells the host to keep.

4. **`lust fix` and `--sandbox`.** The 0.3.0 notes specify `lust fix file.rs [--all] [--dry-run]`. They do not say that `fix` accepts `--sandbox`. A rewrite that is legal Rust can still be illegal in the sandbox, and a fix that does not re-check under `--sandbox` would report success for a program the sandbox then rejects. Proposal: `lust fix` takes the same check flags as `lust check`, including `--sandbox`. Crab Cage passes `--sandbox` and, if the binary says the flag is unknown, retries without it and records that in the run.

5. **No sandbox capability list.** `lust capabilities --json` describes the full subset, not the sandbox subset. The system prompt cannot ask lust what the sandbox refuses. Proposal: `lust capabilities --sandbox --json`, the same probe format, limited to what `--sandbox` accepts.

6. **Run results are not structured.** A host infers the stop from the exit code and a stderr line. Stdout belongs to the program (rule zero). Proposal: an opt-in flag, off by default so existing tests stay quiet, that writes one JSON object to stderr after the program stops: class (`exit`, `panic`, `sandbox`, `memory`, `signal`), peak memory, and the `L0532` family when there is one. Crab Cage would read that instead of matching prose.

7. **`std::process::exit` is allowed.** A candidate can exit 0 without speaking the protocol. That is not an escape. Crab Cage treats a short or non-numeric transcript as a failed evaluation. No lust change is required. It is recorded so nobody "fixes" it by banning `exit` and breaking `Termination`.

8. **Warm checks on Windows.** The daemon listens on a Unix socket and the older docs mark Linux x86_64 as the verified host. Pull request 29 adds Windows and macOS arm64 packages. The 2–5 ms warm figure is a Linux number. Proposal: publish the warm-check distribution per OS, and make `lust watch` / the daemon a supported Windows path. Crab Cage's population loop wants the same check latency there. Until that exists, Windows runs pay the cold one-file check, which is still the right trade against `rustc` for a single file of this size.

9. **`--rustc` and `--release` refuse `--sandbox`.** A second opinion cannot be a sandboxed `rustc` binary. `lust check --sandbox --rustc` can still compare diagnostics, because check does not run the program. The OS boundary is lust's codegen stop, which `rustc` does not insert. Crab Cage uses `rustc` only to prove the source compiles and matches the genome numerically. Proposal: document that split in `docs/sandbox.md` so a host does not expect `rustc` to enforce `L0532`.

10. **Extra file descriptors.** The sandbox doc says a host that passes extra fds still owns them; std has no safe way to reach a descriptor it did not open, but the host should pass only the three stdio streams. Crab Cage pipes only those. No lust change. The runner must not grow a shared-memory channel later without revisiting this.

## License

Crab Cage is MIT OR Apache-2.0. Dependencies are checked with `cargo deny` (`deny.toml`): MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0 / Unicode-DFS-2016, CC0, Unlicense. GPL and LGPL are not allowed. Git dependencies are not allowed. Lust is an external binary, not a crate, so its own license does not enter this graph. Model weights are not shipped.
