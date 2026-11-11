# Crab Cage: review and strategy

Date: 2026-11-11. Reviewed: crab-cage `3389031` (main), lust-lang `31e1d4da` (next, source 0.3.0), the locally built `lust.exe` (0.2.0, build `9d1c4d3a`, older than its source), rustc 1.98.1, Windows 11.

The work packages that implement this strategy are in [WORK-PACKAGES.md](WORK-PACKAGES.md). They are written to be handed, one at a time, to a weaker model.

## 1. The mission this review measures against

- **rustc is the oracle.** Slow, always right.
- **lust is the fast validator and compiler** for the subset of Rust it supports. Everything it accepts is valid Rust with the same output.
- **Crab Cage is the loop.** It tries many small changes ("nuggets" of code), throws most away in milliseconds with lust, and proves the few survivors with rustc.

So the shape is a funnel: many candidates in, cheap gates first, the expensive gate last and rarely.

## 2. What the repo is today

One Rust crate, 5,600 lines, 3 dependencies (clap, serde, serde_json), 24 tests that pass on this machine. It evolves a PI/feedforward controller for one built-in simulator (a jacketed tank).

| Part | Lines | Reusable for the mission? |
|---|---:|---|
| `supervisor.rs` (wall clock, memory cap, env scrub, Windows job object) | 628 | Yes. The best code in the repo. |
| `lust.rs` (probe, check, fix, build, classify) | 409 | Yes, after fixes (section 3). |
| `http.rs`, transport half of `llm.rs` | ~400 | Yes. |
| `rng.rs`, `error.rs` | 120 | Yes. |
| `plant.rs`, `control.rs`, `genome.rs`, `emit.rs`, `fitness.rs`, `ga.rs`, `report.rs`, `models.rs`, most of `evolve.rs` | ~2,400 | No. Tank only. |

## 3. Findings

Ordered by how much each one blocks the mission. F1 to F3 were observed by running the tool, not by reading it.

**F1. It hangs against a real lust on Windows.** `crab-cage lust-probe --lust-bin <real lust.exe>` did not return in 8 minutes (its own timeout is 10 s). Each lust call that auto-starts lust's daemon blocks Crab Cage until that daemon dies; deleting the daemon's `.sock` file unblocked one call each time. Most likely cause: the daemon inherits the stdout/stderr pipe, so the reader in `supervisor::run_capture` never sees end-of-file. The tests never see this because they use a fake lust. The same runs also leave `.lust-daemon-*.sock` and `.lust-cache/` in the directory Crab Cage was started from.

**F2. Feature detection is wrong.** The probe reported `--time-limit: yes` for lust 0.2.0, which has no such flag. `lust build --time-limit` prints `error: cannot read ...\--time-limit`, and the probe only looks for the substring. Crab Cage would then pass a flag lust reads as a file name.

**F3. The flagship demo is rejected by lust.** With the daemon out of the way, `crab-cage evolve --lust-bin ...` ends with `L0104 no method clamp on type f64`. The emitter uses `f64::clamp`; lust does not have it, in the 0.2.0 binary or in current source (`FLOAT_METHODS` in `src/check/mod.rs`).

**F4. lust is not in the search loop.** Every candidate is scored in-process by interpreting the genome. lust sees one file, the winner, at the end. `--eval sandbox` runs the same native search and re-scores only the winner (`evolve.rs`, `search_sandbox`). The "fast validator" half of the mission is not implemented.

**F5. The oracle is skipped exactly when lust is used.** In `verify_winner`, if lust builds the winner the function returns before `compile_rustc`. rustc runs only when lust is missing. Nothing ever compares a lust build with a rustc build, so a lust miscompile would pass unnoticed. Also: Crab Cage calls rustc with `--edition 2021`, while lust defines itself against edition 2024.

**F6. The model never writes code.** It picks one of six enum values and some numbers. `LlmBackend::repair` (source in, source out) is never called. `MutationRequest.diagnostics` is always the empty string. A candidate is 17 floats, not a code tweak.

**F7. lust's answers are mostly discarded.** `Diagnostic` keeps `code` and `message` and drops `span`, `fix`, `edits`, `severity`. Run outcomes are classified by matching English in stderr. `build` and `fix` force `LUST_NO_DAEMON=1`, which gives up the warm path lust exists for.

**F8. Smaller defects.**
- Config merge treats "flag equals the default" as "flag not passed", so `--seed 1` on the command line loses to `seed = 7` in the file.
- No record of what was tried. No candidate log, no dedup, no resume.
- Candidates are evaluated one at a time.
- `cargo build` warns about an unneeded `unsafe` block at `supervisor.rs:472`.

**What is good and should stay:** the supervisor; the loopback-only model policy; the 3-dependency budget and `cargo deny`; determinism from a fixed seed; the fake-lust test technique; the honest "gaps in lust" list at the end of `DESIGN.md`.

## 4. Tech fit

| Choice | Verdict |
|---|---|
| Rust, one crate, std only plus clap/serde | Keep. Right size. The host must run on Windows and Linux with no runtime. |
| lust as an external binary, not a library | Keep. lust's CLI and JSON are its declared interface (`AGENTS.md`, "Who depends on this"). |
| Typed genome plus a Rust emitter | Wrong unit for the mission. Replace with "text of a marked region in a template file". No parser needed. Do not add `syn`. |
| Genetic algorithm as the core | Demote. It is one proposer among several, useful for numeric literals. |
| In-process scoring | Wrong for the mission (it is why lust is not in the loop). Scoring must come from running the compiled candidate. |
| Hand-rolled HTTP client, loopback only | Keep. An outside model (Grok, Claude, anything) does not need network access from Crab Cage: it can call `crab-cage eval` as a tool or drop files in a folder (WP-12, WP-15). |
| Hand-rolled TOML-like config | Keep the format, fix the merge (WP-05). Use JSON for the new nugget manifest so no new parser is written. |

