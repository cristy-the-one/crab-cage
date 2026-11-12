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
9. Tests in one file run in parallel, so each test must use its own output path, or build shared helpers once (as `fake_lust()` in `tests/fake_lust.rs` does, with `std::sync::OnceLock`).

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
Phase 2:  WP-15 → WP-16 → WP-17 → WP-18 → WP-19;  WP-20 any time after WP-14
Phase 3:  L-03 … L-11, any order
```

## Status (2026-11-11)

| Item | State | Where |
|---|---|---|
| WP-01 to WP-06 | done | crab-cage [#2](https://github.com/cristy-the-one/crab-cage/pull/2); WP-01 step B in [#5](https://github.com/cristy-the-one/crab-cage/pull/5) |
| WP-10 to WP-14 | done | crab-cage #4, #6, #7, #8, #9 |
| WP-15 | done | crab-cage [#10](https://github.com/cristy-the-one/crab-cage/pull/10) |
| Warm lust daemon (`eval --lust-daemon`, default in `run`) | done, not in the original plan | crab-cage #11, #12 |
| WP-16 | done | crab-cage #14 |
| WP-17 | done | crab-cage #16; see WP-17 for the numbers |
| WP-17b | done | crab-cage #16, `src/spawn.rs`; numbers below WP-17 |
| WP-18 | done | crab-cage #15; the proposer has run against a mock and a scripted stand-in server, not yet a real model |
| WP-18b | done | crab-cage #18: an "already tried" list, the failing case, verdicts by hash |
| The model prompt, tuned against a real model | done | see "First run against a real model" below |
| The model ladder, an experiment to run on a GPU machine | ready, not yet run there | `experiments/model-ladder/`; two medium nuggets added for it |
| WP-19 | docs done; the tank port is not planned | README and DESIGN.md |
| WP-20 | open | needs zig |
| L-01 | done for lust's own standard handles | lust-lang [#38](https://github.com/cristy-the-one/lust-lang/pull/38) |
| L-02 | done for literal bounds; variable bounds open | lust-lang #37 |
| L-03 | done | lust-lang #36 |
| L-04, L-05, L-06, L-08, L-09 | open | |
| L-07 | measured; see the package | |
| L-10 | in review | lust-lang [#39](https://github.com/cristy-the-one/lust-lang/pull/39); it also closes the same leak for file descriptors on Unix |
| L-11 | new, open | below |

## First run against a real model (2026-11-11)

Ollama 0.40 on the work laptop (Ryzen 7 8840HS, no discrete GPU, CPU inference), model `Thox-ai/thoxcode-8b` (Qwen family, 7.6B, Q4_K_M), demo nugget, `--jobs 1`. About 2 s per reply with a 1,500-token prompt.

- Asked fresh, the model wrote the whole correct region every time, with the first wording and with the current one.
- One run began with a fragment (`.map(|x| x * x)`, a continuation of the old line). After that rejection the first wording got a fragment back every time, and the proposer gave up after two candidates.
- Two changes fixed it, each measured by replaying the exact prompt eight to ten times: the system prompt says the reply *replaces* the lines and must never be a fragment, and the user prompt shows the current lines and puts the request **last**, after the diagnostics and the "already tried" list. With the request first, 8 of 8 replies were fragments; with it last, 8 of 8 were the whole region.
- In the loop: 10 of 10 plain runs solved in one request (about 5 s for the run); 6 of 6 runs whose first reply was forced to be a fragment solved in two.

What this does not show: a `minimize` nugget, any other model, or `--jobs` above 1.

## A harder nugget against the same model (2026-11-11)

`nuggets/normalize-path`: normalize a Unix path as text (repeated `/`, `.`, `..`, the root, relative paths that climb, nothing left). Ten cases; the rules are in a comment above the function; `reference.rs` passes all ten under lust and under `rustc`. The seed fails eight.

`crab-cage run nuggets/normalize-path --backend local --model Thox-ai/thoxcode-8b --budget 16 --lust-bin …`:

```text
1 proven oracle 8 seed
2 proven oracle 6 local
3 scored run 6 local
4 scored run 7 local
5 scored run 7 local
6 scored run 7 local
local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried
```

19 requests, 320 s, about 17 s each (2,000 to 2,500 prompt tokens, about 120 reply tokens, CPU only). Not solved.

What worked: every candidate the model wrote passed `lust check` and built (iterators, `peekable`, `Vec<&str>`, `join`, `format!`), so lust's subset was not the obstacle; the failing case reached the prompt with its input, expected and actual output; a repeat cost one retry and not fifty requests.

What did not: the model kept changing how it handles `..` and never the thing the failing case showed (`a//b` must become `a/b`). Replaying that state with other prompts, four to eight times each, scored through the funnel:

