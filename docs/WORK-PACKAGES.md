# Work packages

These implement [STRATEGY.md](STRATEGY.md). Each package is one pull request. Give the implementing model **the rules below plus one package**, nothing else.

## Rules for the implementer

1. Edit only the files the package lists. If you think another file must change, stop and say so.
2. Add no dependency. The crate has clap, serde, serde_json. That is all.
3. Do not edit `src/supervisor.rs` unless the package is WP-01.
4. Before you finish, these must pass:
   ```sh
   cargo fmt --check
   cargo test --locked
   ```
   plus the package's own acceptance check.
5. Do not delete or weaken an existing test. If one fails, your change is wrong.
6. Errors use `crate::error::err("message")`. No `unwrap()` outside tests.
7. Code must build on Windows and Linux. Paths go through `std::path`. Executables get `.exe` on Windows (see `bin_name` in `src/evolve.rs`).
8. Write nothing outside the directory the user named. No files in the current directory.

## Tiers

| Tier | Who can do it | Shape |
|---|---|---|
| **T1** | A local coding model (7–14B) | One or two files, under ~120 changed lines, steps are literal |
| **T2** | A Grok-class model | Several files, a small design choice or two |
| **T3** | An Opus 5.5-class model | OS-specific or cross-process behaviour; needs debugging on a real machine |

## Order

```text
Phase 0:  WP-01 → WP-02 → WP-03 → WP-04 → WP-05, WP-06      (L-01, L-02 any time)
Phase 1:  WP-10 → WP-11 → WP-12 → WP-13 → WP-14
Phase 2:  WP-15 → WP-16 → WP-17 → WP-18 → WP-19
Phase 3:  L-03 … L-08, any order
```

---

# Phase 0: make what exists true

## WP-01 Never hang on a tool call (T3, or T1 for step A only)

**Problem.** `supervisor::run_capture` can block forever when the child starts a background process that keeps the stdout/stderr pipe open. Seen with real lust on Windows: `crab-cage lust-probe` never returned.

**Files.** `src/lust.rs`, `src/supervisor.rs`, `tests/limits.rs`, new `tests/probes/lingerer.rs`.

**Step A (T1, do first, own pull request is fine).** In `src/lust.rs`, pass `("LUST_NO_DAEMON", "1")` in the `env` argument of **every** `tool_output` call that targets lust: the three calls in `probe` and the one in `check`. `build` and `fix` already do.

**Step B (T3).** Make `run_capture` return within `limits.wall` plus 2 seconds no matter who holds the pipes. After the child has exited or been killed, give the reader threads a short grace period (500 ms), then return what was read so far without joining them. Use a channel or shared buffer; do not join a thread that may block.

**Test.** `tests/probes/lingerer.rs`: a program that, when run with no arguments, spawns itself with the argument `child` (inheriting stdio), prints `parent done`, and exits; with `child` it sleeps 30 s. In `tests/limits.rs`, compile it the same way `sleeper.rs` is compiled, run it through `run_capture` with a 2 s wall limit, and assert the call returns in under 6 s with stdout containing `parent done`.

**Done when.** The new test passes on Windows and Linux, and `crab-cage lust-probe --lust-bin <real lust.exe>` returns in under 15 s.

## WP-02 Detect lust features without guessing (T1)

**Problem.** `probe` reports `--time-limit: yes` for lust 0.2.0, because the error text `cannot read ...\--time-limit` contains the flag name.

**Files.** `src/lust.rs`, `tests/probes/fake_lust.rs`.

**Steps.**
1. In `probe`, delete the `build --help` and `build --time-limit` calls.
2. Run lust with **no arguments**. Real lust prints lines starting with `usage: lust`, followed by indented lines, on stdout or stderr.
3. Build `usage_text` from only those lines: every line that starts with `usage:` or with whitespace, after the first `usage:` line. Ignore everything else.
4. `supports_max_memory = usage_text.contains("--max-memory")`, same for `--time-limit`.
5. If no `usage:` line exists, both are `false`.
6. In `tests/probes/fake_lust.rs`, when there are no arguments print `usage: lust <check|build|run> <file.rs> [--sandbox] [--max-memory <size>] [--time-limit <duration>]` and exit 2. Keep the old `build --help` arm.