### Speed on this machine (context for the funnel)

Measured with the local lust 0.2.0, a 9-line program, through a Git Bash loop that also runs `sed` each iteration, so absolute numbers are inflated by process start-up:

| Step | Per iteration |
|---|---:|
| edit + `lust check --json --sandbox`, warm daemon | ~74 ms |
| edit + `lust build --sandbox` + run | ~300 ms |
| `lust build`, `LUST_NO_DAEMON=1` | ~360 ms |
| `rustc --edition 2021` debug build | ~680 ms |

lust's own docs report 7.5 ms for a warm check on Linux. On Windows the per-call cost is dominated by starting processes, not by checking. That is a lust-side opportunity (L-07) and the reason parallel slots matter (WP-17). Do not quote these as benchmarks; WP-04 and L-07 produce real ones.

## 5. Target design

### The nugget

A directory. No Rust knowledge is needed to make one.

```text
nuggets/<name>/
  nugget.json      name, objective, timeout_ms, max_memory
  template.rs      a complete program with one region between
                   `// CRAB:BEGIN` and `// CRAB:END`
  cases/01.in      stdin for case 1
  cases/01.out     expected stdout for case 1
```

A **candidate** is replacement text for the region. The text already between the markers is the seed.

### The funnel

```text
proposer ──► G0 dedup + prefilter      microseconds   host
         ──► G1 lust check --sandbox   milliseconds   diagnostics go back to the proposer
         ──► G2 lust build + run cases ~0.1–0.3 s     score
         ──► G3 rustc build + run      seconds        only for candidates that beat the best
                └─ stdout and exit code must equal G2's, byte for byte
                   equal     → PROVEN
                   different → DIVERGED: saved as a lust bug report
```

Every candidate, at whatever gate it stops, is one line in `journal.jsonl`.

### Proposers (all produce region text)

1. **Folder.** Reads `*.rs` snippets from a directory. Any outside model or script can feed it.
2. **Literal tweak.** Perturbs numeric literals in the region. No model needed. This is the GA, generalised.
3. **Local model.** Gets the region, the diagnostics with line numbers, and lust's capability summary; returns a new region.

### Two outputs, not one

- **For the user:** a proven candidate and a journal.
- **For lust:** every `DIVERGED` case is a rule-zero bug with a ready repro, and the histogram of L-codes at G1 says which missing features cost the most candidates. Crab Cage becomes lust's fuzzer and its roadmap input. This is why lust-lang is part of the plan and not just a dependency.

## 6. The plan

Four phases. Each phase leaves the repo working and more useful than before. Stop after any phase and nothing is half-built.

| Phase | Goal | Packages | Done when |
|---|---|---|---|
| 0. Make it true | The existing tool works against a real lust on Windows | WP-01 … WP-06, L-01, L-02 | `crab-cage evolve --lust-bin <real lust>` finishes, lust and rustc both run on the winner, their outputs are compared |
| 1. The primitive | One candidate through the funnel, as a CLI call any agent can use | WP-10 … WP-14 | `crab-cage eval nuggets/demo cand.rs --json` prints a verdict |
| 2. The loop | Many candidates, journal, model in the loop | WP-15 … WP-19 | `crab-cage run nuggets/demo` ends in a `PROVEN` candidate or says none was found |
| 3. Feed lust | Close the gaps the loop exposes | L-03 … L-08 | Divergences and top reject codes turn into lust test cases |

Phase 0 and the L-packages can run in parallel with Phase 1; they touch different files.

The tank demo stays as it is (`crab-cage evolve`). New work is added beside it (`eval`, `run`). Nothing in Phases 1–2 edits `plant.rs`, `genome.rs`, `control.rs`, `fitness.rs`, `ga.rs`. Porting the tank to a nugget is the last, optional package (WP-19).

## 7. Not doing

- No AST parsing or `syn`. Markers and text.
- No UI, no CMA-ES, no CSV reader, no second simulator (all in the old roadmap; none serve the funnel).
- No change to the loopback-only rule.
- No rewrite of `supervisor.rs`. WP-01 is the one allowed edit.
- No lust-only syntax or attributes in nuggets. A nugget is plain Rust.

## 8. Risks

| Risk | Handling |
|---|---|
| The F1 cause is a hypothesis (pipe inheritance). | WP-01 fixes the Crab Cage side whatever the cause (never wait on a pipe after the deadline). L-01 confirms and fixes the lust side. |
| lust rejects much of what a small model writes. | Send `lust capabilities --summary` in the prompt; feed back `fix` text; count rejects per L-code in the journal (WP-13) so the cost is visible. |
| Byte-exact comparison flags harmless differences (float printing). | lust's rule zero says output is equal. A difference is a lust bug by definition; record it, do not loosen the comparison. |
| The local lust binary is 0.2.0, the source is 0.3.0. | First step of WP-04 is a rebuild. |
| A weak model edits outside its package. | Rules at the top of WORK-PACKAGES.md: listed files only, one package per pull request, the acceptance command must pass. |
