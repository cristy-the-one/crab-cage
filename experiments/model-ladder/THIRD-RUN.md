# The third ladder run: what to do on the RTX 5080 laptop

Written 2026-11-13. This run measures one thing: whether a model does better when the prompt carries rustc's whole message with its `help:`. Nothing else may change against the second run, so lust stays at 0.3.1.

## Before you start

1. **crab-cage must contain pull request #30.** In the crab-cage folder:

   ```powershell
   git checkout main
   ```

   ```powershell
   git pull
   ```

   ```powershell
   git log --oneline -8
   ```

   One of the lines must read `Give the model rustc's whole first error, and save every prompt.` If none does, #30 is not merged yet: stop here.

2. **lust: do not update it.** Use the same `lust.exe` as in the second run. Check it:

   ```powershell
   C:\path\to\lust.exe --version
   ```

   It must print `lust 0.3.1 (build 16d67467)`. If lust was rebuilt since, the run is still valid, but say so when you bring the results.

3. **Ollama is running, with the long context.** Nothing to change if the second run worked. The summary of this run will show it again in the column "Context": it must say 16384.

## The run

One command, from the crab-cage folder. Put the real path of `lust.exe` in it:

```powershell
.\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:7b,qwen2.5-coder:14b -Nuggets run-lengths,merge-ranges,normalize-path -Seed 1 -LustBin C:\path\to\lust.exe
```

- The same two models, three nuggets, three runs each and budget 20 as the second run. Do not add `-Reps`, `-Budget` or `-Tweak`.
- `-Seed 1` is new. Run 1, 2 and 3 of each model and nugget get the seeds 1, 2 and 3, so the same command gives the same run again later.
- It took about 15 minutes last time. It may take longer now: runs that used to stop early now use the whole budget.
- If PowerShell refuses to run the script, start it as `powershell -ExecutionPolicy Bypass -File .\experiments\model-ladder\run.ps1` followed by the same parameters.

## What to bring back

The **whole folder** of this run: `experiments\model-ladder\results\<date-time>\`, as last time.

New in it: every run folder now has `prompts\`, with one text file for each request to the model and its reply. These are the files that show whether the model was given rustc's help and what it wrote back. Do not leave them out.

You may leave out the `work\` folders if the copy is too large; the `.exe` and `.pdb` files in them are not needed. The `.rs` files in `work\` are needed.

## Optional, five minutes: does a seed repeat on the GPU?

After the run, start this twice, one after the other:

```powershell
.\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:7b -Nuggets normalize-path -Reps 1 -Seed 1 -LustBin C:\path\to\lust.exe
```

Bring both result folders. If the two `journal.jsonl` files list the same hashes in the same order, a seed repeats a run on this machine. On the work laptop (CPU) it does.

## What the result will be read for

| Question | Where the answer is |
|---|---|
| Does the 7b model get past `c != "."` on `normalize-path`? | It solved 0 of 3 twice. The `prompts\` files show how many replies it needed after rustc's help appeared. |
| Does the 14b model annotate the `Vec` on `merge-ranges`, where rustc's help now names the binding? | It took 8 and 10 rejections last time. Fewer `L0610` rejections before the solving candidate is the sign. |
| Is anything worse? | The solved table against the second run, and the "Unsolved runs" table. |
| Do lust and rustc still agree? | `diverged` must be 0, as in both runs before. |

The lust change that belongs to this (L-13, a better fix text for the same comparison) is not part of this run. It gets its own run after it is merged and released, so that the two effects can be told apart.
