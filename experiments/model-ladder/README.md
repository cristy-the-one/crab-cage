# The model ladder

One script that runs the search loop with local models on nuggets of rising difficulty and writes one table. It is meant to be run on a machine with a real GPU and sent back.

## What it answers

1. **Which model size solves what.** On a laptop processor a 7.6B model solved the easy nugget in one request and stalled on the hard one. Does a 14B or 20B model on a GPU get further, and how many candidates does it take?
2. **What a candidate costs.** Seconds per candidate, per model, with the model on the GPU.
3. **What lust rejects.** Which lust error codes model-written code runs into, and how often. That is the measured input for what lust should support next.
4. **The real problem.** `nuggets/delta-kernel` is a `minimize` nugget with known reference points: 1530 bytes for the seed, 731 and 860 for the two hand strategies, 626 for the better of them, 625 for the optimum. How far does a local model get from 1530?

## The rungs

| Nugget | Objective | Size of the task |
|---|---|---|
| `sum-squares` | pass all 3 cases | one line; the function name gives it away |
| `run-lengths` | pass all 8 cases | a loop with one special rule (digits) and Unicode |
| `merge-ranges` | pass all 9 cases | parse, sort, merge; touching ranges count |
| `normalize-path` | pass all 10 cases | several rules that interact; the 7.6B model did not solve it |
| `delta-kernel` | minimize bytes | a firmware delta strategy; a 14 KB template |

Each of the first four has a `reference.rs` beside it that passes every case, so a failure is the model's and not the nugget's.

## Before the first run

On the machine with the GPU, once:

1. **Rust.** `rustup` with a stable toolchain; `rustc` on `PATH`.
2. **This repository.** `git clone https://github.com/cristy-the-one/crab-cage` and `cd crab-cage`.
3. **Ollama.** Installed and running.
4. **Context length.** The `delta-kernel` prompt is about 8,000 tokens, and Ollama cuts a prompt that does not fit without saying so. Stop Ollama, set `OLLAMA_CONTEXT_LENGTH` to `16384`, and start it again. In PowerShell, for the current user:

   ```powershell
   [Environment]::SetEnvironmentVariable('OLLAMA_CONTEXT_LENGTH', '16384', 'User')
   ```

   then quit Ollama from the tray and start it. The script prints each model's context and warns when it is too small.
5. **Optional: lust.** Without it the script still runs, with `rustc` doing the fast gates; question 3 then has no answer. To build lust on Windows: Visual Studio Build Tools with "Desktop development with C++", then in a lust-lang checkout `pwsh -File scripts/build-tcc.ps1`, set `LUST_TCC_HOME` to `third_party\tcc`, and `cargo build --release` (about ten minutes). Use lust-lang `next` at `10148c4d` or later.

## Run it

A two-minute check that everything is wired up (first model, first nugget, one run):

```powershell
.\experiments\model-ladder\run.ps1 -Quick -Pull
```

The ladder: three models, four nuggets, three runs each. `-Pull` downloads the models in `models.txt` that are not installed yet (about 4.7, 9 and 13 GB).

```powershell
.\experiments\model-ladder\run.ps1 -Pull -LustBin C:\path\to\lust.exe
```

The real problem, with a larger budget, and the literal-tweak proposer taking turns with the model:

```powershell
.\experiments\model-ladder\run.ps1 -Nuggets delta-kernel -Budget 40 -Reps 3 -Tweak -LustBin C:\path\to\lust.exe
```

If PowerShell refuses to run the script, start it as `powershell -ExecutionPolicy Bypass -File .\experiments\model-ladder\run.ps1 …`. On Linux or macOS use `pwsh`.

| Parameter | Default | Meaning |
|---|---|---|
| `-Models` | the lines of `models.txt` | model names as Ollama knows them |
| `-Nuggets` | the four pass-all rungs | directory names under `nuggets/` |
| `-Reps` | 3 | runs per model and nugget; a model's replies vary |
| `-Budget` | 20 | candidates to journal per run, the seed included |
| `-LustBin` | none | a lust binary |
| `-Pull` | off | `ollama pull` a model that is not installed |
| `-Tweak` | off | also use the literal-tweak proposer |
| `-OutDir` | `results/<date-time>` here | where everything is written |
| `-Quick` | off | one model, one nugget, one run, budget 6 |

## How long

On the laptop processor one candidate took 9 to 60 seconds, depending on the size of the prompt. A GPU should be several times faster. The ladder is at most 3 × 4 × 3 = 36 runs of at most 20 candidates, and most runs stop early, either solved or with the proposer finished. Expect well under an hour on a 16 GB GPU; the `delta-kernel` run is the long one.

## What comes back

Everything is under the output directory:

```text
summary.md          the tables; this is the file to read
results.jsonl       one line per run
environment.json    CPU, GPU, RAM, versions, the parameters; no user or machine name
<model>/<nugget>/rep-N/
    run.log         what crab-cage printed
    journal.jsonl   one verdict per candidate
    best.region.rs  the best proven candidate
    work/           every candidate as <hash>.rs, and divergences/ if lust and rustc disagreed
```

`summary.md` has: the machine and versions; each model's size, how much of it sat on the GPU, and its context; a solved table (runs solved of runs made, then the median candidates and seconds over the solved runs); the unsolved runs with how each ended; the `minimize` runs with seed and best score; where candidates were rejected; and the lust error codes.

To send it back, zip the output directory, or commit it on a branch:

```powershell
git checkout -b ladder-results
git add experiments/model-ladder/results
git commit -m "Model ladder results"
git push -u origin ladder-results
```

The `work/` directories hold built programs and can be large; `summary.md`, `results.jsonl`, `environment.json` and the `journal.jsonl` files are what matter. A `divergences/` directory, if there is one, matters most: it is a case where lust and `rustc` disagreed.

## Reading the result

- A model that solves `sum-squares` and nothing else is at the level already seen on the laptop.
- `normalize-path` solved in any run is new: no local model has done it yet.
- On `delta-kernel`, any proven score under 1530 is a real improvement found by the loop; under 860 beats one hand strategy; 626 or less matches the best known.
- `0 GB` in "On the GPU" means the model ran on the processor, and the times say nothing about the GPU.
- A context marked too small makes that model's rows worthless: fix the context and run again.

## What it does not do

It runs one job at a time (`--jobs 1`), because a model proposer with several jobs gives up early. It does not change the prompt, the temperature (0.2, and 0.8 for the retry after a repeat), or anything in the loop. It does not test a model outside Ollama; any server with an OpenAI-style chat endpoint on loopback works with `-Endpoint`, but the size and context columns come from Ollama's own API.
