# Delta-kernel trajectory

Each candidate replaces the marked region of `template.rs`. `crab-cage delta eval` is the score of record: the harness score counts only when the reference decoder accepts the packed file. A rejected candidate has no byte count.

| # | Candidate | Result |
|---|---|---|
| 1 | `candidates/00-add-only.rs` | Proven. 1530 bytes. One ADD per new-real run, split to the width's `max_add`. |
| 2 | `candidates/01-gap-load.rs` | Rejected. The harness printed `error: no width covered the new bytes` and no `score:` line. The commands were a correct ADD cover plus a LOAD whose source is not old-real. No score. |
| 3 | `candidates/02-strategy-a.rs` | Proven. 731 bytes. One LOAD of each old real interval, then ADDs for the bytes that still differ, merged when the gap is smaller than the ADD header. |
| 4 | `candidates/03-strategy-b.rs`, first compile | Rejected by rustc (`E0282` / `E0616`). `Vec::new()` in the merge passes had no type, so a field access was read as a private field of `Vec`. |
| 5 | `candidates/03-strategy-b.rs`, second compile | Rejected by rustc (`E0308`). The hash lookup's vector was annotated `Vec<Cmd>` and had to be `Vec<u32>`. |
| 6 | `candidates/03-strategy-b.rs` | Proven. 860 bytes. Sector-local strategy B: window 8, stride 4, FNV-1a, 64 probes, `MIN_MATCH` 8, demote, touching-ADD merge, drop an all-0xFF ADD that no LOAD covers. |
| 7 | `candidates/04-best-of.rs` | Proven. 626 bytes. At each width, the cheaper legal plan of A and B. Matches the spec reference generator on every pair in this corpus. |
| 8 | `candidates/05-family.rs` | Proven. 860 bytes. The greedy grid's best point (window 8, stride 1, `MIN_MATCH` 3, one probe). It ties spec B and does not beat it. |
| 9 | `candidates/06-optimal.rs` | Proven. 625 bytes. Shortest path per sector: a LOAD may be patched by later ADDs, a 0xFF byte may be left to erased RAM, and each width is costed exactly. This is the v0.03 optimum for the corpus. |

Cage run time for every proven candidate was 86 or 87 ms for all eight pairs, inside the nugget's 20 s / 128 MB limit. Compile time is not part of that figure.

Re-run the folder with the cage's own proposers:

```sh
crab-cage run nuggets/delta-kernel --candidates nuggets/delta-kernel/candidates
crab-cage run nuggets/delta-kernel --backend local --model qwen2.5-coder:14b
```

`--backend local` posts to a loopback OpenAI-style server (default `http://127.0.0.1:11434/v1`). The prompt is the spliced template, so the comment above `// CRAB:BEGIN` is the contract the model sees. `delta eval` remains the decoder check; `run` records the harness score.
