# The coverage run: what to do on the RTX 5080 laptop

This is a separate run from the one in `NEXT-RUN.md`. It can be made before or after it.

Written 2026-11-13.

## What it is for

The runs so far ask "does the loop solve the nugget". This one asks "where does lust differ from rustc". Models write solutions to 26 small tasks of ordinary Rust (strings, maps and sets, iterators, traits, generics, enums, integer and float arithmetic, error handling), and every candidate is put to both compilers:

- **lust refuses it and rustc accepts it:** a gap in lust. The program is saved, and the summary ranks the gaps by how many candidates hit them. This is the work list for lust.
- **both accept it and the two builds print different output:** a divergence, a bug in lust. Saved under `divergences\`.
- **lust accepts and runs it and rustc refuses it:** lust is too lenient. Counted as "rejected by rustc only".

Whether a model solves a task matters little here. A weak model is as useful as a strong one: it writes more varied code.

## Before you start

1. **crab-cage: pull `main`.** In the crab-cage folder:

   ```powershell
   git checkout main
   ```

   ```powershell
   git pull
   ```

   ```powershell
   git log --oneline -8
   ```

   One of the lines must read `Add a coverage mode and 26 coverage nuggets.` If none does, stop here.

2. **lust: do not update it.** `C:\path\to\lust.exe --version` must print `lust 0.3.1 (build 16d67467)`. The gaps found are gaps of that build.

3. **Ollama is running.** These prompts are short (about 1,000 tokens), so the context length does not matter for this run.

## The run

One command, from the crab-cage folder. Put the real path of `lust.exe` in it:

```powershell
.\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:7b,qwen2.5-coder:14b,gpt-oss:20b -NuggetSet coverage -Coverage -Reps 2 -Budget 8 -Seed 1 -LustBin C:\path\to\lust.exe
```

- `-NuggetSet coverage` runs every nugget in `nuggets\coverage\` (26 of them).
- `-Coverage` turns on the comparison with rustc for every candidate.
- Three models, two runs each with different seeds, at most 8 candidates a run: 156 runs, most of which end after one or two candidates because the task is solved.
- **Plan for one to two hours.** This is an estimate from the times per candidate of the earlier runs (about 2.5, 6.5 and 17 seconds for the three models), plus one rustc run per candidate. If it is too long, leave out `gpt-oss:20b`: it writes the least varied code.
- If PowerShell refuses to run the script, start it as `powershell -ExecutionPolicy Bypass -File .\experiments\model-ladder\run.ps1` followed by the same parameters.

## If something looks wrong while it runs

- **A line says `diverged`:** let it finish. That is the most valuable thing this run can find.
- **Many candidates are `rejected` at `check`:** expected. That is what is being counted.
- **The script stops with an error about the model or the endpoint:** check that Ollama is running (`ollama ps`) and start the command again. Runs already finished are not reused; the run starts over in a new results folder.

## What to bring back

The **whole folder** of this run: `experiments\model-ladder\results\<date-time>\`. It is small: the tasks are small programs. Do not leave anything out. The folders that matter most are the `lust-only\` and `divergences\` folders inside the runs.

## What the result will be read for

| Question | Where the answer is |
|---|---|
| Which valid Rust does lust refuse, and how often does a model write it? | "Rejected by lust only" in `summary.md`: one row per lust message, with a count, the models, and an example file. |
| Do lust and rustc ever print different output? | "Diverged" in "Coverage totals", and the `divergences\` folders. In 609 candidates so far there was none, but those were five programs. |
| Does lust accept anything rustc refuses? | "Rejected by rustc only" in "Coverage totals". |
| How wide was the sample? | "Candidates" and "Ran and matched rustc" in "Coverage totals". |

## What is already known

While the tasks were written, their solutions and a few probes were put to lust 0.3.1. Five gaps turned up before any model ran. The run should find these again, and they show what a row of the table will look like:

| lust says | For |
|---|---|
| `L0505` the module path `std::char::from_digit` is not supported yet | `std::char::from_digit(d, 16)`; `char::from_digit(d, 16)` is accepted |
| `L0104` no method `as_str` on type `std::str::Chars` | `chars.as_str()` |
| `L0101` cannot find type `LinkedList` | `std::collections::LinkedList` |
| `L0104` no method `checked_mul` on type `i128` | `n.checked_mul(2)` on an `i128` |
| `L0111` cannot call method `wrapping_add` on an integer whose type is not known yet | `v.iter().copied().fold(0, \|a, b\| a.wrapping_add(b))` over `u32`s |
