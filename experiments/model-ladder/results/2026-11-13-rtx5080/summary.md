# Model ladder

- Date: 2026-11-13 10:11
- CPU: Intel(R) Core(TM) Ultra 9 275HX
- GPU: Intel(R) Graphics; NVIDIA GeForce RTX 5080 Laptop GPU (nvidia-smi: NVIDIA GeForce RTX 5080 Laptop GPU, 16303 MiB)
- RAM: 63 GB
- Ollama: 0.40.16
- crab-cage: 106eddc; rustc 1.96.0 (ac68faa20 2026-05-25)
- lust: lust 0.3.1 (build 16d67467)
- Runs per model and nugget: 3; budget: 20 candidates; one job

## Models as loaded

| Model | Size | On the GPU | Context | First load |
|---|---:|---:|---:|---:|
| qwen2.5-coder:7b | 5.6 GB | 5.6 GB | 16384 | 5.1 s |
| qwen2.5-coder:14b | 11.9 GB | 11.9 GB | 16384 | 7.3 s |

## Solved

Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.

| Model | run-lengths | merge-ranges | normalize-path |
|---|---|---|---|
| qwen2.5-coder:7b | 1/3; 1 cand.; 2.9 s | 3/3; 1 cand.; 3.9 s | 0/3 |
| qwen2.5-coder:14b | 3/3; 1 cand.; 4 s | 3/3; 9 cand.; 37.4 s | 1/3; 10 cand.; 63.7 s |

## Unsolved runs

| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |
|---|---|---:|---:|---:|---:|---:|---|
| qwen2.5-coder:7b | run-lengths | 1 | 7 | 2 | 5 | 27.3 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | run-lengths | 2 | 7 | 2 | 5 | 35.6 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 1 | 8 | 8 | 4 | 17.7 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 2 | 8 | 8 | 19 | 38.8 s |  |
| qwen2.5-coder:7b | normalize-path | 3 | 8 | 8 | 18 | 59.2 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:14b | normalize-path | 2 | 8 | 5 | 19 | 93.1 s |  |
| qwen2.5-coder:14b | normalize-path | 3 | 8 | 1 | 19 | 204.1 s |  |

## Where candidates were rejected

| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| qwen2.5-coder:7b | 56 | 0 | 44 | 0 | 1 | 0 | 0 | 3.5 |
| qwen2.5-coder:14b | 72 | 0 | 23 | 0 | 0 | 0 | 0 | 6.5 |

## lust error codes, all models

| Code | Candidates |
|---|---:|
| L0113 | 68 |
| L0610 | 54 |
| L0110 | 7 |
| L0640 | 1 |

Every run keeps its journal, its candidates and any `divergences/` under this directory.
