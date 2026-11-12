# Model ladder

- Date: 2026-11-12 09:09
- CPU: Intel(R) Core(TM) Ultra 9 275HX
- GPU: Intel(R) Graphics; NVIDIA GeForce RTX 5080 Laptop GPU (nvidia-smi: NVIDIA GeForce RTX 5080 Laptop GPU, 16303 MiB)
- RAM: 63 GB
- Ollama: 0.40.16
- crab-cage: c28e6d5; rustc 1.96.0 (ac68faa20 2026-05-25)
- lust: lust 0.3.0 (build 8eb1e88c)
- Runs per model and nugget: 3; budget: 20 candidates; one job

## Models as loaded

| Model | Size | On the GPU | Context | First load |
|---|---:|---:|---:|---:|
| qwen2.5-coder:14b | 11.9 GB | 11.9 GB | 16384 | 5.1 s |

## Solved

Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.

| Model | sum-squares | run-lengths | merge-ranges | normalize-path |
|---|---|---|---|---|
| qwen2.5-coder:14b | 3/3; 1 cand.; 1.8 s | 3/3; 1 cand.; 3.5 s | 0/3 | 1/3; 5 cand.; 38.9 s |

## Unsolved runs

| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |
|---|---|---:|---:|---:|---:|---:|---|
| qwen2.5-coder:14b | merge-ranges | 1 | 7 | 7 | 9 | 86.7 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:14b | merge-ranges | 2 | 7 | 7 | 7 | 64 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:14b | merge-ranges | 3 | 7 | 7 | 16 | 226.2 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:14b | normalize-path | 1 | 8 | 4 | 4 | 32.8 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:14b | normalize-path | 2 | 8 | 2 | 3 | 33.7 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |

## Where candidates were rejected

| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| qwen2.5-coder:14b | 50 | 0 | 34 | 0 | 0 | 0 | 0 | 10 |

## lust error codes, all models

| Code | Candidates |
|---|---:|
| L0103 | 96 |
| L0110 | 2 |

Every run keeps its journal, its candidates and any `divergences/` under this directory.