**Unit tests to add in `src/lust.rs`.** A helper `fn flags_from_usage(text: &str) -> (bool, bool)` and two tests: the real 0.2.0 usage text (copy the first two lines from `lust` run with no arguments) gives `(false, false)`; the string `error: cannot read C:\x\--time-limit` gives `(false, false)`.

**Done when.** `crab-cage lust-probe` against lust 0.2.0 prints `--time-limit: no`. Before merging, also check a 0.3.0 build prints `--max-memory: yes`; if 0.3.0's usage text does not name the flag, report that instead of working around it (it becomes part of L-04).

## WP-03 Stop emitting `f64::clamp` (T1)

**Problem.** lust rejects the emitted controller: `L0104 no method clamp on type f64`.

**Files.** `src/emit.rs` only.

**Steps.**
1. In the Rust source that `emit` produces, add this function next to `clamp01`:
   ```rust
   fn clamp_range(x: f64, lo: f64, hi: f64) -> f64 {
       if x < lo { lo } else if x > hi { hi } else { x }
   }
   ```
2. Replace every `EXPR.clamp(A, B)` **inside the emitted text** with `clamp_range(EXPR, A, B)`.
3. Do not touch `src/control.rs`. For finite inputs and NaN the two forms return the same value, so the replay test still passes.

**Done when.** `cargo test --locked` passes (the replay test compares the emitted file with the genome to 1e-6), and `lust check --sandbox crab-out/best_controller.rs` reports no error.

## WP-04 A test that uses the real lust (T2)

**Problem.** Every lust test uses a fake. F1–F3 in the strategy were invisible.

**Files.** new `tests/real_lust.rs`, `README.md` (one paragraph), `.github/workflows/ci.yml` (optional job).

**Steps.**
1. Rebuild lust first: in the lust-lang checkout, `pwsh -File scripts/build-tcc.ps1`, set `LUST_TCC_HOME`, `cargo build --release`. `lust --version` must say 0.3.0.
2. The test reads the environment variable `CRAB_LUST_BIN`. If unset, print `skipped: CRAB_LUST_BIN not set` and return (the test passes).
3. If set: run `execute` with `lust_bin` set, `out_dir` a fresh temp directory, default options. Assert it returns `Ok`, `verification.lust_check` starts with `pass`, and the whole call took under 120 s.
4. Add a second test: `lust::probe` returns in under 15 s.
5. README: how to run it (`CRAB_LUST_BIN=... cargo test --test real_lust`).

**Done when.** Both tests pass locally with a real 0.3.0 lust and are skipped in plain `cargo test`.

## WP-05 Command-line flags must win over the config file (T1)

**Problem.** `merge_unset` in `src/main.rs` treats "flag equals default" as "not passed". `--seed 1` loses to `seed = 7` in the file.

**Files.** `src/main.rs`, `tests/cli.rs`.

**Steps.**
1. In `EvolveArgs`, change every field that has a `default_value` to `Option<T>` with no default (`seed: Option<u64>`, `population: Option<usize>`, `backend: Option<String>`, and so on).
2. Resolve each option in this order: flag, then config file, then the built-in default from `RunOptions::default()`.
3. Delete `merge_unset`.
4. A bad value in the file (`max_memory = "lots"`) is an error, not silently ignored.

**Test.** In `tests/cli.rs`: write a config with `seed = 7` and `generations = 3`; run with `--seed 1 --eval native --config <file>`; read `report.json`; assert `seed` is 1.

**Done when.** The new test passes and `crab-cage evolve --help` still lists every flag.

## WP-06 Always run the oracle, and compare (T2)

**Problem.** When lust builds the winner, rustc never runs. Nothing compares the two builds.

**Files.** `src/evolve.rs`, `src/lust.rs`, `src/report.rs`, `tests/fake_lust.rs`.