| Prompt | Result |
|---|---|
| as sent | failed 6 or 7 of 10, every time |
| the failing case moved to the end, a two-line "already tried" list | the same |
| "begin with a comment that says what the lines do wrong", then the code | the comment is right ("do not handle consecutive slashes"), the code skips empty components, and the score gets **worse** (9 of 10): absolute paths passed before only because the empty first component produced the leading `/` |
| the same, but each step starts from the model's own last reply instead of the best (8 steps, twice) | 6 → 9 → 10 → 10 …; never recovers |

So on this nugget the limit is the model, not the layout of the prompt. Three things the run shows about the tool, none built yet:

- **The fix needs two changes at once** (skip empty components, and put the leading `/` back). The loop keeps only candidates that beat the best, so the half-way state is thrown away, and a small model does not make both changes in one reply.
- **The "already tried" list is long and says little** when the regions are 500 characters that differ in one line: three entries cut at 400 characters each look the same. Listing only the lines that differ from the current region would be shorter and clearer.
- **The feedback and the base can disagree.** The prompt shows the best region as "now" and the failing case of the proposer's *last* region. Here both failed the same case; in general the case shown should be the one the shown region fails.

Options from here, in the order I would try them: a stronger model for this proposer (the 14B build, or the other machine's GPU); candidates from an outside model through `--candidates` or `crab-cage eval`, which need no change; then the two prompt points above.

Also seen: `crab-cage run … | head -1` panics with `failed printing to stdout` when the reader closes the pipe early. Harmless, and worth a quiet exit.

Decisions made while building, which later packages must keep:

- **Exit code of `crab-cage run`.** A `pass_all` nugget exits 0 only when a proven candidate has score 0. A `minimize` nugget exits 0 when any candidate is proven.
- **Only `proven` is a result.** `diverged`, and `rejected` with the note `nondeterministic output`, never become the best.
- **The oracle matches lust's profile.** rustc is run with `-C debug-assertions=on -C overflow-checks=on`.
- **The daemon needs one file.** With lust's daemon on, lust always compiles `<work>/slot.rs`. A new file name per candidate is slower than no daemon at all.
- **The hash.** `pipeline::candidate_hash` is the only hash function. Do not copy it.
- **The model prompt ends with the request.** Feedback and the "already tried" list come before it. Changing the order or the wording needs a replay against a real model, not only the mock.

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

**Done when.** `crab-cage lust-probe` against lust 0.2.0 prints `--time-limit: no`.

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

**Files.** `src/evolve.rs`, `src/lust.rs`, `src/report.rs`, `tests/fake_lust.rs`, `tests/replay.rs` (one call site only).

**Steps.**
1. `lust::compile_rustc`: add an `edition: &str` parameter, placed after `output`. In `src/evolve.rs` call it with `"2024"`; if rustc's stderr says the edition is unknown or invalid, retry with `"2021"`. Record the edition that worked in a new field `Verification.rustc_edition: Option<String>`. In `tests/replay.rs` line 31, change the existing call to pass `"2021"` and change nothing else in that file.
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

**Since WP-15.** This is the first proposer that can repeat a candidate. `run::execute` stops after 50 proposals in a row that leave no journal line. Add a test on a nugget with one literal: the run must end with that stop line, not spin.

## WP-17 Parallel slots (T2)

**Files.** `src/run.rs`.

**Steps.** Add `--jobs N` (default 1). Gather up to `N` unseen candidates from the proposers, evaluate them on `N` `std::thread::scope` threads, then append the verdicts to the journal **in the order the candidates were proposed** so a run is reproducible. Best-tracking and the oracle stay on the main thread.

**Slots, given the daemon.** lust starts one daemon per directory it is started in, and keeps one warm session per source path. So use **one** work directory and `N` slot files in it (`slot-0.rs` … `slot-<N-1>.rs`), one per thread: one daemon, `N` warm sessions. `N` directories would mean `N` daemons of about 120 MB each. `pipeline::evaluate` takes the slot file name from its options instead of the constant `SLOT_FILE`.

**Measured before this package (40 candidates, demo nugget, Windows, daemon on).** Median per candidate: check 38 ms, build 73 ms, running the three cases 256 ms. Running the cases is the largest part, and it is plain process start-up, so that is what `--jobs` has to spread. Record the same three numbers with `--jobs 4` in the pull request. If the whole run is not at least 1.5× faster, say so.

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

## WP-20 Speed as an objective, scored with the zig build (T2)

**Why.** lust's normal build uses TinyCC, which does not optimise, so its run times say little about how the rustc build will perform. lust already has `--opt`, which compiles the same generated C with `zig cc -O2` (or `cc`) and accepts `--sandbox`. Use it to score candidates whose goal is to run faster.

**Needs.** `zig` on `PATH` (or `cc`, or `LUST_CC`). Without it this package degrades as described in step 4; it never fails the run.

**Files.** `src/nugget.rs`, `src/lust.rs`, `src/pipeline.rs`, `tests/eval.rs`, new `nuggets/fast-sum/`.

**Steps.**
1. `nugget.json` accepts a third objective, `"fastest"`. Every case needs a `.out` file, as for `pass_all`.
2. `lust::build` takes one more argument, `opt: bool`. When true, add `--opt` to the arguments.
3. In `evaluate`, for `fastest`: first do the normal fast build and run every case once. If any case fails, the verdict is as for `pass_all` (`scored`, score = 1e9 + number of failed cases, so a wrong candidate never beats a correct one). If all pass: build again with `opt: true` to `<work_dir>/<hash>.opt[.exe]`, run every case three times, and score = the sum over cases of the **median** elapsed milliseconds. Outputs of the opt build must also match `.out`; if one does not, the status is `diverged`.
4. If the opt build fails because no C compiler was found (lust's error text names `zig` or `cc`), time the fast build instead and put `opt build unavailable, timed the tcc build` in `note`. With `Toolchain { lust: None }`, time the rustc build (`opt-level=2`, already what `compile_rustc` passes).
5. Add `"opt"` to the verdict's `ms` object.
6. The oracle (WP-14) is unchanged: it compares outputs, never times.

**Demo nugget.** `nuggets/fast-sum/`: sums the squares of 1…N for N read from stdin; the seed uses a deliberately slow double loop; a closed-form body is the intended win.

**Tests (no lust needed).** The seed scores higher (slower) than the closed-form body; a wrong body scores at least 1e9.

**Done when.** With a real lust and zig installed, `crab-cage eval nuggets/fast-sum cand.rs --json` shows a non-zero `ms.opt` and an empty `note`; with zig absent, the note from step 4 appears and the exit code is still 0.

---

**Result (40 candidates, demo nugget, Windows 11, daemon on, back to back).**

| `--jobs` | Whole run | check | build | run 3 cases |
|---:|---:|---:|---:|---:|
| 1 | 11.7 s | 25 ms | 38 ms | 165 ms |
| 2 | 10.0 s | 27 ms | 52 ms | 280 ms |
| 4 | 9.8 s | 33 ms | 75 ms | 510 ms |
| 8 | 11.4 s | 46 ms | 144 ms | 1012 ms |

The journals hold the same decisions in the same order for all four. The run is not faster: the time to run the cases grows with the number of jobs. The cause is WP-17b.

## WP-17b Start candidates without the standard library's process lock (T3, Windows)

**Measured.** 24 starts of one program from Rust threads, Windows 11:

| | 1 thread | 2 | 4 | 8 |
|---|---:|---:|---:|---:|
| a program that has run before | 297 ms | 128 ms | 99 ms | 99 ms |
| 24 copies that have never run | 976 ms | 1001 ms | 1024 ms | 1023 ms |

The first start of a new executable costs about 40 ms (the system inspects the file), and from Rust threads those starts do not overlap at all. `std::process::Command::spawn` on Windows holds one process-wide lock for the whole `CreateProcessW` call, so that a child cannot inherit a handle meant for another child. Every candidate is a new executable, so every candidate pays 40 ms under that lock, and every other spawn (lust, rustc) waits behind it. The same loop driven from Python, which has no such lock, went from 270 ms to 72 ms per candidate with four jobs.

**Fix.** In `src/supervisor.rs`, on Windows, start children with a direct `CreateProcessW` call and `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` naming only the three pipe ends for that child. With a handle list no lock is needed, because a child can only get the handles named for it. This also replaces `keep_std_handles_private`. It needs: pipe creation, the command line built with Windows quoting rules, the environment block (the parent's, minus what `harden_env` removes), a small type in place of `std::process::Child` (wait with a timeout, kill, exit code, the raw handle for the job object), and the same for `Session`. lust-lang's `protocol::spawn_daemon` is a working example of the call. Unix is untouched.

**Done when.** The table above shows the run 3 cases column staying near its `--jobs 1` value, and the whole run at least 2× faster with `--jobs 4`. All of `tests/limits.rs` passes unchanged.

**Result (same 40 candidates, same machine, after the change).**

| `--jobs` | Whole run | check | build | run 3 cases |
|---:|---:|---:|---:|---:|
| 1 | 11.1 s | 27 ms | 50 ms | 137 ms |
| 2 | 7.5 s | 28 ms | 63 ms | 134 ms |
| 4 | 6.8 s | 32 ms | 93 ms | 136 ms |
| 8 | 8.7 s | 59 ms | 156 ms | 141 ms |

The run column is flat now: candidate starts overlap. The whole run is 1.6× faster with four jobs, not the 2× asked for. About 2.4 s of every run is fixed (the probe, the daemon's cold start, the seed and its oracle), and the time lust takes to build grows as builds overlap (50 ms alone, 93 ms with four, 156 ms with eight), which is on the lust side. Without the daemon the same run goes from 33.8 s to 15.2 s with four jobs (2.2×). Same decisions in the same order for 1, 2, 4 and 8. `tests/limits.rs` passes unchanged, with one test added.

# Phase 3: lust-lang

Work in the lust-lang checkout. Its own rules apply (`AGENTS.md`): trunk is `next`; the gate is `bash scripts/ci.sh`; a new accepted program gets a case in `tests/pass`, a new reject in `tests/reject`; rule zero is absolute.

## L-01 The daemon must not hold the caller's pipes (T3)

**Symptom.** A host that runs `lust check file.rs` with stdout/stderr captured through pipes, on Windows, never sees end-of-file while the auto-started daemon lives. Repro: from PowerShell, in an empty directory with no daemon running, `$p = Start-Process lust.exe -ArgumentList 'check','ok.rs' -RedirectStandardOutput o.txt -PassThru` is not enough (files are not pipes); use a 10-line Rust or Python program that spawns `lust check ok.rs` with piped stdout and reads to end. It blocks until `.lust-daemon-*.sock` is deleted.

**Fix direction.** Where the client spawns the daemon (`src/client.rs`, the Windows detach path), start it with handle inheritance off for everything except the lock handle it passes on purpose, and with stdin/stdout/stderr set to `NUL` or the daemon log. Confirm first that this is the cause; if it is not, report what is.

**Test.** Add the piped-reader repro to `scripts/test-client.sh` with a 10 s limit.

**Done in lust-lang #38**, for the client's own stdin, stdout and stderr. Confirmed cause: a child gets a copy of every inheritable handle of its parent. What #38 does not cover is L-10.

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

**Measured (Windows 11, lust `next` at `f2dc9a8b`, demo nugget, medians).**

| Setup | `lust check` | `lust build` |
|---|---:|---:|
| daemon off | 108 ms | 378 ms |
| daemon on, a new file per candidate | 97 ms | 630 ms |
| daemon on, one file rewritten | 18 ms | 35 ms |

So step 2 (`--batch`) is not needed for now: a host that rewrites one file gets most of the gain, and Crab Cage does. What is left of the 18 ms is mostly starting the client process. Still open: publish Windows numbers in lust's `docs/speed.md`, and say in lust's docs that a host should reuse one source path.

## L-08 Turn Crab Cage journals into lust cases (T1 script, T2 triage)

A script `scripts/crab-intake.py <crab-out-dir>`:
- for each `divergences/<hash>/`: copy `source.rs` to `tests/crab/diverged/<hash>.rs` and print the lust and rustc outputs side by side. Each one is a rule-zero bug to fix or to turn into a reject.
- from `journal.jsonl`: print the L-codes ranked by how many candidates each one rejected, with one example source per code. This is the "what should lust support next" list, measured on what models actually write.

## L-09 Make the zig path a first-class, visible option (T2)

`--opt` already uses `zig cc` when it is on `PATH`. A host cannot tell in advance whether it will work, and there are no Windows numbers for it.

1. `lust capabilities --json` (the object from L-04) gains `"opt": {"available": true, "compiler": "zig cc", "version": "…"}`; `available` is false when neither `zig`, `cc` nor `LUST_CC` resolves.
2. When `--opt` is asked for and no compiler is found, the error names what was looked for and in what order, with its own L-code, so a host can match the code instead of the text (WP-20 step 4 then switches to it).
3. Measure on Windows and add to `docs/opt.md`: build time and run time of the same program under tcc, `--opt` with zig, and `rustc -O`, for the 80-hex bench the doc already uses.
4. Out of scope: replacing TinyCC as the dev backend. TinyCC is linked in and compiles in-process; that is where the millisecond edit loop comes from.

## L-10 The daemon inherits only the handles it is given (T3)

**Symptom.** With lust's daemon on, `crab-cage eval … | reader` finished and the reader never saw end-of-file. Crab Cage's own stdout was an inheritable handle; lust inherited it as an extra handle and passed it to the daemon. #38 makes only lust's own three standard handles private while the daemon starts, so any other inheritable handle of the caller still reaches the daemon.

**Fix.** Start the daemon with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, naming exactly its stdin (NUL), stdout and stderr (the log, or NUL) and the lock handle. std has that only behind an unstable feature (`windows_process_extensions_raw_attribute`), so it is a direct `CreateProcessW` call in `src/protocol.rs`.

**Test.** In `scripts/test-client.sh`: a caller that holds an extra inheritable pipe, starts `lust check` with handle inheritance on, closes its own write end, and must then read end-of-file from the pipe within a bound.

Crab Cage does not depend on this: since crab-cage #11 it keeps its own standard handles out of its children. Another host that embeds lust does.

## L-11 Decide what `HashMap` iteration order means under rule zero (T2 to document, T3 to change)

A program that folds over `HashMap::iter()` printed three different results in three runs of the same rustc binary, and the same result every time from the lust build. rustc randomises the hash seed per process; lust does not. No single output can match a randomised one, so this is not a miscompile, but a lust program can come to depend on an order that the rustc release build will not keep.

Options: say so in lust's docs and keep it; or randomise the seed as rustc does; or reject order-dependent uses (hard). Crab Cage's oracle already handles the candidate side: it runs the rustc binary twice on a mismatch and rejects the candidate as nondeterministic instead of filing a divergence.

