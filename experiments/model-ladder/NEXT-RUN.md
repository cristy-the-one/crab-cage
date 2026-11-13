# The next ladder run: what to do on the RTX 5080 laptop

This file always describes the run that is due. It is rewritten after each run; the runs already made are in `results/` and in `docs/WORK-PACKAGES.md`.

**Due now: the sixth run, the first on the real problem.** Written 2026-11-13.

The five runs so far used three practice nuggets. This one uses `nuggets/delta-kernel`: a firmware delta encoder whose score is the number of bytes it produces for eight test cases, lower is better. The question is how far a capable local model gets, with the literal tweaker taking turns with it.

Known scores for this nugget:

| Score | What it is |
|---:|---|
| 1530 | the seed, where every run starts |
| 860 | one strategy written by hand |
| 731 | the other strategy written by hand |
| 626 | the better of the two, tuned |
| 625 | the optimum |

Any proven score under 1530 is an improvement the loop found by itself.

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

   One of the lines must read `Record the fifth ladder run, and set the next run to delta-kernel.` If none does, stop here.

2. **lust: do not update it.** `C:\path\to\lust.exe --version` must print `lust 0.3.1 (build 16d67467)`.

3. **Ollama's context must be 16384.** It was in every run so far, and this nugget needs it: its prompt is about 8,000 tokens before any feedback, and Ollama cuts a prompt that does not fit without saying so. Nothing to change if the fifth run worked. After the run, the column "Context" in `summary.md` must say 16384; if it says less, the run does not count.

## The run

One command, from the crab-cage folder. Put the real path of `lust.exe` in it:

```powershell
.\experiments\model-ladder\run.ps1 -Models gpt-oss:20b -Nuggets delta-kernel -Budget 40 -Reps 3 -Tweak -Seed 1 -LustBin C:\path\to\lust.exe
```

- `gpt-oss:20b` is the model that solved every practice nugget.
- `-Tweak` adds the literal tweaker: it changes one number of the best candidate at a time, with no model. The two take turns, so about half of the 40 candidates of a run come from each.
- `-Budget 40 -Reps 3`: three runs of 40 candidates.
- `-Seed 1`: the runs can be repeated.
- **It takes longer than the runs before: plan for 30 to 60 minutes.** A reply to this prompt is slower than the 17 seconds per candidate measured on the small nuggets, and a run on this nugget is expected to use its whole budget.
- If PowerShell refuses to run the script, start it as `powershell -ExecutionPolicy Bypass -File .\experiments\model-ladder\run.ps1` followed by the same parameters.

## If something looks wrong while it runs

- **Every candidate of the model is `rejected` at `check`, from the start:** let it finish. That is a result: it says which lust error codes stand between the model and this nugget.
- **The script stops with an error about the model or the endpoint:** check that Ollama is running (`ollama ps`) and start the command again. A run that was cut short starts over; that is fine.
- **A line says `diverged`:** let it finish and bring everything. A divergence is a case where lust and rustc disagree, and it is the most valuable thing a run can find.

## What to bring back

The **whole folder** of this run: `experiments\model-ladder\results\<date-time>\`, as before, with the `prompts\` folders. You may leave out the `.exe` and `.pdb` files under `work\` if the copy is too large; the `.rs` files there are needed, and so is any `divergences\` folder.

## What the result will be read for

| Question | Where the answer is |
|---|---|
| Does the loop get under 1530, and how far? | "Best proven" of each run in `summary.md`, against the table of known scores above. |
| Who found the improvements, the model or the tweaker? | The proposer's name on each `proven` line of `journal.jsonl`. |
| What does lust reject in code a model writes for a real problem? | The table of lust error codes. On the practice nuggets `gpt-oss:20b` was rejected 0 times in 10 candidates. |
| Is the prompt whole? | "Context" 16384, and the saved prompts end with the request line. |
| Do lust and rustc still agree? | `diverged` must be 0, as in all five runs (609 candidates). |

## Optional: the smaller model on the same problem

About as long again. It shows what the larger model buys on the real problem:

```powershell
.\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:14b -Nuggets delta-kernel -Budget 40 -Reps 3 -Tweak -Seed 1 -LustBin C:\path\to\lust.exe
```

Bring its results folder as well.