**Steps.**
1. `lust::compile_rustc`: take the edition as a parameter. Call it with `"2024"`; if rustc says the edition is unknown, retry with `"2021"` and record which was used.
2. In `verify_winner`, remove the early `return Ok(())` after the lust build. Always continue to the rustc build and replay.
3. Record three results in `Verification`: `lust_replay_error: Option<f64>`, `rustc_replay_error: Option<f64>`, `oracle_agrees: Option<bool>`. `oracle_agrees` is true when both replays ran and the two binaries printed identical lines for the same input trace. Capture each binary's reply lines in `replay` and compare the strings, not the parsed floats.
4. If they differ, return an error that names the first differing line and says "lust and rustc disagree; this is a lust bug".
5. Show the three fields in `report.md` and `report.json`. Bump the report `schema` to 2.

**Done when.** With a real lust: the report shows both replays and `oracle_agrees: true`. With no lust: rustc runs alone and `oracle_agrees` is null. Existing tests pass.

---

# Phase 1: the primitive (`crab-cage eval`)

Shared definitions for WP-10 to WP-19. Copy these exactly.

**Nugget directory**

```text
<dir>/nugget.json
<dir>/template.rs
<dir>/cases/<id>.in      one per case, <id> is any file stem
<dir>/cases/<id>.out     required when objective is "pass_all"
```

**`nugget.json`**

```json
{ "name": "sum-squares", "objective": "pass_all", "timeout_ms": 2000, "max_memory": "64M" }
```

