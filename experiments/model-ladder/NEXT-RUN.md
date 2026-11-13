# The next ladder run: what to do on the RTX 5080 laptop

This file always describes the run that is due. It is rewritten after each run; the runs already made are in `results/` and in `docs/WORK-PACKAGES.md`.

**Due now: the fifth run.** Written 2026-11-13.

It measures one thing: whether a model does better once it is told what became of a region that only compiled after rustc's fix. Nothing else may change against the fourth run, so lust stays at 0.3.1 and the command is the same.

## Before you start

1. **crab-cage must contain pull request #34.** In the crab-cage folder:

   ```powershell
   git checkout main
   ```

   ```powershell
   git pull
   ```

   ```powershell
   git log --oneline -8
   ```

   One of the lines must read `Tell the model what became of a region that only compiled after repair.` If none does, stop here: the pull did not bring #34.

2. **lust: do not update it.** Use the same `lust.exe` as in the runs before. Check it:

   ```powershell
   C:\path\to\lust.exe --version
   ```

   It must print `lust 0.3.1 (build 16d67467)`. If it prints something else, the run is still valid, but say so when you bring the results.

3. **Ollama is running, with the long context.** Nothing to change if the fourth run worked. The summary of this run shows it again in the column "Context": it must say 16384.

## The run

One command, from the crab-cage folder. Put the real path of `lust.exe` in it:

```powershell
.\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:7b,qwen2.5-coder:14b -Nuggets run-lengths,merge-ranges,normalize-path -Seed 1 -LustBin C:\path\to\lust.exe
```

- This is the command of the third and fourth run, unchanged. Do not add `-Reps`, `-Budget` or `-Tweak`, and keep `-Seed 1`.
- The script builds crab-cage itself before the first run.
- If PowerShell refuses to run the script, start it as `powershell -ExecutionPolicy Bypass -File .\experiments\model-ladder\run.ps1` followed by the same parameters.

## What to bring back

The **whole folder** of this run: `experiments\model-ladder\results\<date-time>\`, as before.

The `prompts\` folders inside it are needed: they show what the model was told and what it answered. You may leave out the `.exe` and `.pdb` files under `work\` if the copy is too large; the `.rs` files there are needed.

## What the result will be read for

| Question | Where the answer is |
|---|---|
| Does the 7b model move off 10 failing cases on `normalize-path`, now that it sees the case its own lines fail? | In the fourth run every candidate that ran failed 10 of 10, and the best stayed the seed's 8. A `local` or `repair` candidate with a score below 8 is the sign. |
| Does it stop writing the same region again? | The number of requests per run in `prompts\`, and the number of different candidates. In the fourth run it alternated between one rejection and one repair. |
| Are the runs the repair proposer does not touch still the same? | With the same seed they must be the fourth run again, candidate for candidate: all of the 14b model's, and the 7b model's `run-lengths` and `merge-ranges`. |
| Do lust and rustc still agree? | `diverged` must be 0, as in all four runs. |

If the 7b model still solves 0 of 3 on `normalize-path`, the conclusion is that this nugget is beyond a 7B model, and that the loop is no longer what holds it back. `gpt-oss:20b` solved it 3 of 3 in the first run.

## Optional: the larger model, once

This is a separate command and takes a few minutes. It shows whether the changes since the first run cost the strongest model anything:

```powershell
.\experiments\model-ladder\run.ps1 -Models gpt-oss:20b -Nuggets run-lengths,merge-ranges,normalize-path -Seed 1 -LustBin C:\path\to\lust.exe
```

In the first run it solved all three nuggets in every run. Bring its results folder as well.