- `objective`: `"pass_all"` (score = number of failed cases; 0 is perfect) or `"minimize"` (each run's last stdout line must be `score: <number>`; score = the sum over cases; a missing or non-finite number fails the candidate).
- `timeout_ms`, `max_memory`: optional. Defaults 2000 and `"64M"`.
- **Lower score is better** in both modes.

**`template.rs`**: a complete Rust program with exactly one line that is `// CRAB:BEGIN` and one later line that is `// CRAB:END` (leading spaces allowed). The lines between them are the seed candidate.

**Verdict** (the result of evaluating one candidate; also one line of the journal)

```json
{
  "hash": "9f2c0a1b44e0d7c3",
  "gate": "check",
  "status": "rejected",
  "score": null,
  "passed": 0, "total": 3,
  "diagnostics": [ { "code": "L0110", "message": "...", "line": 4, "col": 9, "fix": "..." } ],
  "ms": { "check": 31, "build": 0, "run": 0, "oracle": 0 },
  "note": ""
}
```

- `gate`: the last gate reached: `prefilter`, `check`, `build`, `run`, `oracle`.
- `status`: `rejected` (stopped at a gate), `scored` (ran all cases, oracle not run), `proven` (oracle agrees), `diverged` (oracle disagrees).
- `diagnostics[].line` is relative to the candidate text: line 1 is the first line after `// CRAB:BEGIN`. A diagnostic outside the region keeps `line: 0`.

## WP-10 Nugget loader and splice (T1)

**Files.** new `src/nugget.rs`, `src/lib.rs` (add `pub mod nugget;`), new `nuggets/sum-squares/` (the demo).

**API.**

```rust
pub struct Case { pub id: String, pub input: String, pub expected: Option<String> }
pub enum Objective { PassAll, Minimize }
pub struct Nugget {
    pub name: String, pub objective: Objective,
    pub timeout: Duration, pub max_memory: u64,
    pub prefix: String,   // template text up to and including the BEGIN line
    pub seed: String,     // text between the markers
    pub suffix: String,   // END line and everything after
    pub region_first_line: usize, // 1-based line number of the first seed line in template.rs
    pub cases: Vec<Case>, // sorted by id
}
pub fn load(dir: &Path) -> Result<Nugget>;
impl Nugget { pub fn splice(&self, candidate: &str) -> String; }
```

**Rules.** `load` fails with a clear message when: a marker is missing or appears twice; there are no cases; `objective` is `pass_all` and a case has no `.out`. Read files with `\r\n` turned into `\n`. `splice` returns `prefix + candidate + "\n" (if candidate does not end with one) + suffix`. Parse `max_memory` with `supervisor::parse_memory`.

**Demo nugget.** `template.rs` reads integers from stdin, one per line, and prints the sum of their squares. The region holds the body of `fn sum_squares(xs: &[i64]) -> i64`; the seed is deliberately wrong (`xs.iter().sum()`). Three cases, with correct `.out` files.

**Tests (in `src/nugget.rs`).** Load the demo; `splice(seed)` equals the original `template.rs`; a template with no `// CRAB:END` is an error.

## WP-11 Keep the whole diagnostic (T1)

**Files.** `src/lust.rs`.

**Steps.** Extend `Diagnostic` with `severity: String`, `line: u32`, `col: u32`, `fix: String`, `auto_edits: usize`. Real lust output looks like this (one JSON object per line):

```json
{"schema":1,"kind":"diagnostic","code":"L0110","severity":"error","message":"mismatched types: expected `u32`, found `&str`","file":"C:\\t\\bad.rs","span":{"start":29,"end":32,"line":2,"col":18,"end_line":2,"end_col":21},"labels":[],"fix":"Change the value to type `u32`, or change the expected type.","edits":[]}
```

Read `line` and `col` from `span`. `auto_edits` is the length of `edits`. Every field is optional in the parser; a missing one becomes empty or 0. The old fake-lust shape (`span.file`, no `severity`) must still parse.

**Tests.** Parse the line above; assert `code == "L0110"`, `line == 2`, `col == 18`, `fix` starts with `Change`. Keep the existing test.

## WP-12 The funnel for one candidate, and `crab-cage eval` (T2)

**Files.** new `src/pipeline.rs`, `src/lib.rs`, `src/main.rs`, new `tests/eval.rs`.

**API.**

```rust
pub struct Toolchain { pub lust: Option<LustTool> }   // None: use rustc for the fast gates too
pub struct EvalOptions { pub work_dir: PathBuf, pub host_memory: u64, pub compile_timeout: Duration, pub oracle: bool }
pub fn evaluate(nugget: &Nugget, candidate: &str, tools: &Toolchain, opts: &EvalOptions) -> Result<Verdict>;
```

**Behaviour, in order.**
1. `hash` = 16 hex digits of FNV-1a 64 over the candidate bytes (write the 6-line function; no dependency).
2. **prefilter**: `emit::sandbox_prefilter(candidate)`. On `Err`, return `rejected` at `prefilter` with the message in `note`.
3. Write `nugget.splice(candidate)` to `<work_dir>/<hash>.rs`.
4. **check** (only when lust is present): `lust::check`. If not ok, return `rejected` at `check`. Convert each diagnostic's line: `line - nugget.region_first_line + 1`; if the result is below 1 or past the candidate's last line, use 0.
5. **build**: `lust::build` with `nugget.max_memory`, or `lust::compile_rustc` when there is no lust. On failure: `rejected` at `build`, error text in `note`.
6. **run**: for each case, `supervisor::run_capture(Command::new(bin), &lust::candidate_limits(nugget.timeout, opts.host_memory), case.input.as_bytes())`. A case fails when `lust::classify` is not `Clean`, or (`pass_all`) stdout differs from `expected` after trimming trailing whitespace on both. For `minimize`, parse the last non-empty stdout line as `score: <f64>`.
7. If any run was not `Clean`: `rejected` at `run`, with the class name in `note`. Otherwise `scored`.
8. If `opts.oracle` is false, stop here. (WP-14 adds the oracle.)
9. Fill `ms` with wall-clock times of each stage.

**CLI.** `crab-cage eval <nugget-dir> <candidate-file> [--lust-bin P] [--work-dir D] [--json]`. With `--json` print the verdict as one JSON line; otherwise print three human lines. `<candidate-file>` of `-` means read stdin. Exit code 0 when status is `scored` or `proven`, 1 otherwise. Default `work-dir` is a fresh directory under `std::env::temp_dir()`.

**Tests (`tests/eval.rs`, no lust needed: `Toolchain { lust: None }`).** On the demo nugget: the seed gives `scored` with `score == 3` (or however many cases fail); the correct body gives `score == 0`; a candidate with a syntax error gives `rejected` at `build`; a candidate containing `std::fs` gives `rejected` at `prefilter`.

**Done when.** The tests pass, and by hand with a real lust: a type error returns `rejected` at `check` with a region-relative line number.

## WP-13 Journal and dedup (T1)

**Files.** new `src/journal.rs`, `src/lib.rs`.

**API.**

```rust
pub struct Journal { /* file handle + HashSet<String> of hashes */ }
impl Journal {
    pub fn open(path: &Path) -> Result<Journal>;       // creates the file; if it exists, loads the hashes already in it
    pub fn seen(&self, hash: &str) -> bool;
    pub fn append(&mut self, verdict: &Verdict, proposer: &str) -> Result<()>; // one JSON line, then flush
    pub fn summary(path: &Path) -> Result<String>;     // see below
}
```

Each line is the verdict object plus `"proposer"` and `"n"` (a running count from 1).

`summary` returns text: total candidates; count per `status`; count per `gate` for rejected ones; the ten most frequent diagnostic `code`s with counts; the best score and its hash.

**Tests.** Append two verdicts, reopen, `seen` is true for both; `summary` on a hand-written three-line journal contains the right counts.

## WP-14 The oracle gate (T2)

**Files.** `src/pipeline.rs`, `tests/eval.rs`.

**Steps.** When `opts.oracle` is true and the status so far is `scored`:
1. Compile the same spliced file with `lust::compile_rustc` (edition handling from WP-06) to `<work_dir>/<hash>.oracle[.exe]`.
2. Run every case with the same limits.
3. For each case compare **exactly**: stdout bytes and exit kind of the fast build against the oracle build.
4. All equal: status `proven`. Any difference: status `diverged`, and write `<work_dir>/divergences/<hash>/` containing `source.rs`, `case.in`, `fast.stdout`, `oracle.stdout`, and `README.txt` with the lust version and both commands.
5. If rustc **rejects** a program lust accepted: also `diverged`, `note` = `lust accepted, rustc rejected`, and save rustc's stderr.
6. When `tools.lust` is `None` the fast build already is rustc: skip the second compile and return `proven`.

Add `--oracle` to `crab-cage eval`.

**Tests.** Without lust: correct body with `oracle: true` gives `proven`. With the fake lust (extend it so `build` produces a binary that prints a fixed wrong line): status is `diverged` and the directory exists.

---

# Phase 2: the loop (`crab-cage run`)

## WP-15 `crab-cage run` with the folder proposer (T2)

**Files.** new `src/proposer.rs`, new `src/run.rs`, `src/lib.rs`, `src/main.rs`, new `tests/run.rs`.

**Trait.**

```rust
pub struct Feedback<'a> { pub best: Option<(&'a str, f64)>, pub last: Option<(&'a str, &'a Verdict)> }
pub trait Proposer {
    fn name(&self) -> &str;
    /// The next candidate text, or None when this proposer has nothing more.
    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Option<String>>;
}
```

**FolderProposer.** Given a directory, returns the contents of each `*.rs` file once, in name order. It re-lists the directory on each call, so files added while the run is going are picked up.

**The run loop (`run::execute`).**
1. Open the journal at `<out>/journal.jsonl`. Evaluate the seed first (proposer name `seed`).
2. Repeat until `--budget N` candidates were evaluated (default 200) or every proposer returned `None`: ask the next proposer in round-robin; skip a candidate whose hash the journal has seen; `evaluate` with `oracle: false`; append to the journal.
3. Keep `best` = the lowest score among `scored` candidates.
4. Whenever a candidate becomes the new best, run the oracle on it (WP-14). `proven` → write `<out>/best.rs` (the spliced program) and `<out>/best.region.rs`. `diverged` → it does not count as best; keep going.
5. Stop early when a proven candidate has score 0 and the objective is `pass_all`.
6. Print one line per candidate (`n`, status, gate, score, proposer) and at the end the journal summary.

**CLI.** `crab-cage run <nugget-dir> [--candidates DIR] [--budget N] [--out DIR] [--lust-bin P]`. Exit 0 when a proven candidate exists.

**Test.** The demo nugget, a temp folder with one wrong and one right snippet, no lust: exit result is proven, `best.rs` exists, the journal has 3 lines.

## WP-16 Literal-tweak proposer (T1)

**Files.** `src/proposer.rs`.

**Behaviour.** `LiteralProposer::new(seed: u64)`. On `next`: take the best candidate text (or the nugget seed); find numeric literals with a hand-written scanner (a run of digits, optionally `.` plus digits, optionally an exponent; not preceded by a letter, digit, `_` or `.`; skip everything inside `"…"` strings and after `//`). If there are none, return `None`. Pick one with `rng::Pcg64`; float literal: multiply by `1 + gaussian * 0.2`, print with `genome::rust_f64`; integer literal: add −2…+2 except 0, never below 0. Return the text with that one literal replaced.

**Tests.** `"let k = 0.5; // 7"` changes only `0.5`; `"x1 + \"3\""` has no literals; the same seed gives the same sequence twice.

Add `--tweak` to `crab-cage run` to enable it.

## WP-17 Parallel slots (T2)

**Files.** `src/run.rs`.

**Steps.** Add `--jobs N` (default 1). Create `N` work directories `<out>/slot-<i>/`. Gather up to `N` unseen candidates from the proposers, evaluate them on `N` `std::thread::scope` threads (one slot each), then append the verdicts to the journal **in the order the candidates were proposed** so a run is reproducible. Best-tracking and the oracle stay on the main thread.

**First, measure.** Before writing code, time 40 evaluations of the demo nugget with `--jobs 1`. Record check / build / run milliseconds from the journal in the pull request. After: the same with `--jobs 4`. If the speed-up is under 1.5×, say so; the bottleneck is then in lust (see L-07).

## WP-18 Local-model proposer (T2)

**Files.** `src/proposer.rs`, `src/llm.rs` (reuse `chat`; make it `pub(crate)`), `src/run.rs`, `src/main.rs`.

**Behaviour.** `ModelProposer` holds an endpoint, a model name, and the text of `lust capabilities --summary` (run once at start; empty when lust is absent). On `next` it sends:

- *system*: "You write the body of one region of a Rust program. Reply with only the new region text, no markdown fence, no explanation. Stay inside this subset of Rust:" + the capability summary.
- *user*: the template with the current best region in place; the objective in one sentence; and, if the last verdict from this proposer was `rejected`, its diagnostics as `line L, col C: CODE message. Hint: fix` (at most 10), or its `note`.

Strip a leading/trailing markdown fence from the reply (`strip_fences` exists). Reject a reply that contains `// CRAB:BEGIN` or `// CRAB:END`. Endpoint validation is `normalize_endpoint`, unchanged: loopback only.

After three rejected replies in a row, send the nugget seed as the base instead of the best (a reset).

**CLI.** `crab-cage run … --backend local --endpoint URL --model NAME`.

**Test.** A `MockModelProposer` used in tests returns a fixed list of strings; assert that the second prompt contains the first verdict's diagnostic code. No network in tests.

**Then delete** `LlmBackend::repair`, `RepairRequest`, `SYSTEM_REPAIR` and their test: they are unused (strategy F6).

## WP-19 Docs, and (optional) the tank as a nugget (T1 docs, T3 port)

**Docs (T1).** Rewrite the first screen of `README.md` and the "The job" and "Roadmap" sections of `docs/DESIGN.md` around the funnel in STRATEGY.md section 5. Keep the tank demo as "Example: controller search". Document `eval` and `run`.

**Port (T3, optional).** Add an "interactive" case kind where the host drives the candidate line by line (the existing `Session` and `fitness::evaluate_exchange`), then express the tank as `nuggets/tank/`. Only worth doing once someone needs a second closed-loop problem.

---

# Phase 3: lust-lang

Work in the lust-lang checkout. Its own rules apply (`AGENTS.md`): trunk is `next`; the gate is `bash scripts/ci.sh`; a new accepted program gets a case in `tests/pass`, a new reject in `tests/reject`; rule zero is absolute.

## L-01 The daemon must not hold the caller's pipes (T3)

**Symptom.** A host that runs `lust check file.rs` with stdout/stderr captured through pipes, on Windows, never sees end-of-file while the auto-started daemon lives. Repro: from PowerShell, in an empty directory with no daemon running, `$p = Start-Process lust.exe -ArgumentList 'check','ok.rs' -RedirectStandardOutput o.txt -PassThru` is not enough (files are not pipes); use a 10-line Rust or Python program that spawns `lust check ok.rs` with piped stdout and reads to end. It blocks until `.lust-daemon-*.sock` is deleted.

**Fix direction.** Where the client spawns the daemon (`src/client.rs`, the Windows detach path), start it with handle inheritance off for everything except the lock handle it passes on purpose, and with stdin/stdout/stderr set to `NUL` or the daemon log. Confirm first that this is the cause; if it is not, report what is.

**Test.** Add the piped-reader repro to `scripts/test-client.sh` with a 10 s limit.

## L-02 `f64::clamp` and `f32::clamp` (T2)

lust rejects `x.clamp(0.0, 1.0)` on floats with L0104; integers and `Ord::clamp` already work. Add `clamp` to the float method table (`FLOAT_METHODS` and its lowering in `src/check/mod.rs`; follow how `max`/`min` are done). rustc panics when `min > max` or either bound is NaN; match that message byte for byte, or keep rejecting the cases you cannot match. Add `tests/pass` cases: in range, below, above, NaN input, and a `should panic` case for `min > max`.

## L-03 `--time-limit` (T2 to rebase, T3 if it conflicts)

The implementation exists as `crab-cage/docs/lust-time-limit.patch`, written against `cbc913832b6f`. Rebase it onto `next`, run the gate, add the `L0533` doc under `docs/errors/`, and update `docs/sandbox.md` ("It does not limit time" becomes false).

## L-04 Machine-readable interface probe (T2)

Hosts currently grep the usage text. Add to `lust capabilities --json` a top-level object: `{"version":"0.3.0","build":"…","flags":{"check":["--json","--sandbox",…],"build":[…],"run":[…]},"commands":["check","build","run","test","fix",…]}`. Add `lust capabilities --sandbox --summary`: the same summary limited to what `--sandbox` accepts. Extend `scripts/test-capabilities.py`. Then make the usage text name every flag `build` accepts (WP-02 depends on it until Crab Cage switches to the JSON).

## L-05 Structured run result (T2)

Opt-in flag on `build`/`run` (name it `--report-stop`): when the program stops, the runtime writes one JSON line to stderr, last: `{"lust_stop":{"class":"exit|panic|sandbox|memory|time","code":101,"family":"fs","peak_bytes":123}}`. Off by default, so rule zero holds for normal builds. Crab Cage then replaces `lust::classify`'s text matching.

## L-06 `lust fix --sandbox` (T2)

`fix` should accept the same profile flags as `check`, so the re-check after applying edits uses the sandbox rules. Extend `scripts/test-fix.sh`.

## L-07 Per-call cost on Windows (T3; measure first)

On Windows a warm `lust check` of a 9-line file cost tens of milliseconds end to end in a shell loop, against 7.5 ms documented on Linux. Step 1: measure properly. A small Rust program that calls `lust check --json --timings` 200 times on alternating edits, reporting client wall time and the daemon's own time. Step 2, only if the client side dominates: add `lust check --batch`, which reads file paths from stdin, one per line, and writes one JSON result line per path, over one daemon connection. Publish the Windows numbers in `docs/speed.md` either way.

## L-08 Turn Crab Cage journals into lust cases (T1 script, T2 triage)

A script `scripts/crab-intake.py <crab-out-dir>`:
- for each `divergences/<hash>/`: copy `source.rs` to `tests/crab/diverged/<hash>.rs` and print the lust and rustc outputs side by side. Each one is a rule-zero bug to fix or to turn into a reject.
- from `journal.jsonl`: print the L-codes ranked by how many candidates each one rejected, with one example source per code. This is the "what should lust support next" list, measured on what models actually write.
